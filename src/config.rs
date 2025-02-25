// src/config.rs
use config::{Config, ConfigError, File, FileFormat};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppSettings {
    pub raydium_liquidity_pool_v4: String,
}

#[derive(Debug, Deserialize)]
pub struct ShyftSettings {
    pub endpoint: String,
    pub x_token: String,
    pub commitment_level: String,
}

#[derive(Debug, Deserialize)]
pub struct TransactionMonitorSettings {
    pub program_id: String,
    pub migration_pubkey: String,
    pub filter_initialize2: bool,
    pub filter_specific_signer: bool,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseSettings {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub app: AppSettings,
    pub shyft: ShyftSettings,
    pub transaction_monitor: TransactionMonitorSettings,
    pub database: DatabaseSettings,
}

impl Settings {
    /// Load the config from `Settings.toml` plus optional environment overrides.
    pub fn new() -> Result<Self, ConfigError> {
        // Configure our builder
        let builder = Config::builder()
            // 1) Read the default file-based config
            .add_source(File::new("Settings", FileFormat::Toml))
            .add_source(config::Environment::default().separator("__"));

        // Build the config and deserialize
        let config = builder.build()?;
        config.try_deserialize()
    }
}
