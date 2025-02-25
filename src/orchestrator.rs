// Improved orchestrator.rs implementation
use anyhow::Result;
use log::{error, info};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::config::Settings;
use crate::db::Database;
use crate::monitors::raydium_v4::{RaydiumV4Monitor, RaydiumV4PoolEvent};

pub struct Orchestrator {
    database: Arc<Database>,
    monitor_handles: Vec<JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    raydium_v4_tx: Option<mpsc::Sender<RaydiumV4PoolEvent>>,
}

impl Orchestrator {
    pub fn new(database: Database) -> Self {
        Self {
            database: Arc::new(database),
            monitor_handles: Vec::new(),
            shutdown_tx: None,
            raydium_v4_tx: None,
        }
    }

    // Use &self instead of self since we're only reading the database
    async fn handle_raydium_v4_event(&self, event: RaydiumV4PoolEvent) -> Result<()> {
        info!(
            "CALLBACK: initialize2 found! signature={}, pool={:?}, mint={:?}, signers={:?}",
            event.signature, event.pool_address, event.mint_address, event.signers
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
        
        // Create shutdown channel
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        self.shutdown_tx = Some(shutdown_tx.clone());
        let shutdown_rx = shutdown_tx.subscribe();
        
        // Create event channel for callbacks
        let (event_tx, mut event_rx) = mpsc::channel::<RaydiumV4PoolEvent>(100);
        self.raydium_v4_tx = Some(event_tx.clone());
        
        // Create shared data for the event handler
        // Clone only what's needed for the handler, not the entire self
        let db_for_handler = Arc::clone(&self.database);
        
        // Spawn a task to process events
        let event_handle = tokio::spawn(async move {
            info!("Raydium V4 event processor started");
            while let Some(event) = event_rx.recv().await {
                // Process event directly in the closure
                // This avoids the need to move `self` into the closure
                let signature = &event.signature;
                
                if let (Some(pool_address), Some(mint_address)) = (&event.pool_address, &event.mint_address) {
                    // Create JSON representation
                    let raw_tx = serde_json::json!({
                        "signature": event.signature,
                        "pool_address": pool_address,
                        "mint_address": mint_address,
                        "signers": event.signers,
                        "timestamp": event.timestamp,
                        "logs": event.raw_logs
                    });
                    
                    // Insert into database
                    match db_for_handler.insert_pool(
                        signature,
                        pool_address,
                        mint_address,
                        &raw_tx
                    ).await {
                        Ok(_) => info!("Successfully inserted new pool: {}", pool_address),
                        Err(e) => error!("Failed to insert pool {}: {}", pool_address, e)
                    }
                } else {
                    error!("Missing pool_address or mint_address in event: {}", signature);
                }
            }
            info!("Raydium V4 event processor stopped");
        });
        
        self.monitor_handles.push(event_handle);
        
        // Initialize the Raydium V4 monitor
        // No need to clone the database reference again
        let raydium_monitor = RaydiumV4Monitor::new(
            settings.shyft.endpoint.clone(),
            settings.shyft.x_token.clone(), 
            settings.transaction_monitor.program_id.clone(),
            settings.transaction_monitor.migration_pubkey.clone(),
            Arc::clone(&self.database),
            event_tx,
        );
        
        // Spawn the monitor in its own task
        let handle = tokio::spawn(async move {
            match raydium_monitor.run(shutdown_rx).await {
                Ok(_) => info!("Raydium V4 monitor completed successfully"),
                Err(e) => error!("Raydium V4 monitor error: {}", e),
            }
        });
        
        self.monitor_handles.push(handle);
        
        Ok(())
    }
    
    pub async fn run(mut self) -> Result<()> {
        info!("All monitors started. Press Ctrl+C to stop...");
        
        // Wait for Ctrl+C
        tokio::signal::ctrl_c().await?;
        
        info!("Shutdown signal received, stopping monitors...");
        
        // Mark all active pools as inactive with "control+c" reason
        info!("Marking all active pools as inactive...");
        match self.mark_all_pools_inactive("control+c").await {
            Ok(count) => info!("Marked {} pools as inactive", count),
            Err(e) => error!("Failed to mark pools as inactive: {}", e)
        }
        
        // First, drop all event senders to terminate event processor tasks
        self.raydium_v4_tx = None;
        
        // Then send shutdown signal to all monitors
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
            // Explicitly drop the sender to ensure all receivers get notified
            drop(tx);
        }
        
        // Wait for all handles to complete with a timeout
        let shutdown_timeout = tokio::time::Duration::from_secs(5);
        for handle in self.monitor_handles {
            match tokio::time::timeout(shutdown_timeout, handle).await {
                Ok(_) => {},
                Err(_) => {
                    error!("Timeout waiting for task to complete, forcing shutdown");
                    // We've given up waiting
                }
            }
        }
        
        info!("All monitors stopped, shutting down gracefully");
        
        Ok(())
    }
    
    async fn mark_all_pools_inactive(&self, reason: &str) -> Result<i64> {
        // We'll create a new method in the Database struct to handle this operation
        self.database.mark_all_pools_inactive(reason).await
    }
}