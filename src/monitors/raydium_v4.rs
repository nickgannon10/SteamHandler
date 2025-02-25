// src/monitors/raydium_v4.rs
use {
    anyhow::{Context, Result},
    chrono,
    log::{error, info},
    solana_sdk::signature::Signature,
    std::collections::HashMap,
    yellowstone_grpc_proto::prelude::{
        SubscribeRequestFilterTransactions, SubscribeUpdateTransactionInfo,
    },
};

use crate::connection::{Subscription, SubscriptionEvent, SubscriptionType};

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

#[derive(Clone)]
pub struct RaydiumV4Monitor {
    program_id: String,
    migration_pubkey: String,
}

impl RaydiumV4Monitor {
    pub fn new(program_id: String, migration_pubkey: String) -> Self {
        Self {
            program_id,
            migration_pubkey,
        }
    }

    /// Create a subscription for the Raydium V4 program
    pub fn create_subscription<F>(
        &self,
        client_id: impl Into<String>,
        handler: F,
    ) -> Result<Subscription>
    where
        F: Fn(SubscriptionEvent) -> Result<()> + Send + Sync + 'static,
    {
        // Create a transaction filter
        let filter = SubscribeRequestFilterTransactions {
            vote: None,
            failed: Some(false),
            signature: None,
            account_include: vec![self.program_id.clone()],
            account_exclude: vec![],
            account_required: vec![],
        };

        // Create the subscription
        Ok(Subscription::new_transactions(
            client_id,
            filter,
            format!("Raydium V4 Monitor for program {}", self.program_id),
            handler,
        ))
    }

    /// Process a transaction event from the subscription
    pub fn create_pool_event_from_subscription(
        &self,
        event: SubscriptionEvent,
    ) -> Result<RaydiumV4PoolEvent> {
        match event {
            SubscriptionEvent::Transaction(tx_info) => self.process_transaction(&tx_info),
            _ => {
                anyhow::bail!("Expected transaction event but got something else")
            }
        }
    }

    /// Process a transaction to extract pool information
    fn process_transaction(
        &self,
        tx_info: &SubscribeUpdateTransactionInfo,
    ) -> Result<RaydiumV4PoolEvent> {
        // Parse out the "pool address", "mint token", and signers
        let (maybe_pool, maybe_mint, signers) =
            extract_pool_address_mint_and_signers(tx_info, &self.program_id)?;

        let signature_b58 = match Signature::try_from(tx_info.signature.as_slice()) {
            Ok(sig) => sig.to_string(),
            Err(_) => String::from("invalid-signature"),
        };

        // Get log messages
        let log_messages = if let Some(meta) = &tx_info.meta {
            if !meta.log_messages_none {
                meta.log_messages.clone()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        // Check if "initialize2" is in the logs
        let has_initialize2 = log_messages
            .iter()
            .any(|log_line| log_line.contains("initialize2"));

        // Check if our target pubkey is a signer
        let has_target_signer = signers.contains(&self.migration_pubkey);

        // Create event
        let event = RaydiumV4PoolEvent {
            signature: signature_b58,
            pool_address: maybe_pool,
            mint_address: maybe_mint,
            signers,
            timestamp: chrono::Utc::now().timestamp() as u64,
            raw_logs: log_messages,
        };

        // Only log successful match for debugging
        if has_initialize2 && has_target_signer {
            info!("Detected initialize2 event: {}", event.signature);
        }

        Ok(event)
    }
}

/// Extract pool address, mint address, and signers from a transaction
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
    let num_signers = message
        .header
        .as_ref()
        .map(|h| h.num_required_signatures)
        .unwrap_or(0);
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
        let program_key = &account_keys[program_id_index];
        if program_key == program_id {
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
                        let account_indices: Vec<usize> =
                            inner_ix.accounts.iter().map(|b| *b as usize).collect();
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
