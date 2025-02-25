// src/orchestrator.rs
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
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>, // Change type here
    // Channels for each monitor type
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
    
    // Callback for processing Raydium V4 pool events
    async fn handle_raydium_v4_event(&self, event: RaydiumV4PoolEvent) -> Result<()> {
        info!(
            "CALLBACK: initialize2 found! signature={}, pool={:?}, mint={:?}, signers={:?}",
            event.signature, event.pool_address, event.mint_address, event.signers
        );
        
        // Additional business logic can be added here
        // For example, storing in the database:
        // self.database.store_raydium_pool(&event).await?;
        
        // Or triggering other business processes:
        // self.notify_admins(&event).await?;
        // self.analyze_market_impact(&event).await?;
        
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
        
        // Clone database for the callback processor
        let db = Arc::clone(&self.database);
        let orchestrator_self = Arc::new(self.clone());
        
        // Spawn a task to process events
        let event_handle = tokio::spawn(async move {
            info!("Raydium V4 event processor started");
            while let Some(event) = event_rx.recv().await {
                if let Err(e) = orchestrator_self.handle_raydium_v4_event(event).await {
                    error!("Error processing Raydium V4 event: {}", e);
                }
            }
            info!("Raydium V4 event processor stopped");
        });
        
        self.monitor_handles.push(event_handle);
        
        // Initialize the Raydium V4 monitor
        let raydium_monitor = RaydiumV4Monitor::new(
            settings.shyft.endpoint.clone(),
            settings.shyft.x_token.clone(),
            settings.transaction_monitor.program_id.clone(),
            settings.transaction_monitor.migration_pubkey.clone(),
            Arc::clone(&self.database),
            event_tx, // Pass the event sender channel
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
    
    // Add Clone implementation for Orchestrator
    pub fn clone(&self) -> Self {
        Self {
            database: Arc::clone(&self.database),
            monitor_handles: Vec::new(), // Don't clone handles
            shutdown_tx: None, // Don't clone shutdown channel
            raydium_v4_tx: self.raydium_v4_tx.clone(),
        }
    }
    
    pub async fn run(mut self) -> Result<()> {
        info!("All monitors started. Press Ctrl+C to stop...");
        
        // Wait for Ctrl+C
        tokio::signal::ctrl_c().await?;
        
        info!("Shutdown signal received, stopping monitors...");
        
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
}