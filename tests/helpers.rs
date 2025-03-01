// tests/helpers.rs
use anyhow::Result;
use sqlx::{Postgres, Pool};
use dotenvy::dotenv;
use trade_streamer::db::Database;

/// Creates a test database connection for testing purposes.
/// This ensures we're using the test database URL from environment 
/// variables rather than the production one.
pub async fn setup_test_database() -> Result<Database> {
    // Make sure we load environment variables
    dotenv().ok();
    
    // Use the database URL from environment
    let database_url = std::env::var("DATABASE__URL")
        .expect("DATABASE__URL must be set for tests");
    
    let max_connections = 5; // Reasonable default for tests
    
    // Create the database connection
    let db = Database::connect(&database_url, max_connections).await?;
    
    // Return the database
    Ok(db)
}

/// Clean up the test database by removing test data
pub async fn cleanup_test_database(pool: &Pool<Postgres>) -> Result<()> {
    // Remove test data but keep the table structure
    sqlx::query!("DELETE FROM liquidity_pools_v4")
        .execute(pool)
        .await?;
    
    Ok(())
}