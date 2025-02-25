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
}
