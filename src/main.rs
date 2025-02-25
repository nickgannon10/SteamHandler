// src/main.rs
mod config;
mod connection;
mod db;
mod monitors;
mod orchestrator;

use config::Settings;
use db::Database;
use dotenvy::dotenv;

use anyhow::Result;
use env_logger;
use log::info;
use orchestrator::Orchestrator;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    env_logger::init();

    info!("Starting application...");
    let settings = Settings::new()?;

    // Set up database connection
    let database = Database::connect(
        &settings.database.url,
        settings.database.max_connections
    )
    .await?;

    // Initialize the orchestrator with connection manager
    let mut orchestrator = Orchestrator::new(database, &settings).await?;
    
    // Set up the Raydium V4 monitor
    orchestrator.setup_raydium_v4_monitor(&settings).await?;
    
    // Run the orchestrator (this will block until shutdown)
    orchestrator.run().await?;

    Ok(())
}