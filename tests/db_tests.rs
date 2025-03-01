use anyhow::Result;
use serde_json::json;

// Import our test helpers
mod helpers;
use helpers::{setup_test_database, cleanup_test_database};

#[tokio::test]
async fn test_database_connection() -> Result<()> {
    // Set up the test database
    let db = setup_test_database().await?;
    
    // Test that the connection works
    let pool = &db.pool;
    let result = sqlx::query!("SELECT 1 as one")
        .fetch_one(pool)
        .await?;
    
    assert_eq!(result.one, Some(1));
    
    Ok(())
}

#[tokio::test]
async fn test_insert_and_mark_inactive() -> Result<()> {
    // Set up the test database
    let db = setup_test_database().await?;
    
    // Clean up any existing test data
    cleanup_test_database(&db.pool).await?;
    
    // Test data
    let signature = "test_signature";
    let pool_address = "test_pool_address";
    let token_mint = "test_token_mint";
    let raw_tx = json!({
        "signature": signature,
        "pool_address": pool_address,
        "mint_address": token_mint,
        "test": true
    });
    
    // Test inserting a pool
    db.insert_pool(signature, pool_address, token_mint, &raw_tx).await?;
    
    // Verify the pool was inserted
    let result = sqlx::query!(
        "SELECT pool_address, is_active FROM liquidity_pools_v4 WHERE signature = $1",
        signature
    )
    .fetch_one(&db.pool)
    .await?;
    
    assert_eq!(result.pool_address, pool_address);
    assert!(result.is_active);
    
    // Test marking pools as inactive
    let count = db.mark_all_pools_inactive("test").await?;
    assert_eq!(count, 1);
    
    // Verify the pool was marked inactive
    let result = sqlx::query!(
        "SELECT is_active, death_reason FROM liquidity_pools_v4 WHERE signature = $1",
        signature
    )
    .fetch_one(&db.pool)
    .await?;
    
    assert!(!result.is_active);
    assert_eq!(result.death_reason, Some("test".to_string()));
    
    // Clean up test data
    cleanup_test_database(&db.pool).await?;
    
    Ok(())
}