#!/bin/bash
# scripts/run_tests.sh

# Load test environment variables
echo "Loading environment variables from .env.test..."
export $(grep -v '^#' .env.test | xargs)

# Set DATABASE_URL specifically for SQLX
export DATABASE_URL=postgres://mluser:mlpassword@localhost:5432/test_db

# Print environment variables for debugging
echo "=== Test Environment Variables ==="
echo "DATABASE_URL: $DATABASE_URL"
echo "DATABASE__URL: $DATABASE__URL" 
echo "SHYFT__ENDPOINT: $SHYFT__ENDPOINT"
echo "=================================="

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


# # Load test environment variables
# echo "Loading environment variables from .env.test..."
# export $(grep -v '^#' .env.test | xargs)



# # Initialize the test database schema
# echo "Initializing test database schema..."
# psql $DATABASE_URL -c "
# CREATE TABLE IF NOT EXISTS liquidity_pools_v4 (
#     id SERIAL PRIMARY KEY,
#     signature TEXT NOT NULL,
#     pool_address TEXT NOT NULL,
#     token_mint TEXT NOT NULL,
#     raw_transaction JSONB NOT NULL,
#     is_active BOOLEAN NOT NULL DEFAULT TRUE,
#     death_reason TEXT DEFAULT NULL,
#     created_at TIMESTAMPTZ NOT NULL DEFAULT now()
# );"

# # Run the unit tests
# echo "Running unit tests..."
# cargo test

# # Run the integration tests
# echo "Running integration tests..."
# cargo test --test integration_tests --features testing

# # # Run the manual test
# # echo "Running manual Raydium test..."
# # cargo run -- test-raydium