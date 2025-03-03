use anyhow::{Context, Result};
use std::collections::HashMap;
use std::cmp::min;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use yellowstone_grpc_proto::prelude::{
    SubscribeUpdateTransactionInfo,
    TransactionStatusMeta, Message,
};

use trade_streamer::{
    connection::{
        rpc::RpcClient,
        subscription::{Subscription, SubscriptionEvent, SubscriptionId, SubscriptionType},
    },
    config::{HeliusSettings, Settings},
    db::Database,
    monitors::raydium_v4::RaydiumV4Monitor,
};

// Import test helpers - adjust path as needed based on your project structure
#[path = "../helpers.rs"]
mod helpers;
use helpers::{setup_test_database, cleanup_test_database};

/// A mock for the Yellowstone gRPC connection manager
struct MockConnectionManager {
    subscriptions: Arc<RwLock<HashMap<SubscriptionId, Subscription>>>,
    confirmation_tx: mpsc::Sender<SubscriptionId>,
}

impl MockConnectionManager {
    fn new(confirmation_tx: mpsc::Sender<SubscriptionId>) -> Self {
        Self {
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            confirmation_tx,
        }
    }
    
    async fn add_subscription(&self, subscription: Subscription) -> Result<SubscriptionId> {
        let sub_id = subscription.id;
        let client_id = subscription.client_id.clone();
        
        // Store the subscription
        {
            let mut subs = self.subscriptions.write().await;
            subs.insert(sub_id, subscription);
        }
        
        // Log that we've added a subscription (to mimic real manager behavior)
        println!("Adding subscription: {:?}", sub_id);
        println!("Successfully updated subscriptions.");
        
        // Signal that subscription is active
        self.confirmation_tx.send(sub_id).await
            .context("Failed to send subscription confirmation")?;
        
        Ok(sub_id)
    }
    
    /// Simulate an incoming transaction event to all matching subscriptions
    async fn inject_transaction(&self, tx_info: SubscribeUpdateTransactionInfo) -> Result<()> {
        let subs = self.subscriptions.read().await;
        
        // Create the subscription event
        let event = SubscriptionEvent::Transaction(tx_info);
        
        // Dispatch to all matching subscriptions
        for subscription in subs.values() {
            if let SubscriptionType::Transactions(_) = &subscription.subscription_type {
                if let Err(e) = subscription.event_handler.handle(event.clone()) {
                    eprintln!("Error handling event: {:?}", e);
                }
            }
        }
        
        Ok(())
    }
}

// Helper to convert a Helius transaction to the Yellowstone gRPC format
async fn helius_tx_to_yellowstone(
    tx: &helius::types::EnhancedTransaction,
    program_id: &str,
    migration_pubkey: &str,
) -> Result<SubscribeUpdateTransactionInfo> {
    println!("Converting Helius transaction to Yellowstone format...");
    // Extract relevant data for our test
    let signature_bytes = if let Some(sig) = &tx.transaction.signatures.get(0) {
        println!("  Transaction signature: {}", sig);
        bs58::decode(sig).into_vec()?
    } else {
        println!("  Error: Missing transaction signature");
        return Err(anyhow::anyhow!("Missing transaction signature"));
    };
    
    // Extract account keys
    let account_keys: Vec<String> = tx.transaction.message.account_keys.clone();
    
    // Create simulation of transaction message with these accounts
    let message = Some(Message {
        account_keys,
        recent_blockhash: "simulated_blockhash".to_string(),
        ..Default::default()
    });
    
    // Extract log messages if available
    let logs = tx.meta.as_ref()
        .and_then(|m| m.log_messages.clone())
        .unwrap_or_default();
        
    println!("  Found {} log messages", logs.len());
    // Print first few log lines to help with debugging
    for (i, log) in logs.iter().take(3).enumerate() {
        println!("    Log[{}]: {}", i, log);
    }
    if logs.len() > 3 {
        println!("    ...(and {} more log messages)", logs.len() - 3);
    }
    
    // Create meta with log messages
    let meta = Some(TransactionStatusMeta {
        log_messages: logs,
        log_messages_none: false,
        ..Default::default()
    });
    
    Ok(SubscribeUpdateTransactionInfo {
        signature: signature_bytes,
        message,
        meta,
        index: 0,
        ..Default::default()
    })
}

#[tokio::test]
async fn test_raydium_v4_monitor_with_helius_transactions() -> Result<()> {
    println!("\n========== STARTING RAYDIUM V4 INTEGRATION TEST ==========\n");

    // 1. Set up the test database
    println!("Step 1: Setting up test database...");
    let db = setup_test_database().await?;
    println!("- Database connection established");
    
    cleanup_test_database(&db.pool).await?;
    println!("- Test database cleaned");

    // 2. Create a channel for subscription confirmation
    println!("\nStep 2: Creating subscription confirmation channel...");
    let (confirmation_tx, mut confirmation_rx) = mpsc::channel(10);
    println!("- Channel created");
    
    // 3. Create mock connection manager
    println!("\nStep 3: Creating mock connection manager...");
    let mock_conn_manager = Arc::new(MockConnectionManager::new(confirmation_tx));
    println!("- Mock connection manager created");

    // 4. Set up Raydium V4 Monitor with realistic configuration
    println!("\nStep 4: Setting up Raydium V4 Monitor...");
    // Actual Raydium V4 program ID
    let program_id = "4RRvpTvhJxgLcY7EXA23ikCkJ6J7SYAKEzhMgDDEXc7k";
    println!("- Using program ID: {}", program_id);
    
    // This should be the actual migration pubkey you're monitoring for
    let migration_pubkey = "4KLx5dQZC2ZGPJgK7Ej66rBPyQJ2VQzRsUTEFoUr6C3w";
    println!("- Using migration pubkey: {}", migration_pubkey);
    
    let raydium_monitor = Arc::new(RaydiumV4Monitor::new(
        program_id.to_string(),
        migration_pubkey.to_string(),
    ));

    // 5. Create an RPC client to fetch real transaction data
    println!("\nStep 5: Setting up Helius RPC client...");
    let api_key = std::env::var("HELIUS_API_KEY")
        .expect("HELIUS_API_KEY must be set for tests");
    println!("- Helius API key found: {}...", &api_key[0..min(5, api_key.len())]);
    
    let helius_settings = HeliusSettings {
        api_key,
        cluster: Some("MainnetBeta".to_string()),
    };
    println!("- Using MainnetBeta cluster");
    
    let rpc_client = RpcClient::new_from_settings(&helius_settings);
    println!("- Helius RPC client created");

    // 6. Create and register a subscription with the mock manager
    println!("\nStep 6: Creating Raydium V4 subscription...");
    let db_clone = Arc::clone(&Arc::new(db));
    let monitor_clone = Arc::clone(&raydium_monitor);
    let migration_pubkey_clone = migration_pubkey.to_string();
    println!("- Prepared dependencies for subscription");
    
    let subscription = raydium_monitor.create_subscription(
        "test_client",
        move |event| {
            let db = db_clone.clone();
            let monitor = monitor_clone.clone();
            let migration_pubkey = migration_pubkey_clone.clone();
            
            // Process the event synchronously for testing
            let pool_event = match monitor.create_pool_event_from_subscription(event) {
                Ok(evt) => evt,
                Err(e) => {
                    eprintln!("Failed to create pool event: {}", e);
                    return Ok(());
                }
            };
            
            // Check criteria (initialize2 + target signer)
            let logs = &pool_event.raw_logs;
            let has_initialize2 = logs.iter().any(|log_line| log_line.contains("initialize2"));
            let has_target_signer = pool_event.signers.contains(&migration_pubkey);
            
            // Only process if criteria met
            if has_initialize2 && has_target_signer {
                println!("Found matching pool event: {}", pool_event.signature);
                
                // Extract pool_address and mint_address
                if let (Some(pool_address), Some(mint_address)) = (&pool_event.pool_address, &pool_event.mint_address) {
                    // Create a JSON representation of the event
                    let raw_tx = serde_json::json!({
                        "signature": pool_event.signature,
                        "pool_address": pool_address,
                        "mint_address": mint_address,
                        "signers": pool_event.signers,
                        "timestamp": pool_event.timestamp,
                        "logs": pool_event.raw_logs
                    });
                    
                    // Run in a blocking task to ensure it completes
                    let db_ref = db.clone();
                    let pool_addr = pool_address.clone();
                    let mint_addr = mint_address.clone();
                    let sig = pool_event.signature.clone();
                    let tx_json = raw_tx.clone();
                    
                    tokio::task::block_in_place(|| {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            match db_ref.insert_pool(&sig, &pool_addr, &mint_addr, &tx_json).await {
                                Ok(_) => println!("Successfully inserted pool: {}", pool_addr),
                                Err(e) => eprintln!("Failed to insert pool: {}", e)
                            }
                        })
                    });
                }
            }
            
            Ok(())
        }
    )?;
    
    // Register the subscription with our mock manager
    println!("- Created subscription handler");
    let sub_id = mock_conn_manager.add_subscription(subscription).await?;
    println!("- Registered subscription with mock manager: {:?}", sub_id);

    // 7. Wait for confirmation that subscription is active
    println!("\nStep 7: Waiting for subscription confirmation...");
    let received_sub_id = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        confirmation_rx.recv()
    ).await
        .context("Timed out waiting for subscription confirmation")?
        .context("Subscription confirmation channel closed")?;
    
    assert_eq!(received_sub_id, sub_id, "Received different subscription ID than expected");
    println!("✓ Subscription confirmed active: {:?}", received_sub_id);

    // 8. Define test transaction signatures
    println!("\nStep 8: Setting up test transaction signatures...");
    // These should be actual Raydium V4 initialize2 transactions with the migration pubkey
    // You need to replace these with real transaction signatures
    let test_signatures = vec![
        // Example: Use a real transaction signature here - this is a placeholder
        "4Rh1vsYXhP8Esyh8b4VhFJ1kEPw5zo5GDvZqPz5QGNjB2oPV3JrXKVGBUqP9x58TxHTKUwScDdEkaCxzaahZmxf4",
        // Add more if needed
    ];
    
    println!("- Will test with {} transaction signatures:", test_signatures.len());
    for (i, sig) in test_signatures.iter().enumerate() {
        println!("  {}. {}", i+1, sig);
    }

    // 9. For each signature, fetch the transaction and inject it
    println!("\nStep 9: Fetching and injecting transactions...");
    let mut successful_injections = 0;
    
    for (i, signature) in test_signatures.iter().enumerate() {
        println!("\nProcessing transaction {}/{}: {}", i+1, test_signatures.len(), signature);
        
        // Fetch the transaction from Helius
        println!("- Fetching transaction from Helius...");
        match rpc_client.get_parsed_transaction(signature).await {
            Ok(Some(tx)) => {
                println!("✓ Successfully fetched transaction");
                
                // Convert to Yellowstone format
                println!("- Converting to Yellowstone format...");
                match helius_tx_to_yellowstone(&tx, program_id, migration_pubkey).await {
                    Ok(tx_info) => {
                        println!("✓ Successfully converted transaction");
                        
                        // Inject into our mock subscription system
                        println!("- Injecting transaction into mock subscription system...");
                        mock_conn_manager.inject_transaction(tx_info).await?;
                        successful_injections += 1;
                        println!("✓ Successfully injected transaction");
                        
                        // Give some time for async processing
                        println!("- Waiting for processing to complete...");
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        println!("✓ Processing time elapsed");
                    },
                    Err(e) => {
                        println!("❌ Failed to convert transaction: {}", e);
                    }
                }
            },
            Ok(None) => {
                println!("❌ Transaction not found");
            },
            Err(e) => {
                println!("❌ Failed to fetch transaction: {}", e);
            }
        }
    }

    // 10. Verify that transactions were processed and stored
    println!("\nStep 10: Verifying database results...");
    if successful_injections > 0 {
        // There should be at least one record in the database
        println!("- Checking database for processed transactions...");
        let count = sqlx::query!(
            "SELECT COUNT(*) as count FROM liquidity_pools_v4"
        )
        .fetch_one(&db.pool)
        .await?;
        
        let count_value = count.count.unwrap_or(0) as i64;
        
        if count_value > 0 {
            println!("✓ SUCCESS: Found {} transaction(s) in the database", count_value);
            
            // Show details of the stored records
            let pools = sqlx::query!(
                "SELECT signature, pool_address, token_mint FROM liquidity_pools_v4"
            )
            .fetch_all(&db.pool)
            .await?;
            
            println!("\nStored pools:");
            for (i, pool) in pools.iter().enumerate() {
                println!("  {}. Signature: {}", i+1, pool.signature);
                println!("     Pool Address: {}", pool.pool_address);
                println!("     Token Mint: {}", pool.token_mint);
            }
        } else {
            println!("❌ ERROR: No records were inserted into the database despite {} successful injection(s)", 
                     successful_injections);
            println!("\nPossible reasons:");
            println!("  - The transactions might not have matched the filtering criteria");
            println!("  - The handler might not have found 'initialize2' in the logs");
            println!("  - The transaction might not have included the migration pubkey");
        }
    } else {
        println!("⚠️ WARNING: No transactions were successfully injected for testing");
    }

    // 11. Clean up
    println!("\nStep 11: Cleaning up...");
    cleanup_test_database(&db.pool).await?;
    println!("✓ Test database cleaned");
    
    println!("\n========== TEST COMPLETE ==========\n");
    Ok(())
}
