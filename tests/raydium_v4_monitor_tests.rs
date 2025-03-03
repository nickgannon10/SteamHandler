// tests/raydium_v4_test.rs

use anyhow::Result;
use std::env;
use trade_streamer::connection::rpc::RpcClient;
use trade_streamer::config::HeliusSettings;
use trade_streamer::monitors::raydium_v4::RaydiumV4Monitor;

#[path = "helpers.rs"]
mod helpers;
use helpers::{setup_test_database, cleanup_test_database};

#[tokio::test]
async fn test_raydium_v4_with_helius_data() -> Result<()> {
    println!("\n===== RAYDIUM V4 TEST WITH HELIUS DATA =====\n");
    
    // 1. Setup the database
    println!("Step 1: Setting up test database...");
    let db = setup_test_database().await?;
    println!("- Database connection established");
    
    cleanup_test_database(&db.pool).await?;
    println!("- Test database cleaned");
    
    // 2. Create the Helius client
    println!("\nStep 2: Setting up Helius client...");
    let api_key = env::var("HELIUS__API_KEY")
        .expect("HELIUS__API_KEY must be set for tests");
    
    println!("- Using API key: {}...", &api_key[0..std::cmp::min(5, api_key.len())]);
    
    let helius_settings = HeliusSettings {
        api_key,
        cluster: Some("MainnetBeta".to_string()),
    };
    
    let rpc_client = RpcClient::new_from_settings(&helius_settings);
    println!("- Helius client created");
    
    // 3. Create the Raydium V4 monitor
    println!("\nStep 3: Creating Raydium V4 monitor...");
    let program_id = "4RRvpTvhJxgLcY7EXA23ikCkJ6J7SYAKEzhMgDDEXc7k";
    let migration_pubkey = "4KLx5dQZC2ZGPJgK7Ej66rBPyQJ2VQzRsUTEFoUr6C3w";
    
    println!("- Program ID: {}", program_id);
    println!("- Migration pubkey: {}", migration_pubkey);
    
    let monitor = RaydiumV4Monitor::new(
        program_id.to_string(),
        migration_pubkey.to_string(),
    );
    
    println!("- Monitor created");
    
    // 4. Fetch a transaction from Helius
    println!("\nStep 4: Fetching transaction from Helius...");
    // Replace with a known Raydium V4 transaction signature
    let signature = "4Rh1vsYXhP8Esyh8b4VhFJ1kEPw5zo5GDvZqPz5QGNjB2oPV3JrXKVGBUqP9x58TxHTKUwScDdEkaCxzaahZmxf4";
    
    println!("- Fetching transaction: {}", signature);
    
    let tx = match rpc_client.get_parsed_transaction(&signature).await? {
        Some(tx) => {
            println!("✓ Transaction found");
            tx
        },
        None => {
            println!("❌ Transaction not found");
            anyhow::bail!("Transaction not found");
        }
    };
    
    // 5. Extract important data from the transaction
    println!("\nStep 5: Extracting transaction data...");
    
    // Transaction signature
    let tx_signature = tx.signature.clone().unwrap_or_else(|| signature.to_string());
    println!("- Signature: {}", tx_signature);
    
    // Extract logs
    let logs = tx.logs.clone().unwrap_or_default();
    println!("- Found {} log messages", logs.len());
    for (i, log) in logs.iter().take(3).enumerate() {
        println!("  Log[{}]: {}", i, log);
    }
    
    // Check for initialize2 instruction
    let has_initialize2 = logs.iter().any(|log| log.contains("initialize2"));
    println!("- Contains 'initialize2': {}", has_initialize2);
    
    // 6. Manually insert into database for testing
    println!("\nStep 6: Manually inserting to database...");
    
    // In a real test, we would extract these from the transaction
    // For this simplified test, we'll use placeholder values
    let test_pool_address = "TestPoolAddress123";
    let test_token_mint = "TestTokenMint456";
    
    // Create JSON representation
    let tx_json = serde_json::json!({
        "signature": tx_signature,
        "pool_address": test_pool_address,
        "mint_address": test_token_mint,
        "logs": logs,
        "timestamp": chrono::Utc::now().timestamp()
    });
    
    // Insert into database
    println!("- Inserting test record into database");
    db.insert_pool(&tx_signature, test_pool_address, test_token_mint, &tx_json).await?;
    println!("✓ Record inserted successfully");
    
    // 7. Verify the database insertion
    println!("\nStep 7: Verifying database insertion...");
    let results = sqlx::query!(
        "SELECT signature, pool_address, token_mint FROM liquidity_pools_v4 WHERE signature = $1",
        tx_signature
    )
    .fetch_one(&db.pool)
    .await?;
    
    println!("- Found record:");
    println!("  Signature: {}", results.signature);
    println!("  Pool address: {}", results.pool_address);
    println!("  Token mint: {}", results.token_mint);
    
    assert_eq!(results.signature, tx_signature, "Signature mismatch");
    assert_eq!(results.pool_address, test_pool_address, "Pool address mismatch");
    assert_eq!(results.token_mint, test_token_mint, "Token mint mismatch");
    
    println!("✓ Database verification successful");
    
    // 8. Clean up
    println!("\nStep 8: Cleaning up...");
    cleanup_test_database(&db.pool).await?;
    println!("✓ Test database cleaned");
    
    println!("\n===== TEST COMPLETED SUCCESSFULLY =====");
    
    Ok(())
}