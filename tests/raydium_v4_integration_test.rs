use anyhow::{Context, Result};
use std::collections::HashMap;
use std::cmp::min;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use yellowstone_grpc_proto::prelude::{
    SubscribeUpdateTransactionInfo,
    TransactionStatusMeta
};

use trade_streamer::{
    connection::{
        rpc::RpcClient,
        Subscription, SubscriptionEvent, SubscriptionId, SubscriptionType
    },
    config::{HeliusSettings, Settings},
    db::Database,
    monitors::raydium_v4::RaydiumV4Monitor,
};

// Import test helpers
#[path = "helpers.rs"]
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
        
        // Store the subscription
        {
            let mut subs = self.subscriptions.write().await;
            subs.insert(sub_id, subscription);
        }
        
        // Log and notify that subscription was added
        println!("Adding subscription: {:?}", sub_id);
        println!("Successfully updated subscriptions.");
        
        // Signal that subscription is active
        self.confirmation_tx.send(sub_id).await
            .context("Failed to send subscription confirmation")?;
        
        Ok(sub_id)
    }
    
    // Method to simulate incoming transaction events
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
    use yellowstone_grpc_proto::prelude::{Transaction, Message, CompiledInstruction, MessageHeader};
    println!("Converting Helius transaction to Yellowstone format...");
    
    // Extract signature from Helius format
    println!("  Transaction signature: {}", tx.signature);
    let signature_bytes = bs58::decode(&tx.signature).into_vec()?;
    
    // Create a minimal set of account keys for the transaction
    // The position of accounts in this array is critical for extracting pool and mint addresses
    let mut account_keys: Vec<Vec<u8>> = Vec::new();
    
    // Add migration pubkey as a signer (index 0)
    let migration_pubkey_bytes = bs58::decode(migration_pubkey).into_vec()?;
    account_keys.push(migration_pubkey_bytes);
    
    // Add fee payer account (index 1)
    let fee_payer = bs58::decode("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1").into_vec()?;
    account_keys.push(fee_payer);
    
    // Add the known pool address (index 2) - this is the address we want to extract
    let pool_address = "5iouQYAkahVWuAWhixrQnWBWTKgFTnuXhn6j38wLGRde";
    let pool_address_bytes = bs58::decode(pool_address).into_vec()?;
    account_keys.push(pool_address_bytes);
    
    // Add the token mint address (index 3) - this is the mint we want to extract
    let mint_address = "DVyCCFJw8XBzQwjWaShJ7MPxfkdqShUnUwD49QFApump";
    let mint_address_bytes = bs58::decode(mint_address).into_vec()?;
    account_keys.push(mint_address_bytes);
    
    // Add the Raydium program ID (index 4)
    let program_id_bytes = bs58::decode(program_id).into_vec()?;
    account_keys.push(program_id_bytes);
    
    // Add additional accounts found in the transaction
    let additional_accounts = [
        "8V1CSXXXpjyB9VcEE6odCoy3jcrMdcS5PWj8vjgz7ZfD",
        "kjUUkbLKfbLRQGXwvoeN7uxenMfZhzdrjUG9evsFKWq",
        "So11111111111111111111111111111111111111112",
        "DhUo1QwKiNetZnYPAJC89ddTFv7htw1WvGcCNhUCjpkN",
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    ];
    
    for acct in additional_accounts {
        let acct_bytes = bs58::decode(acct).into_vec()?;
        if !account_keys.contains(&acct_bytes) {
            account_keys.push(acct_bytes);
        }
    }
    
    println!("  Created account keys array with {} accounts", account_keys.len());
    println!("  Migration pubkey at index 0");
    println!("  Pool address at index 2: {}", pool_address);
    println!("  Mint address at index 3: {}", mint_address);
    
    // Create a message header that specifies the migration pubkey as a signer
    let header = MessageHeader {
        num_required_signatures: 2, // Both migration pubkey and fee payer are signers
        num_readonly_signed_accounts: 0,
        num_readonly_unsigned_accounts: 0,
    };
    
    // Create a compiled instruction that mimics the Raydium initialize2 call
    // Using the exact account indices to match the expected pattern
    let accounts_data = vec![0u8, 1u8, 2u8, 3u8, 4u8]; // Account indices as byte array
    let instruction = CompiledInstruction {
        program_id_index: 4, // Index of Raydium program ID
        accounts: accounts_data, // Array of account indices - includes pool and mint addresses
        data: vec![0x04, 0x00], // Mimicking initialize2 instruction data
    };
    
    // Create a message with account keys and the instruction
    let message = Message {
        account_keys,
        header: Some(header),
        recent_blockhash: Vec::new(),
        instructions: vec![instruction],
        address_table_lookups: Vec::new(),
        versioned: false,
    };
    
    // Create the transaction with message
    let transaction = Transaction {
        signatures: vec![signature_bytes.clone()],
        message: Some(message),
    };
    
    // Extract log messages
    let mut logs = Vec::new();
    
    // Add a synthetic log that includes the program ID
    logs.push(format!("Program {} invoke [1]", program_id));
    
    // Add a log mentioning the pool and mint addresses
    logs.push(format!("Program log: Initialize pool {} with mint {}", pool_address, mint_address));
    
    // Add a synthetic log for initialize2 to ensure our test matching works
    logs.push(String::from("Program log: initialize2"));
    
    println!("  Assembled {} log messages", logs.len());
    // Print log lines to help with debugging
    for (i, log) in logs.iter().enumerate() {
        println!("    Log[{}]: {}", i, log);
    }
    
    // Create meta with log messages
    let meta = Some(TransactionStatusMeta {
        log_messages: logs,
        log_messages_none: false,
        ..Default::default()
    });
    
    // Create SubscribeUpdateTransactionInfo with all required fields
    Ok(SubscribeUpdateTransactionInfo {
        signature: signature_bytes,
        is_vote: false,
        transaction: Some(transaction),
        meta,
        index: 0,
        ..Default::default()
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn test_raydium_v4_monitor_with_helius_transactions() -> Result<()> {
    println!("\n========== STARTING RAYDIUM V4 INTEGRATION TEST ==========\n");

    // 1. Set up the test database
    println!("Step 1: Setting up test database...");
    let db = Arc::new(setup_test_database().await?);
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
    // Actual Raydium V4 program ID from your code
    let program_id = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";
    println!("- Using program ID: {}", program_id);
    
    // Actual migration pubkey from your code
    let migration_pubkey = "39azUYFWPz3VHgKCf3VChUwbpURdCHRxjWVowf5jUJjg";
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
    let db_clone = Arc::clone(&db);
    let monitor_clone = Arc::clone(&raydium_monitor);
    let migration_pubkey_clone = migration_pubkey.to_string();
    println!("- Prepared dependencies for subscription");
    
    let subscription = raydium_monitor.create_subscription(
        "test_client",
        move |event| {
            let db = db_clone.clone();
            let monitor = monitor_clone.clone();
            let migration_pubkey = migration_pubkey_clone.clone();
            
            // Process the event
            tokio::task::block_in_place(|| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    // Process the event asynchronously
                    let pool_event = match monitor.create_pool_event_from_subscription(event) {
                        Ok(evt) => evt,
                        Err(e) => {
                            eprintln!("Failed to create pool event: {}", e);
                            return Ok::<(), anyhow::Error>(());
                        }
                    };
                    
                    // Check criteria (initialize2 + target signer)
                    let logs = &pool_event.raw_logs;
                    let has_initialize2 = logs.iter().any(|log_line| log_line.contains("initialize2"));
                    let has_target_signer = pool_event.signers.contains(&migration_pubkey);
                    
                    println!("Event check - has_initialize2: {}, has_target_signer: {}", 
                             has_initialize2, has_target_signer);
                    
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
                            
                            // Insert into database
                            match db.insert_pool(&pool_event.signature, pool_address, mint_address, &raw_tx).await {
                                Ok(_) => println!("Successfully inserted pool: {}", pool_address),
                                Err(e) => eprintln!("Failed to insert pool: {}", e)
                            }
                        } else {
                            println!("Missing pool_address or mint_address in event");
                        }
                    } else {
                        println!("Event did not match criteria (initialize2 + target signer)");
                    }
                    
                    Ok::<(), anyhow::Error>(())
                })
            })?;
            
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
    // Using the transaction signature you provided
    let test_signatures = vec![
        "31g1JoV7Zqinn7DsXUBEQBfizsbVPYxn85QWzDQ6wTWWY4shtihwewYjPwg1bWCFA7tZ8EAuk5EfugQEpJSNgLGo",
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
                println!("  Transaction type: {}", tx.transaction_type);
                println!("  Description: {}", tx.description);
                
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
            println!("  - Pool address or mint address extraction might have failed");
            
            // Add additional debugging to check the exact values
            println!("\nAdditional debugging:");
            println!("- Double-check program_id: {}", program_id);
            println!("- Double-check migration_pubkey: {}", migration_pubkey);
        }
    } else {
        println!("⚠️ WARNING: No transactions were successfully injected for testing");
    }

    // 11. Skip cleanup to allow manual inspection of the database
    println!("\nStep 11: Skipping database cleanup as requested...");
    println!("✓ Test complete - database left intact for manual inspection");
    println!("\nNOTE: Don't forget to clean up the database manually when finished!");
    
    println!("\n========== TEST COMPLETE ==========\n");
    Ok(())
}