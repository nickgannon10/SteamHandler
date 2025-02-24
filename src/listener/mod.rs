// src/listener/mod.rs

use anyhow::Result;
use futures::{StreamExt, SinkExt};
use log::{error, info};
use std::{collections::HashMap, time::Duration};
use tonic::transport::ClientTlsConfig;
use yellowstone_grpc_client::GeyserGrpcClient;
use yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestFilterTransactions,
    SubscribeRequestPing, SubscribeUpdateTransactionInfo
};
use tokio::sync::mpsc;

pub struct RaydiumListener {
    endpoint: String,
    token: String,
    program_id: String,
    // If we want to pass events outward, we might have something like a sender:
    sender: mpsc::UnboundedSender<SubscribeUpdateTransactionInfo>,
}

impl RaydiumListener {
    pub fn new(
        endpoint: String,
        token: String,
        program_id: String,
        sender: mpsc::UnboundedSender<SubscribeUpdateTransactionInfo>,
    ) -> Self {
        Self {
            endpoint,
            token,
            program_id,
            sender,
        }
    }

    /// The main routine: connect, subscribe, and stream
    pub async fn run(&mut self) -> Result<()> {
        // 1) Build gRPC client
        let mut client = GeyserGrpcClient::build_from_shared(self.endpoint.clone())?
            .x_token(Some(self.token.clone()))?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(10))
            .tls_config(ClientTlsConfig::new().with_native_roots())?
            .max_decoding_message_size(1024 * 1024 * 1024)
            .connect()
            .await?;

        // 2) Build our subscription request
        let subscribe_request = self.build_subscribe_request();

        // 3) Initiate streaming
        let (mut tx, mut rx) = client.subscribe_with_request(Some(subscribe_request)).await?;

        info!("RaydiumListener connected. Beginning stream...");

        // 4) Process messages in a loop
        while let Some(msg) = rx.next().await {
            match msg {
                Ok(update) => {
                    // Check which update type
                    match update.update_oneof {
                        Some(UpdateOneof::Transaction(tx_info)) => {
                            // We could forward the raw SubscribeUpdateTransactionInfo
                            // to the orchestrator for further logic.
                            if let Err(e) = self.sender.send(tx_info) {
                                error!("Failed to send transaction info via channel: {:?}", e);
                            }
                        },
                        Some(UpdateOneof::Ping(_)) => {
                            // Respond with a Pong to keep the subscription alive
                            let _ = tx.send(tonic::IntoRequest::into_request(
                                SubscribeRequest {
                                    ping: Some(SubscribeRequestPing { id: 1 }),
                                    ..Default::default()
                                }
                            ))
                            .await;
                        },
                        Some(UpdateOneof::Pong(_)) => {
                            // Acknowledge Pong
                            info!("Received Pong from server");
                        },
                        _ => {}, // For blocks, accounts, etc. Not used in this example
                    }
                },
                Err(e) => {
                    error!("Stream error: {e:?}");
                    break;
                }
            }
        }

        info!("RaydiumListener gRPC stream ended.");
        Ok(())
    }

    fn build_subscribe_request(&self) -> SubscribeRequest {
        let mut transactions = HashMap::new();
        transactions.insert(
            "client".to_string(),
            SubscribeRequestFilterTransactions {
                vote: None,
                failed: Some(false),
                signature: None,
                account_include: vec![self.program_id.clone()],
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
            accounts_data_slice: vec![],
            ping: None,
            from_slot: None,
        }
    }
}
