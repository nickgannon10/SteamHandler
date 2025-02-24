// src/main.rs
mod db;
mod config;
mod listener;
mod orchestrator;

use config::Settings;
use db::Database;
use dotenvy::dotenv;

use anyhow::Result;
use env_logger;
use orchestrator::Orchestrator;
use tokio::main;

#[main]
async fn main() -> Result<()> {
    dotenv().ok();
    env_logger::init();

    let settings = Settings::new()?;

    let database = Database::connect(
        &settings.database.url,
        settings.database.max_connections
    )
    .await?;

    // Get the endpoint, token, and program ID from your config
    let endpoint = settings.shyft.endpoint;
    let x_token = settings.shyft.x_token;
    let program_id = settings.app.raydium_liquidity_pool_v4;

    // Build orchestrator
    let orchestrator = Orchestrator::new(endpoint, x_token, program_id);

    // Kick it off
    orchestrator.run().await?;

    Ok(())
}
