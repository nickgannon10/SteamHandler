// src/orchestrator.rs
use anyhow::Result;
use log::info;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use yellowstone_grpc_proto::prelude::SubscribeUpdateTransactionInfo;

use crate::listener::RaydiumListener;

pub struct Orchestrator {
    // We hold the listener if we wish, or we can store config values and reconstruct as needed
    listener: RaydiumListener,
    // The receiving side of messages from the listener
    rx: UnboundedReceiver<SubscribeUpdateTransactionInfo>,
}

impl Orchestrator {
    pub fn new(endpoint: String, x_token: String, program_id: String) -> Self {
        // Build channel
        let (tx, rx) = mpsc::unbounded_channel();
        
        // Build the listener with the sending half
        let listener = RaydiumListener::new(endpoint, x_token, program_id, tx);
        
        Self { listener, rx }
    }

    pub async fn run(mut self) -> Result<()> {
        // 1) We can either run the listener in a background task, or inline.
        //    Here we do it in a background task so we can simultaneously process messages from rx.

        let mut listener_clone = self.listener;
        let listener_task = tokio::spawn(async move {
            if let Err(e) = listener_clone.run().await {
                eprintln!("Listener encountered error: {:?}", e);
            }
        });

        // 2) Meanwhile, we (Orchestrator) read from the channel to react to events
        while let Some(tx_info) = self.rx.recv().await {
            // In the future, we might parse logs, check for "initialize2", etc.
            // For now, we simply log that we've received a transaction.
            info!("Orchestrator: Received a transaction update from the listener.");

            // Perhaps we’d parse the logs or signature to do something more interesting.
            // For demonstration, we do nothing further.
        }

        // If the channel closes, or an error occurs, join the listener task
        let _ = listener_task.await;
        Ok(())
    }
}
