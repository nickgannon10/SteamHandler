#!/bin/bash
# scripts/run_raydium_integration_test.sh

set -e  # Exit immediately if a command fails

echo "=== Raydium V4 Integration Test Runner ==="

# Load test environment variables
echo "Step 1: Loading environment variables from .env.test..."
if [ -f .env.test ]; then
  export $(grep -v '^#' .env.test | xargs)
  echo "✓ Environment variables loaded"
else
  echo "❌ Error: .env.test file not found!"
  exit 1
fi

# Set DATABASE_URL specifically for SQLX
export DATABASE_URL=${DATABASE__URL:-postgres://postgres:postgres@localhost:5432/test_db}

# Set HELIUS_API_KEY from HELIUS__API_KEY for test compatibility
export HELIUS_API_KEY=$HELIUS__API_KEY

# Print environment variables for debugging
echo
echo "Step 2: Verifying environment variables..."
echo "DATABASE_URL: $DATABASE_URL"
echo "HELIUS_API_KEY: ${HELIUS_API_KEY:0:5}..." # Only show first few chars for security
echo "✓ Environment variables verified"

# Prepare the test database
echo
echo "Step 3: Preparing test database..."
echo "- Connecting to database at $DATABASE_URL"

if which psql > /dev/null; then
  # If psql is available, create the test table
  psql $DATABASE_URL -c "
  DROP TABLE IF EXISTS liquidity_pools_v4;
  CREATE TABLE IF NOT EXISTS liquidity_pools_v4 (
      id SERIAL PRIMARY KEY,
      signature TEXT NOT NULL,
      pool_address TEXT NOT NULL,
      token_mint TEXT NOT NULL,
      raw_transaction JSONB NOT NULL,
      is_active BOOLEAN NOT NULL DEFAULT TRUE,
      death_reason TEXT DEFAULT NULL,
      created_at TIMESTAMPTZ NOT NULL DEFAULT now()
  );" > /dev/null
  echo "✓ Test database schema prepared"
else
  echo "! psql not found, assuming database is already configured"
fi

# Create the test directory if it doesn't exist
echo
echo "Step 4: Setting up test files..."
mkdir -p tests

# Create the main test file directly
TEST_FILE="tests/raydium_v4_integration_test.rs"
echo "- Creating integration test file at $TEST_FILE"

# Check if test file content is already reasonable
if [ -f "$TEST_FILE" ] && [ $(wc -l < "$TEST_FILE") -gt 100 ]; then
  echo "✓ Test file exists and seems reasonable ($(wc -l < "$TEST_FILE") lines)"
  echo "  Keeping existing file. Delete it manually if you want to regenerate."
else
  # Create the test file - you should copy the entire raydium_v4_integration_test.rs content here
  # between the EOF markers below
  cat > "$TEST_FILE" << 'EOF'
// Paste your test file content here
EOF

  if [ $? -eq 0 ] && [ -f "$TEST_FILE" ]; then
    echo "✓ Test file created successfully"
    # Verify the file has content
    LINE_COUNT=$(wc -l < "$TEST_FILE")
    echo "  File contains $LINE_COUNT lines of code"
    
    if [ "$LINE_COUNT" -lt 10 ]; then
      echo "⚠️  WARNING: Test file seems suspiciously small ($LINE_COUNT lines)"
      echo "   Please check that the test file content was properly written"
    fi
  else
    echo "❌ Failed to create test file"
    exit 1
  fi
fi

# Run the integration test with verbose output
echo
echo "Step 5: Building and running Raydium V4 integration test..."
# Build first to catch compilation errors before running
cargo build --test raydium_v4_integration_test

if [ $? -eq 0 ]; then
  echo "✓ Test compiled successfully, running test now..."
  RUST_BACKTRACE=1 cargo test --test raydium_v4_integration_test -- --nocapture
else
  echo "❌ Test failed to compile"
  exit 1
fi

echo
echo "=== Test run complete ==="

# #!/bin/bash
# # scripts/run_tests.sh

# # Load test environment variables
# echo "Loading environment variables from .env.test..."
# export $(grep -v '^#' .env.test | xargs)

# # Set DATABASE_URL specifically for SQLX
# export DATABASE_URL=postgres://mluser:mlpassword@localhost:5432/test_db

# # Print environment variables for debugging
# echo "=== Test Environment Variables ==="
# echo "DATABASE_URL: $DATABASE_URL"
# echo "DATABASE__URL: $DATABASE__URL" 
# echo "SHYFT__ENDPOINT: $SHYFT__ENDPOINT"
# echo "=================================="

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
