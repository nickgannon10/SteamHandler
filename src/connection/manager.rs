// src/connection/manager.rs
use anyhow::{Context, Result};
use backoff::{future::retry, ExponentialBackoff};
use futures::{future::TryFutureExt, sink::SinkExt, stream::StreamExt};
use log::{debug, error, info, warn};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{
    broadcast,
    mpsc::{self, Receiver, Sender},
    Mutex, RwLock,
};
use tonic::transport::channel::ClientTlsConfig;
use yellowstone_grpc_client::{GeyserGrpcClient, Interceptor};
use yellowstone_grpc_proto::prelude::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts,
    SubscribeRequestFilterBlocks, SubscribeRequestFilterBlocksMeta, SubscribeRequestFilterSlots,
    SubscribeRequestFilterTransactions, SubscribeRequestPing,
};

use super::subscription::{Subscription, SubscriptionEvent, SubscriptionId, SubscriptionType};

// Commands that can be sent to the connection manager
#[derive(Debug)]
enum ManagerCommand {
    AddSubscription(Subscription),
    RemoveSubscription(SubscriptionId),
    RemoveClientSubscriptions(String),
    Shutdown,
}

// Events from the stream processor
#[derive(Debug)]
enum StreamEvent {
    ConnectionClosed,
    ConnectionError(String),
    PingReceived,
}

/// Connection manager for Yellowstone gRPC
pub struct ConnectionManager {
    endpoint: String,
    x_token: String,
    command_tx: Sender<ManagerCommand>,
    stream_event_tx: Sender<StreamEvent>,
    shutdown_tx: broadcast::Sender<()>,
    subscriptions: Arc<RwLock<HashMap<SubscriptionId, Subscription>>>,
    client_subscriptions: Arc<RwLock<HashMap<String, HashSet<SubscriptionId>>>>,
}

impl ConnectionManager {
    /// Create a new connection manager with the given gRPC endpoint and authentication token
    pub async fn new(endpoint: String, x_token: String) -> Result<Self> {
        // Validate inputs
        info!("Initializing connection to gRPC server at {}", endpoint);
        if endpoint.is_empty() {
            return Err(anyhow::anyhow!("Empty gRPC endpoint URL provided"));
        }
        if x_token.is_empty() {
            return Err(anyhow::anyhow!("Empty authentication token provided"));
        }
        
        let (command_tx, command_rx) = mpsc::channel(100);
        let (stream_event_tx, stream_event_rx) = mpsc::channel(100);
        let (shutdown_tx, _) = broadcast::channel(1);

        let subscriptions = Arc::new(RwLock::new(HashMap::new()));
        let client_subscriptions = Arc::new(RwLock::new(HashMap::new()));

        let manager = Self {
            endpoint,
            x_token,
            command_tx,
            stream_event_tx,
            shutdown_tx,
            subscriptions,
            client_subscriptions,
        };

        // Start the background task
        manager.spawn_manager_task(command_rx, stream_event_rx).await?;

        Ok(manager)
    }

    /// Add a new subscription to the manager
    pub async fn add_subscription(&self, subscription: Subscription) -> Result<SubscriptionId> {
        let sub_id = subscription.id;
        let client_id = subscription.client_id.clone();

        self.command_tx
            .send(ManagerCommand::AddSubscription(subscription))
            .await
            .context("Failed to send add subscription command")?;

        Ok(sub_id)
    }

    /// Remove a subscription by its ID
    pub async fn remove_subscription(&self, subscription_id: SubscriptionId) -> Result<()> {
        self.command_tx
            .send(ManagerCommand::RemoveSubscription(subscription_id))
            .await
            .context("Failed to send remove subscription command")?;

        Ok(())
    }

    /// Remove all subscriptions for a client
    pub async fn remove_client_subscriptions(&self, client_id: &str) -> Result<()> {
        self.command_tx
            .send(ManagerCommand::RemoveClientSubscriptions(client_id.to_string()))
            .await
            .context("Failed to send remove client subscriptions command")?;

        Ok(())
    }

    /// Shut down the connection manager
    pub async fn shutdown(&self) -> Result<()> {
        // Send shutdown command
        if let Err(e) = self.command_tx.send(ManagerCommand::Shutdown).await {
            error!("Failed to send shutdown command: {}", e);
        }

        // Broadcast shutdown signal
        let _ = self.shutdown_tx.send(());

        Ok(())
    }

    // Spawn the main manager task that handles the gRPC connection
    async fn spawn_manager_task(
        &self,
        command_rx: Receiver<ManagerCommand>,
        stream_event_rx: Receiver<StreamEvent>,
    ) -> Result<()> {
        let endpoint = self.endpoint.clone();
        let x_token = self.x_token.clone();
        let subscriptions = Arc::clone(&self.subscriptions);
        let client_subscriptions = Arc::clone(&self.client_subscriptions);
        let mut shutdown_rx = self.shutdown_tx.subscribe();
        let stream_event_tx = self.stream_event_tx.clone();

        // Define a more sophisticated backoff strategy
        let backoff = ExponentialBackoff {
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            multiplier: 2.0,
            max_elapsed_time: Some(Duration::from_secs(300)), // 5 minutes max retry time
            ..ExponentialBackoff::default()
        };
        
        // Track consecutive failures for circuit breaker
        let consecutive_failures = Arc::new(Mutex::new(0u32));

        // Spawn the connection manager task
        tokio::spawn(async move {
            let mut command_rx = command_rx;
            let mut stream_event_rx = stream_event_rx;

            // Connection state
            let mut client = None;
            let mut subscribe_tx = None;
            let mut needs_update = true;
            let mut last_attempt: Option<std::time::Instant> = None;

            // Main manager loop
            'manager_loop: loop {
                // If we need to update the subscription or don't have a client, try to connect
                if needs_update || client.is_none() {
                    // Implement circuit breaker pattern
                    let failures = {
                        let failures = consecutive_failures.lock().await;
                        *failures
                    };
                    
                    // If we've had too many consecutive failures, wait longer before trying again
                    if failures > 5 {
                        // Check if we need to wait before retrying
                        if let Some(last_time) = last_attempt {
                            let elapsed = last_time.elapsed();
                            let wait_time = Duration::from_secs(failures as u64 * 10);
                            
                            if elapsed < wait_time {
                                let remaining = wait_time.saturating_sub(elapsed);
                                info!("Circuit breaker active: waiting {} seconds before next connection attempt ({} consecutive failures)", 
                                      remaining.as_secs(), failures);
                                tokio::time::sleep(remaining).await;
                            }
                        }
                    }
                    
                    // Update last attempt time
                    last_attempt = Some(std::time::Instant::now());
                    
                    match Self::connect_and_subscribe(
                        &endpoint,
                        &x_token,
                        Arc::clone(&subscriptions),
                        stream_event_tx.clone(),
                    )
                    .await
                    {
                        Ok((new_client, new_subscribe_tx)) => {
                            client = Some(new_client);
                            subscribe_tx = Some(new_subscribe_tx);
                            needs_update = false;
                            info!("Successfully connected to gRPC server");
                            
                            // Reset consecutive failures counter
                            let mut failures = consecutive_failures.lock().await;
                            *failures = 0;
                        }
                        Err(e) => {
                            error!("Failed to connect to gRPC server: {}", e);
                            
                            // Increment consecutive failures counter
                            let mut failures = consecutive_failures.lock().await;
                            *failures += 1;
                            
                            // Calculate backoff time based on failures
                            let backoff_time = backoff.initial_interval
                                .mul_f64(backoff.multiplier.powi(*failures as i32 - 1))
                                .min(backoff.max_interval);
                                
                            info!("Will attempt reconnection in {} seconds (attempt #{})", 
                                  backoff_time.as_secs(), *failures);
                                
                            // Wait before retrying
                            tokio::time::sleep(backoff_time).await;
                            continue;
                        }
                    }
                }

                // Process commands and events
                tokio::select! {
                    // Handle commands from other tasks
                    Some(cmd) = command_rx.recv() => {
                        match cmd {
                            ManagerCommand::AddSubscription(subscription) => {
                                info!("Adding subscription: {:?}", subscription.id);
                                let sub_id = subscription.id;
                                let client_id = subscription.client_id.clone();

                                // Update our subscription registries
                                {
                                    let mut subs = subscriptions.write().await;
                                    subs.insert(sub_id, subscription);
                                }

                                {
                                    let mut client_subs = client_subscriptions.write().await;
                                    client_subs
                                        .entry(client_id)
                                        .or_insert_with(HashSet::new)
                                        .insert(sub_id);
                                }

                                // Flag that we need to update the server subscription
                                needs_update = true;
                            }
                            ManagerCommand::RemoveSubscription(sub_id) => {
                                info!("Removing subscription: {:?}", sub_id);
                                let mut client_id_to_remove = None;

                                // Remove from subscriptions
                                {
                                    let mut subs = subscriptions.write().await;
                                    if let Some(removed) = subs.remove(&sub_id) {
                                        client_id_to_remove = Some(removed.client_id);
                                    }
                                }

                                // Remove from client subscriptions mapping
                                if let Some(client_id) = client_id_to_remove {
                                    let mut client_subs = client_subscriptions.write().await;
                                    if let Some(subs) = client_subs.get_mut(&client_id) {
                                        subs.remove(&sub_id);
                                        if subs.is_empty() {
                                            client_subs.remove(&client_id);
                                        }
                                    }
                                }

                                // Flag that we need to update the server subscription
                                needs_update = true;
                            }
                            ManagerCommand::RemoveClientSubscriptions(client_id) => {
                                info!("Removing all subscriptions for client: {}", client_id);
                                
                                // Get all subscription IDs for this client
                                let sub_ids_to_remove = {
                                    let client_subs = client_subscriptions.read().await;
                                    client_subs
                                        .get(&client_id)
                                        .map(|subs| subs.iter().cloned().collect::<Vec<_>>())
                                        .unwrap_or_default()
                                };

                                // Remove each subscription
                                {
                                    let mut subs = subscriptions.write().await;
                                    for sub_id in &sub_ids_to_remove {
                                        subs.remove(sub_id);
                                    }
                                }

                                // Remove client from client_subscriptions map
                                {
                                    let mut client_subs = client_subscriptions.write().await;
                                    client_subs.remove(&client_id);
                                }

                                // Flag that we need to update the server subscription
                                if !sub_ids_to_remove.is_empty() {
                                    needs_update = true;
                                }
                            }
                            ManagerCommand::Shutdown => {
                                info!("Shutting down connection manager");
                                break 'manager_loop;
                            }
                        }

                        // If we need to update and have a tx channel, update the subscription
                        if needs_update && subscribe_tx.is_some() {
                            let request = Self::build_subscribe_request(&subscriptions).await;
                            let mut tx = subscribe_tx.take().unwrap();
                            
                            match futures::SinkExt::send(&mut tx, request).await {
                                Ok(_) => {
                                    info!("Successfully updated subscriptions");
                                    needs_update = false;
                                    subscribe_tx = Some(tx);
                                }
                                Err(e) => {
                                    error!("Failed to update subscriptions: {}", e);
                                    // Set client to None to force reconnect
                                    client = None;
                                    // subscribe_tx is already None since we took it
                                }
                            }
                        }
                    }

                    // Handle stream events
                    Some(event) = stream_event_rx.recv() => {
                        match event {
                            StreamEvent::ConnectionClosed => {
                                info!("Connection closed, will reconnect");
                                client = None;
                                subscribe_tx = None;
                            }
                            StreamEvent::ConnectionError(err) => {
                                error!("Connection error: {}, will reconnect", err);
                                client = None;
                                subscribe_tx = None;
                            }
                            StreamEvent::PingReceived => {
                                debug!("Received ping, sending pong");
                                if let Some(mut tx) = subscribe_tx.take() {
                                    // Send pong response
                                    let ping_request = SubscribeRequest {
                                        ping: Some(SubscribeRequestPing { id: 1 }),
                                        ..Default::default()
                                    };
                                    
                                    match futures::SinkExt::send(&mut tx, ping_request).await {
                                        Ok(_) => {
                                            // Put the tx back
                                            subscribe_tx = Some(tx);
                                        }
                                        Err(e) => {
                                            error!("Failed to send ping response: {:?}", e);
                                            // Need to reconnect
                                            client = None;
                                            // tx is already dropped
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Handle shutdown signal
                    Ok(_) = shutdown_rx.recv() => {
                        info!("Received shutdown signal");
                        break 'manager_loop;
                    }

                    // All channels closed - this shouldn't normally happen
                    else => {
                        warn!("All channels closed, shutting down connection manager");
                        break 'manager_loop;
                    }
                }
            }

            info!("Connection manager task exiting");
            Result::<()>::Ok(())
        });

        Ok(())
    }

    async fn connect_and_subscribe(
        endpoint: &str,
        x_token: &str,
        subscriptions: Arc<RwLock<HashMap<SubscriptionId, Subscription>>>,
        stream_event_tx: Sender<StreamEvent>,
    ) -> Result<(
        GeyserGrpcClient<impl Interceptor>,
        impl futures::Sink<SubscribeRequest, Error = futures::channel::mpsc::SendError>,
    )> {
        info!("Attempting to connect to gRPC server at {}", endpoint);
        
        // Connect to the server with increased timeouts
        let mut client = GeyserGrpcClient::build_from_shared(endpoint.to_string())?
            .x_token(Some(x_token.to_string()))?
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(30))
            .tls_config(ClientTlsConfig::new().with_native_roots())?
            .max_decoding_message_size(1024 * 1024 * 1024)
            .connect()
            .await
            .context("Failed to establish connection to gRPC server")?;
        
        info!("Successfully connected to gRPC server, setting up subscription");
    
        // Build the initial subscribe request
        let request = Self::build_subscribe_request(&subscriptions).await;
    
        // Add a timeout for the subscription creation
        info!("Creating subscription with server (this may take some time)...");
        let (tx, rx) = tokio::time::timeout(
            Duration::from_secs(60),  // Allow up to 60 seconds for subscription
            client.subscribe_with_request(Some(request))
        ).await
            .context("Subscription creation timed out")?
            .context("Failed to create subscription with gRPC server")?;
        
        info!("Subscription created successfully");
    
        // Spawn a task to process the stream
        Self::spawn_stream_processor(rx, subscriptions, stream_event_tx).await;
    
        Ok((client, tx))
    }

    // Build a Yellowstone subscribe request from our active subscriptions
    async fn build_subscribe_request(
        subscriptions: &RwLock<HashMap<SubscriptionId, Subscription>>,
    ) -> SubscribeRequest {
        let subs = subscriptions.read().await;

        // Maps for each subscription type
        let mut transactions: HashMap<String, SubscribeRequestFilterTransactions> = HashMap::new();
        let mut accounts: HashMap<String, SubscribeRequestFilterAccounts> = HashMap::new();
        let mut slots: HashMap<String, SubscribeRequestFilterSlots> = HashMap::new();
        let mut blocks: HashMap<String, SubscribeRequestFilterBlocks> = HashMap::new();
        let mut blocks_meta: HashMap<String, SubscribeRequestFilterBlocksMeta> = HashMap::new();

        // Apply each subscription to the appropriate map
        for subscription in subs.values() {
            subscription.apply_to_request(&mut transactions, &mut accounts, &mut slots, &mut blocks, &mut blocks_meta);
        }

        SubscribeRequest {
            slots,
            accounts,
            transactions,
            transactions_status: HashMap::default(),
            entry: HashMap::default(),
            blocks,
            blocks_meta,
            commitment: Some(CommitmentLevel::Processed as i32),
            accounts_data_slice: Vec::default(),
            ping: None,
            from_slot: None,
        }
    }

    // Spawn a task to process the subscription stream
    async fn spawn_stream_processor(
        stream: impl futures::stream::Stream<Item = Result<yellowstone_grpc_proto::prelude::SubscribeUpdate, tonic::Status>> + Send + Unpin + 'static,
        subscriptions: Arc<RwLock<HashMap<SubscriptionId, Subscription>>>,
        stream_event_tx: Sender<StreamEvent>,
    ) {
        tokio::spawn(async move {
            let mut stream = stream;
            // Process stream messages
            while let Some(result) = stream.next().await {
                match result {
                    Ok(msg) => {
                        match msg.update_oneof {
                            Some(UpdateOneof::Transaction(tx_msg)) => {
                                if let Some(tx_info) = tx_msg.transaction {
                                    // Handle transaction event
                                    let event = SubscriptionEvent::Transaction(tx_info);
                                    Self::dispatch_event(event, &subscriptions).await;
                                }
                            }
                            Some(UpdateOneof::Account(acct_msg)) => {
                                if let Some(acct_info) = acct_msg.account {
                                    // Handle account event
                                    let event = SubscriptionEvent::Account(acct_info);
                                    Self::dispatch_event(event, &subscriptions).await;
                                }
                            }
                            Some(UpdateOneof::Slot(slot_msg)) => {
                                // Handle slot event
                                let event = SubscriptionEvent::Slot(slot_msg.slot);
                                Self::dispatch_event(event, &subscriptions).await;
                            }
                            Some(UpdateOneof::Block(block_msg)) => {
                                // Handle block event
                                let event = SubscriptionEvent::Block(block_msg);
                                Self::dispatch_event(event, &subscriptions).await;
                            }
                            Some(UpdateOneof::BlockMeta(block_meta_msg)) => {
                                // Handle block meta event
                                let event = SubscriptionEvent::BlockMeta(block_meta_msg);
                                Self::dispatch_event(event, &subscriptions).await;
                            }
                            Some(UpdateOneof::Ping(_)) => {
                                // With the new approach we don't handle ping responses here
                                // The manager loop will handle them with the current tx
                                debug!("Received ping, but handling in manager loop");
                                if let Err(e) = stream_event_tx
                                    .send(StreamEvent::PingReceived)
                                    .await
                                {
                                    error!("Failed to send ping received event: {:?}", e);
                                }
                            }
                            Some(UpdateOneof::Pong(_)) => {
                                // Ignore pong responses
                            }
                            None => {
                                error!("Empty update received");
                            }
                            _ => {
                                // Handle unknown update type
                                warn!("Unknown update type received");
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error from stream: {:?}", e);
                        if let Err(e) = stream_event_tx
                            .send(StreamEvent::ConnectionError(format!("Stream error: {:?}", e)))
                            .await
                        {
                            error!("Failed to send stream event: {:?}", e);
                        }
                        break;
                    }
                }
            }

            // Stream has closed
            info!("Stream closed");
            if let Err(e) = stream_event_tx.send(StreamEvent::ConnectionClosed).await {
                error!("Failed to send connection closed event: {:?}", e);
            }
        });
    }

// Dispatch an event to all matching subscriptions
    async fn dispatch_event(
        event: SubscriptionEvent,
        subscriptions: &RwLock<HashMap<SubscriptionId, Subscription>>,
    ) {
        let subs = subscriptions.read().await;

        for subscription in subs.values() {
            // NEW: only handle if it matches
            if subscription.matches_event(&event) {
                if let Err(e) = subscription.event_handler.handle(event.clone()) {
                    error!(
                        "Error handling event for subscription {:?}: {:?}",
                        subscription.id, e
                    );
                } // Missing closing brace was here
            } // Missing closing brace was here
        }
        // Lock is automatically released when subs goes out of scope
    }
}