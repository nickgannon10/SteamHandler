#!/bin/bash
# scripts/run_tests.sh

# Load test environment variables
echo "Loading environment variables from .env.test..."
export $(grep -v '^#' .env.test | xargs)

# Print environment variables for debugging
# echo "=== Test Environment Variables ==="
# echo "DATABASE_URL: $DATABASE_URL"
# echo "TEST_DATABASE_URL: $TEST_DATABASE_URL" 
# echo "SHYFT_ENDPOINT: $SHYFT_ENDPOINT"
# echo "=================================="

# Initialize the test database schema
echo "Initializing test database schema..."
psql $DATABASE_URL -c "
CREATE TABLE IF NOT EXISTS liquidity_pools_v4 (
    id SERIAL PRIMARY KEY,
    signature TEXT NOT NULL,
    pool_address TEXT NOT NULL,
    token_mint TEXT NOT NULL,
    raw_transaction JSONB NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    death_reason TEXT DEFAULT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);"

# Run the unit tests
echo "Running unit tests..."
cargo test

# Run the integration tests
echo "Running integration tests..."
cargo test --test integration_tests --features testing

# # Run the manual test
# echo "Running manual Raydium test..."
# cargo run -- test-raydium