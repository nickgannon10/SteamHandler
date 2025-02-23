// src/main.rs
mod db;
mod config;

use db::Database;
use config::Settings;
use anyhow::Result;
use dotenvy::dotenv;
use tokio;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let settings = Settings::new()?;

    let database = Database::connect(
        &settings.database.url,
        settings.database.max_connections
    )
    .await?;

    test_query(&database).await?;

    println!("Connected to Postgres successfully!");

    Ok(())
}

async fn test_query(db: &Database) -> Result<()> {

    let rows = sqlx::query!(
        "SELECT signature, pool_address FROM liquidity_pools LIMIT 1;"
    )
    .fetch_all(&db.pool).await?;

    for rec in rows {
        println!("{} => {}", rec.signature, rec.pool_address);
    }
    Ok(())
}
