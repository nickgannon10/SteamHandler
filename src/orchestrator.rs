// src/orchestrator.rs
use anyhow::Result;
use log::{error, info};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::config::Settings;
use crate::connection::{ConnectionManager, Subscription, SubscriptionType};
use crate::db::Database;
use crate::monitors::raydium_v4::{RaydiumV4Monitor, RaydiumV4PoolEvent};

pub struct Orchestrator {
    database: Arc<Database>,
    connection_manager: Arc<ConnectionManager>,
    monitor_handles: Vec<JoinHandle<()>>,
}

impl Orchestrator {
    pub async fn new(database: Database, settings: &Settings) -> Result<Self> {
        // Create the connection manager
        let connection_manager = ConnectionManager::new(
            settings.shyft.endpoint.clone(),
            settings.shyft.x_token.clone(),
        ).await?;

        Ok(Self {
            database: Arc::new(database),
            connection_manager: Arc::new(connection_manager),
            monitor_handles: Vec::new(),
        })
    }

    async fn handle_raydium_v4_event(&self, event: RaydiumV4PoolEvent) -> Result<()> {
        info!(
            "Processing Raydium V4 event: signature={}, pool={:?}, mint={:?}",
            event.signature, event.pool_address, event.mint_address
        );
        
        // Extract pool_address and mint_address
        if let (Some(pool_address), Some(mint_address)) = (&event.pool_address, &event.mint_address) {
            // Create a JSON representation of the event for the raw_transaction field
            let raw_tx = serde_json::json!({
                "signature": event.signature,
                "pool_address": pool_address,
                "mint_address": mint_address,
                "signers": event.signers,
                "timestamp": event.timestamp,
                "logs": event.raw_logs
            });
            
            // Insert the pool data into the database
            match self.database.insert_pool(
                &event.signature,
                pool_address,
                mint_address,
                &raw_tx
            ).await {
                Ok(_) => info!("Successfully inserted new pool: {}", pool_address),
                Err(e) => error!("Failed to insert pool {}: {}", pool_address, e)
            }
        } else {
            error!("Missing pool_address or mint_address in event: {}", event.signature);
        }
        
        Ok(())
    }

    pub async fn setup_raydium_v4_monitor(&mut self, settings: &Settings) -> Result<()> {
        info!("Setting up Raydium V4 monitor...");
        
        // Create a RaydiumV4Monitor
        let raydium_monitor = Arc::new(RaydiumV4Monitor::new(
            settings.transaction_monitor.program_id.clone(),
            settings.transaction_monitor.migration_pubkey.clone(),
        ));
        
        // Create a transaction subscription
        let db_clone = Arc::clone(&self.database);
        let monitor_clone: Arc<RaydiumV4Monitor> = Arc::clone(&raydium_monitor);
        let migration_pubkey = settings.transaction_monitor.migration_pubkey.clone();
        
        let subscription = raydium_monitor.create_subscription(
            "raydium_v4",
            move |event| {
                // Process the event
                let db = db_clone.clone();
                let monitor = monitor_clone.clone();
                let migration_pubkey = migration_pubkey.clone();
                
                tokio::spawn(async move {
                    // Create a synthetic RaydiumV4PoolEvent from the SubscriptionEvent
                    let pool_event = match monitor.create_pool_event_from_subscription(event) {
                        Ok(evt) => evt,
                        Err(e) => {
                            error!("Failed to create pool event: {}", e);
                            return;
                        }
                    };
                    
                    // Get log messages and check criteria
                    let logs = &pool_event.raw_logs;
                    let has_initialize2 = logs.iter().any(|log_line| log_line.contains("initialize2"));
                    let has_target_signer = pool_event.signers.contains(&migration_pubkey);
                    
                    // Only process if it meets our criteria
                    if !has_initialize2 || !has_target_signer {
                        // Not an event we're interested in
                        return;
                    }
                    
                    // Extract pool_address and mint_address
                    if let (Some(pool_address), Some(mint_address)) = (&pool_event.pool_address, &pool_event.mint_address) {
                        // Create a JSON representation of the event
                        let raw_tx = serde_json::json!({
                            "signature": pool_event.signature,
                            "pool_address": pool_address,
                            "mint_address": mint_address,
                            "signers": pool_event.signers,
                            "timestamp": pool_event.timestamp,
                            "logs": pool_event.raw_logs
                        });
                        
                        // Insert into database
                        match db.insert_pool(
                            &pool_event.signature,
                            pool_address,
                            mint_address,
                            &raw_tx
                        ).await {
                            Ok(_) => info!("Successfully inserted new pool: {}", pool_address),
                            Err(e) => error!("Failed to insert pool {}: {}", pool_address, e)
                        }
                    } else {
                        error!("Missing pool_address or mint_address in event: {}", pool_event.signature);
                    }
                });
                
                Ok(())
            }
        )?;
        
        // Add the subscription to the connection manager
        self.connection_manager.add_subscription(subscription).await?;
        
        info!("Raydium V4 monitor setup complete");
        
        Ok(())
    }
    
    pub async fn run(self) -> Result<()> {
        info!("Orchestrator running. Press Ctrl+C to stop...");
        
        // Wait for Ctrl+C
        tokio::signal::ctrl_c().await?;
        
        info!("Shutdown signal received, stopping...");
        
        // Mark all active pools as inactive
        info!("Marking all active pools as inactive...");
        match self.database.mark_all_pools_inactive("control+c").await {
            Ok(count) => info!("Marked {} pools as inactive", count),
            Err(e) => error!("Failed to mark pools as inactive: {}", e)
        }
        
        // Shutdown the connection manager
        info!("Shutting down connection manager...");
        self.connection_manager.shutdown().await?;
        
        info!("All systems shutdown gracefully");
        
        Ok(())
    }
}