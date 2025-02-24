mod db;
mod config;

use anyhow::{anyhow, Context, Result};
use config::Settings;
use db::Database;
use dotenvy::dotenv;
use env_logger;
use futures::{sink::SinkExt, stream::StreamExt};
use log::{error, info};
use serde_json::{json, Value};
use solana_sdk::signature::Signature as SolanaSignature;
use solana_transaction_status::UiTransactionEncoding;
use std::{collections::HashMap, time::Duration};
use tokio::main;
use tonic::transport::ClientTlsConfig;
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::convert_from;
use yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions,
    SubscribeRequestPing, SubscribeUpdateTransactionInfo,
};

#[main]
async fn main() -> Result<()> {
    // Load env vars (if you have a .env file) and init logger
    dotenv().ok();
    env_logger::init();

    // Load your application settings
    let settings = Settings::new()?;

    // Connect to PostgreSQL using your existing db code
    let database = Database::connect(
        &settings.database.url,
        settings.database.max_connections
    )
    .await?;

    // Get the endpoint, token, and program ID from your config
    let endpoint = settings.shyft.endpoint;
    let x_token = settings.shyft.x_token;
    let program_id = settings.app.raydium_liquidity_pool_v4;

    // Connect to the gRPC endpoint
    let mut client = GeyserGrpcClient::build_from_shared(endpoint.clone())?
        .x_token(Some(x_token.clone()))?
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(10))
        .tls_config(ClientTlsConfig::new().with_native_roots())?
        .max_decoding_message_size(1024 * 1024 * 1024)
        .connect()
        .await?;

    // Build our subscription request
    let request = build_subscribe_request(&program_id);

    // Initiate streaming
    let (mut subscribe_tx, mut stream) = client.subscribe_with_request(Some(request)).await?;

    info!("Connected to gRPC at: {endpoint}");

    // Listen to incoming subscription messages
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(update) => match update.update_oneof {
                Some(UpdateOneof::Transaction(tx_info)) => {
                    // We have a new transaction to parse
                    let tx_data = tx_info
                        .transaction
                        .ok_or(anyhow!("No transaction in the message"))?;

                    let value = create_pretty_transaction(tx_data)?;
                    let meta = &value["tx"]["meta"];

                    // Extract logs
                    let logs: Vec<String> = meta["logMessages"]
                        .as_array()
                        .unwrap_or(&vec![])
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();

                    // Check for "initialize2" logs
                    if search_initialize2(&logs) {
                        let signature = value["signature"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let is_vote = value["isVote"].as_bool().unwrap_or(false);

                        info!("Newly migrated pool: signature={signature}");

                        // Write to DB
                        sqlx::query!(
                            r#"
                            INSERT INTO RaydiumLPV4 (signature, pool_address, is_vote)
                            VALUES ($1, $2, $3)
                            "#,
                            signature,
                            program_id,
                            is_vote
                        )
                        .execute(&database.pool)
                        .await?;
                    }

                    // Otherwise, do nothing/no logs for non-"initialize2" tx
                }
                Some(UpdateOneof::Ping(_)) => {
                    // This keeps certain load balancers happy with a Pong
                    subscribe_tx
                        .send(SubscribeRequest {
                            ping: Some(SubscribeRequestPing { id: 1 }),
                            ..Default::default()
                        })
                        .await?;
                }
                Some(UpdateOneof::Pong(_)) => {
                    // Pong response received
                }
                // Unhandled updates
                _ => {}
            },
            Err(err) => {
                error!("Error in stream: {err:?}");
                break;
            }
        }
    }

    info!("gRPC stream closed.");

    Ok(())
}

/// Construct a gRPC subscription request for a specific program ID
fn build_subscribe_request(program_id: &str) -> SubscribeRequest {
    let mut transactions = HashMap::new();
    transactions.insert(
        "client".to_string(),
        SubscribeRequestFilterTransactions {
            vote: None,
            failed: Some(false),
            signature: None,
            account_include: vec![program_id.to_string()],
            account_exclude: vec![],
            account_required: vec![],
        },
    );

    SubscribeRequest {
        slots: HashMap::default(),
        accounts: HashMap::default(),
        transactions,
        transactions_status: HashMap::default(),
        entry: HashMap::default(),
        blocks: HashMap::default(),
        blocks_meta: HashMap::default(),
        commitment: Some(CommitmentLevel::Processed as i32),
        accounts_data_slice: Vec::default(),
        ping: None,
        from_slot: None,
    }
}

/// Turn raw gRPC transaction data into a JSON `Value` with signature + logs
fn create_pretty_transaction(tx: SubscribeUpdateTransactionInfo) -> Result<Value> {
    Ok(json!({
        "signature": SolanaSignature::try_from(tx.signature.as_slice())
            .context("invalid signature")?
            .to_string(),
        "isVote": tx.is_vote,
        "tx": convert_from::create_tx_with_meta(tx)
            .map_err(|e| anyhow!("Error building TxWithMeta: {e}"))?
            .encode(UiTransactionEncoding::Base64, Some(u8::MAX), true)
            .context("Failed to encode transaction")?,
    }))
}

/// Check if any log line contains "initialize2"
fn search_initialize2(logs: &[String]) -> bool {
    logs.iter().any(|log| log.contains("initialize2"))
}
