// Improved raydium_v4.rs implementation
use {
    anyhow::{Context, Result},
    backoff::{future::retry, ExponentialBackoff},
    chrono,
    futures::{future::TryFutureExt, sink::SinkExt, stream::StreamExt},
    log::{error, info},
    solana_sdk::signature::Signature,
    std::{collections::HashMap, sync::Arc, time::Duration},
    tokio::sync::{mpsc, Mutex},
    tonic::transport::channel::ClientTlsConfig,
    yellowstone_grpc_client::{GeyserGrpcClient, Interceptor},
    yellowstone_grpc_proto::prelude::{
        subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestPing,
        SubscribeRequestFilterTransactions, SubscribeUpdateTransactionInfo,
    },
};

use crate::db::Database;

type TransactionsFilterMap = HashMap<String, SubscribeRequestFilterTransactions>;

// Define the event structure that will be sent to callbacks
#[derive(Debug, Clone)]
pub struct RaydiumV4PoolEvent {
    pub signature: String,
    pub pool_address: Option<String>,
    pub mint_address: Option<String>,
    pub signers: Vec<String>,
    pub timestamp: u64,
    pub raw_logs: Vec<String>,
}

// Explicitly derive Clone to be clear about the intent
#[derive(Clone)]
pub struct RaydiumV4Monitor {
    endpoint: String,
    x_token: String,
    program_id: String,
    migration_pubkey: String,
    database: Arc<Database>,
    event_tx: mpsc::Sender<RaydiumV4PoolEvent>,
}

impl RaydiumV4Monitor {
    pub fn new(
        endpoint: String,
        x_token: String,
        program_id: String,
        migration_pubkey: String,
        database: Arc<Database>,
        event_tx: mpsc::Sender<RaydiumV4PoolEvent>,
    ) -> Self {
        Self {
            endpoint,
            x_token,
            program_id,
            migration_pubkey,
            database,
            event_tx,
        }
    }

    async fn connect(&self) -> Result<GeyserGrpcClient<impl Interceptor>> {
        GeyserGrpcClient::build_from_shared(self.endpoint.clone())?
            .x_token(Some(self.x_token.clone()))?
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(10))
            .tls_config(ClientTlsConfig::new().with_native_roots())?
            .max_decoding_message_size(1024 * 1024 * 1024)
            .connect()
            .await
            .map_err(Into::into)
    }

    fn get_subscribe_request(&self) -> Result<SubscribeRequest> {
        let mut transactions: TransactionsFilterMap = HashMap::new();
        transactions.insert(
            "client".to_owned(),
            SubscribeRequestFilterTransactions {
                vote: None,
                failed: Some(false),
                signature: None,
                account_include: vec![self.program_id.clone()],
                account_exclude: vec![],
                account_required: vec![],
            },
        );

        Ok(SubscribeRequest {
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
        })
    }

    async fn process_transaction(&self, tx_info: &SubscribeUpdateTransactionInfo) -> Result<()> {
        // Parse out the "pool address", "mint token", and signers:
        match extract_pool_address_mint_and_signers(tx_info, &self.program_id) {
            Ok((maybe_pool, maybe_mint, signers)) => {
                let signature_b58 = match Signature::try_from(tx_info.signature.as_slice()) {
                    Ok(sig) => sig.to_string(),
                    Err(_) => String::from("invalid-signature"),
                };

                // 1) Check if "initialize2" is in the logs:
                if let Some(meta) = &tx_info.meta {
                    if !meta.log_messages_none {
                        let has_initialize2 = meta
                            .log_messages
                            .iter()
                            .any(|log_line| log_line.contains("initialize2"));

                        // 2) Check if our target pubkey is a signer
                        let has_target_signer = signers.contains(&self.migration_pubkey);

                        // 3) Only process if both conditions are met
                        if has_initialize2 && has_target_signer {
                            // Create event object to send to the orchestrator
                            let event = RaydiumV4PoolEvent {
                                signature: signature_b58.clone(),
                                pool_address: maybe_pool.clone(),
                                mint_address: maybe_mint.clone(),
                                signers: signers.clone(),
                                timestamp: chrono::Utc::now().timestamp() as u64,
                                raw_logs: meta.log_messages.clone(),
                            };
                            
                            // Log locally in the monitor for debugging
                            info!(
                                "Detected initialize2 event, sending to orchestrator. signature={}", 
                                signature_b58
                            );
                            
                            // Send the event to the orchestrator for processing
                            if let Err(e) = self.event_tx.send(event).await {
                                error!("Failed to send event to orchestrator: {}", e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("Failed to parse transaction: {:?}", e);
            }
        }
        
        Ok(())
    }

    async fn subscribe_to_geyser(
        &self,
        mut client: GeyserGrpcClient<impl Interceptor>,
        request: SubscribeRequest,
        mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<()> {
        let (mut subscribe_tx, mut stream) = client.subscribe_with_request(Some(request)).await?;
        info!("Stream opened for Raydium V4 monitor");

        loop {
            tokio::select! {
                // Check for shutdown signal
                Ok(_) = shutdown_rx.recv() => {
                    info!("Shutdown signal received for Raydium V4 monitor");
                    break;
                }
                
                // Process stream messages
                message = stream.next() => {
                    match message {
                        Some(Ok(msg)) => match msg.update_oneof {
                            Some(UpdateOneof::Transaction(tx_msg)) => {
                                // Extract the actual transaction struct
                                if let Some(tx_info) = tx_msg.transaction {
                                    if let Err(e) = self.process_transaction(&tx_info).await {
                                        error!("Error processing transaction: {:?}", e);
                                    }
                                } else {
                                    error!("No transaction in the message");
                                }
                            }
                            Some(UpdateOneof::Ping(_)) => {
                                // Keep-alive ping
                                if let Err(e) = subscribe_tx
                                    .send(SubscribeRequest {
                                        ping: Some(SubscribeRequestPing { id: 1 }),
                                        ..Default::default()
                                    })
                                    .await
                                {
                                    error!("Failed to send ping: {:?}", e);
                                    break;
                                }
                            }
                            Some(UpdateOneof::Pong(_)) => {
                                // Pong response, ignore
                            }
                            None => {
                                error!("Update not found in the message");
                                break;
                            }
                            _ => {}
                        },
                        Some(Err(error)) => {
                            error!("Stream error: {:?}", error);
                            break;
                        }
                        None => {
                            info!("Stream closed");
                            break;
                        }
                    }
                }
            }
        }

        info!("Stream closed for Raydium V4 monitor");
        Ok(())
    }

    pub async fn run(&self, mut shutdown_rx: tokio::sync::broadcast::Receiver<()>) -> Result<()> {
        info!("Starting Raydium V4 monitor");
        let zero_attempts = Arc::new(Mutex::new(true));

        // Copy the data needed for the closures
        let endpoint = self.endpoint.clone();
        let x_token = self.x_token.clone();
        let program_id = self.program_id.clone();
        let migration_pubkey = self.migration_pubkey.clone();
        let database = Arc::clone(&self.database);
        let event_tx = self.event_tx.clone();

        // The default exponential backoff strategy for reconnections
        retry(ExponentialBackoff::default(), move || {
            // Clone only what's needed without cloning the entire monitor
            let endpoint = endpoint.clone();
            let x_token = x_token.clone();
            let program_id = program_id.clone();
            let migration_pubkey = migration_pubkey.clone();
            let database = Arc::clone(&database);
            let event_tx = event_tx.clone();
            let mut shutdown_rx_clone = shutdown_rx.resubscribe();
            let zero_attempts = Arc::clone(&zero_attempts);

            async move {
                let mut zero_attempts = zero_attempts.lock().await;
                if *zero_attempts {
                    *zero_attempts = false;
                } else {
                    info!("Retry to connect to the server for Raydium V4 monitor");
                }
                drop(zero_attempts);

                // Create a temporary monitor with cloned values for this retry attempt
                let temp_monitor = RaydiumV4Monitor::new(
                    endpoint.clone(),
                    x_token.clone(),
                    program_id.clone(),
                    migration_pubkey.clone(),
                    Arc::clone(&database),
                    event_tx.clone(),
                );

                // Use the temporary monitor
                let client = temp_monitor.connect().await.map_err(backoff::Error::transient)?;
                info!("Connected to gRPC server for Raydium V4 monitor");

                let request = temp_monitor
                    .get_subscribe_request()
                    .map_err(backoff::Error::Permanent)?;

                temp_monitor
                    .subscribe_to_geyser(client, request, shutdown_rx_clone)
                    .await
                    .map_err(backoff::Error::transient)?;

                Ok::<(), backoff::Error<anyhow::Error>>(())
            }
            .inspect_err(|error| error!("Failed to connect for Raydium V4 monitor: {error}"))
        })
        .await
        .map_err(Into::into)
    }
}

// This is a placeholder for your extract_pool_address_mint_and_signers function
// You'll need to implement this based on your existing code
fn extract_pool_address_mint_and_signers(
    tx_info: &SubscribeUpdateTransactionInfo,
    program_id: &str,
) -> Result<(Option<String>, Option<String>, Vec<String>)> {
    // If either `transaction` or `meta` is missing, just return Ok with empties
    let transaction = tx_info
    .transaction
    .as_ref()
    .context("No top-level `transaction` in tx_info")?;
    let message = transaction
    .message
    .as_ref()
    .context("No `message` in `transaction`")?;

    // Convert all account keys from bytes to base58 strings
    let account_keys: Vec<String> = message
    .account_keys
    .iter()
    .map(|key_bytes| bs58::encode(key_bytes).into_string())
    .collect();

    // The first `num_required_signatures` are signers
    let num_signers = message.header.as_ref().map(|h| h.num_required_signatures).unwrap_or(0);
    let signers = account_keys
    .iter()
    .take(num_signers as usize)
    .cloned()
    .collect::<Vec<_>>();

    let mut pool_address: Option<String> = None;
    let mut minted_token: Option<String> = None;

    // 1) Search top-level instructions for the Raydium program ID
    for ix in &message.instructions {
    let program_id_index = ix.program_id_index as usize;
    if program_id_index >= account_keys.len() {
        continue;
    }
    let program_id = &account_keys[program_id_index];
    if program_id == program_id {
        // The python code picks account_indices[4] as the "pool" - your logic may vary.
        let account_indices: Vec<usize> = ix.accounts.iter().map(|b| *b as usize).collect();
        if account_indices.len() > 4 {
            let pool_idx = account_indices[4];
            if pool_idx < account_keys.len() {
                pool_address = Some(account_keys[pool_idx].clone());
            }
        }
        // Then look for an address that ends in "pump" => minted_token
        for &acct_idx in &account_indices {
            if acct_idx < account_keys.len() {
                let acct_str = &account_keys[acct_idx];
                if acct_str.ends_with("pump") {
                    minted_token = Some(acct_str.clone());
                    break;
                }
            }
        }
        // Once we find it in top-level instructions, we can break
        if minted_token.is_some() || pool_address.is_some() {
            break;
        }
    }
    }

    // 2) If minted_token was not found, we can search the inner instructions
    if minted_token.is_none() {
    if let Some(meta) = &tx_info.meta {
        // Only look if the meta indicates there may be inner instructions
        if !meta.inner_instructions_none {
            for inner_ix_obj in &meta.inner_instructions {
                for inner_ix in &inner_ix_obj.instructions {
                    let account_indices: Vec<usize> = inner_ix.accounts.iter().map(|b| *b as usize).collect();
                    for &acct_idx in &account_indices {
                        if acct_idx < account_keys.len() {
                            let acct_str = &account_keys[acct_idx];
                            if acct_str.ends_with("pump") {
                                minted_token = Some(acct_str.clone());
                                break;
                            }
                        }
                    }
                    if minted_token.is_some() {
                        break;
                    }
                }
                if minted_token.is_some() {
                    break;
                }
            }
        }
    }
    }

    Ok((pool_address, minted_token, signers))
}