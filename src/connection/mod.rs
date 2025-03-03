// src/connection/mod.rs
mod manager;
mod subscription;
pub mod rpc;

pub use manager::ConnectionManager;
pub use subscription::{Subscription, SubscriptionEvent, SubscriptionId, SubscriptionType};
