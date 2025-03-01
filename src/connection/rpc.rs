// src/connection/rpc.rs

use helius::error::Result as HeliusResult;
use helius::types::{Cluster, ParseTransactionsRequest, EnhancedTransaction};
use helius::Helius;
use crate::config::HeliusSettings;

// REMOVE #[derive(Debug)] as Helius doesn't implement Debug
pub struct RpcClient {
    helius: Helius,
}

impl RpcClient {
    /// Constructs an RpcClient from your config
    pub fn new_from_settings(settings: &HeliusSettings) -> Self {
        let cluster = match settings.cluster.as_deref() {
            Some("MainnetBeta") => Cluster::MainnetBeta,
            _ => Cluster::MainnetBeta, // default
        };

        let helius = Helius::new(&settings.api_key, cluster)
            .expect("Failed to create Helius client");

        RpcClient { helius }
    }

    /// Fetch a parsed transaction without cloning
    pub async fn get_parsed_transaction(
        &self,
        signature: &str
    ) -> HeliusResult<Option<EnhancedTransaction>> {
        let request = ParseTransactionsRequest {
            transactions: vec![signature.to_owned()],
        };

        let mut parsed_txs = self.helius.parse_transactions(request).await?;

        // Avoid clone: use pop() instead
        Ok(parsed_txs.pop())
    }
}
