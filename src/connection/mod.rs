// src/connection/mod.rs
mod manager;
mod subscription;

pub use manager::ConnectionManager;
pub use subscription::{Subscription, SubscriptionEvent, SubscriptionId, SubscriptionType};
