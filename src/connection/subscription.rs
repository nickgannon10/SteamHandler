// src/connection/subscription.rs
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use yellowstone_grpc_proto::prelude::{
    SubscribeRequestFilterAccounts, SubscribeRequestFilterBlocks, SubscribeRequestFilterBlocksMeta,
    SubscribeRequestFilterSlots, SubscribeRequestFilterTransactions,
};

// Generate unique subscription IDs
static NEXT_SUB_ID: AtomicU64 = AtomicU64::new(1);

/// Unique identifier for a subscription
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(u64);

impl SubscriptionId {
    pub fn new() -> Self {
        Self(NEXT_SUB_ID.fetch_add(1, Ordering::SeqCst))
    }
}

impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sub_{}", self.0)
    }
}

/// Types of subscriptions supported by the connection manager
#[derive(Debug, Clone)]
pub enum SubscriptionType {
    Transactions(SubscribeRequestFilterTransactions),
    Accounts(SubscribeRequestFilterAccounts),
    Slots(SubscribeRequestFilterSlots),
    Blocks(SubscribeRequestFilterBlocks),
    BlocksMeta(SubscribeRequestFilterBlocksMeta),
}

/// Represents a single subscription with its configuration
#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: SubscriptionId,
    pub client_id: String,
    pub subscription_type: SubscriptionType,
    pub event_handler: SubscriptionEventHandler,
}

/// Function type for handling events from a subscription
pub struct SubscriptionEventHandler {
    // Using a string description for better debug output
    description: String,
    // The actual handler function
    #[allow(clippy::type_complexity)]
    handler: Box<dyn Fn(SubscriptionEvent) -> anyhow::Result<()> + Send + Sync>,
}

impl SubscriptionEventHandler {
    pub fn new<F, S>(description: S, handler: F) -> Self
    where
        F: Fn(SubscriptionEvent) -> anyhow::Result<()> + Send + Sync + 'static,
        S: Into<String>,
    {
        Self {
            description: description.into(),
            handler: Box::new(handler),
        }
    }

    pub fn handle(&self, event: SubscriptionEvent) -> anyhow::Result<()> {
        (self.handler)(event)
    }
}

impl fmt::Debug for SubscriptionEventHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubscriptionEventHandler")
            .field("description", &self.description)
            .finish()
    }
}

// Custom Clone implementation for SubscriptionEventHandler
// This won't actually clone the function, but will create a new handler that panics
// This is acceptable since we don't expect clones to be used for actual event handling
impl Clone for SubscriptionEventHandler {
    fn clone(&self) -> Self {
        let desc = self.description.clone();
        // Create a placeholder handler that logs and returns Ok
        Self {
            description: format!("{} (cloned - non-functional)", desc),
            handler: Box::new(move |_| {
                log::warn!("Called a cloned event handler: {}", desc);
                Ok(())
            }),
        }
    }
}

/// Events that can be received from subscriptions
#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    Transaction(yellowstone_grpc_proto::prelude::SubscribeUpdateTransactionInfo),
    Account(yellowstone_grpc_proto::prelude::SubscribeUpdateAccountInfo),
    Slot(u64),
    Block(yellowstone_grpc_proto::prelude::SubscribeUpdateBlock),
    BlockMeta(yellowstone_grpc_proto::prelude::SubscribeUpdateBlockMeta),
}

impl Subscription {
    pub fn new_transactions<F>(
        client_id: impl Into<String>,
        filter: SubscribeRequestFilterTransactions,
        handler_description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(SubscriptionEvent) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        Self {
            id: SubscriptionId::new(),
            client_id: client_id.into(),
            subscription_type: SubscriptionType::Transactions(filter),
            event_handler: SubscriptionEventHandler::new(handler_description.into(), handler),
        }
    }

    pub fn new_accounts<F>(
        client_id: impl Into<String>,
        filter: SubscribeRequestFilterAccounts,
        handler_description: impl Into<String>,
        handler: F,
    ) -> Self
    where
        F: Fn(SubscriptionEvent) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        Self {
            id: SubscriptionId::new(),
            client_id: client_id.into(),
            subscription_type: SubscriptionType::Accounts(filter),
            event_handler: SubscriptionEventHandler::new(handler_description.into(), handler),
        }
    }

    // Similar constructor methods for other subscription types...

    /// Apply this subscription to the appropriate field in a SubscribeRequest maps
    pub fn apply_to_request(
        &self,
        transactions: &mut HashMap<String, SubscribeRequestFilterTransactions>,
        accounts: &mut HashMap<String, SubscribeRequestFilterAccounts>,
        slots: &mut HashMap<String, SubscribeRequestFilterSlots>,
        blocks: &mut HashMap<String, SubscribeRequestFilterBlocks>,
        blocks_meta: &mut HashMap<String, SubscribeRequestFilterBlocksMeta>,
    ) {
        match &self.subscription_type {
            SubscriptionType::Transactions(filter) => {
                transactions.insert(self.client_id.clone(), filter.clone());
            }
            SubscriptionType::Accounts(filter) => {
                accounts.insert(self.client_id.clone(), filter.clone());
            }
            SubscriptionType::Slots(filter) => {
                slots.insert(self.client_id.clone(), filter.clone());
            }
            SubscriptionType::Blocks(filter) => {
                blocks.insert(self.client_id.clone(), filter.clone());
            }
            SubscriptionType::BlocksMeta(filter) => {
                blocks_meta.insert(self.client_id.clone(), filter.clone());
            }
        }
    }
    /// Returns true if the given event matches the filter criteria of this subscription.
    /// If it does, the event_handler should be invoked; otherwise, skip it.
    pub fn matches_event(&self, event: &SubscriptionEvent) -> bool {
        match (&self.subscription_type, event) {
            // For transaction subscriptions, check if the transaction accounts include or match the filter
            (
                SubscriptionType::Transactions(tx_filter),
                SubscriptionEvent::Transaction(tx_info),
            ) => Self::matches_transaction(tx_filter, tx_info),

            // For accounts, blocks, etc., you can add similar logic as needed:
            (SubscriptionType::Accounts(filter), SubscriptionEvent::Account(account_info)) => {
                // Perhaps also some local checks against the filter's account list, etc.
                true
            }
            (SubscriptionType::Slots(_), SubscriptionEvent::Slot(_)) => {
                true // or do additional checks if needed
            }
            (SubscriptionType::Blocks(_), SubscriptionEvent::Block(_)) => {
                true // or a deeper check
            }
            (SubscriptionType::BlocksMeta(_), SubscriptionEvent::BlockMeta(_)) => {
                true
            }

            // If the event is a different type than this subscription expects, it's not a match
            _ => false,
        }
    }

    fn matches_transaction(
        tx_filter: &SubscribeRequestFilterTransactions,
        tx_info: &yellowstone_grpc_proto::prelude::SubscribeUpdateTransactionInfo,
    ) -> bool {
        // Perform local checks to see if the transaction touches the included program IDs, etc.

        // 1) If the filter says `failed = Some(false)`, we can check if this transaction actually
        // succeeded. (We can glean that from `tx_info.meta.as_ref().map(|m| m.err.is_none()).unwrap_or(false)`.)

        // 2) If there's an 'account_include' list in tx_filter, confirm that the transaction indeed has
        // those accounts. For example:
        if !tx_filter.account_include.is_empty() {
            // Convert the transaction's account_keys to base58
            if let Some(tx) = &tx_info.transaction {
                if let Some(message) = &tx.message {
                    let account_keys: Vec<String> = message
                        .account_keys
                        .iter()
                        .map(|key_bytes| bs58::encode(key_bytes).into_string())
                        .collect();

                    // Check if every required account_include is present
                    for required in &tx_filter.account_include {
                        // The subscription's `program_id` etc. is in base58 string form. 
                        // We require that 'required' is among `account_keys`.
                        if !account_keys.contains(required) {
                            return false;
                        }
                    }
                }
            }
        }

        // If we pass all relevant checks, then yes, this transaction belongs to this subscription
        true
    }
}
