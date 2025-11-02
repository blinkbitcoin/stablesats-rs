#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUTPUT_FILE="${REPO_ROOT}/.api-key.env"

# Set HOST_PROJECT_PATH so helpers.sh can find the correct GALOY_DIR
export HOST_PROJECT_PATH="${REPO_ROOT}"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-blink-quickstart}"

echo "🔐 Setting up Dealer API Key for integration tests"
echo ""

# Run the vendor script and capture output
if ! output=$("${REPO_ROOT}/vendor/blink-quickstart/bin/create-dealer-api-key.sh" 2>&1); then
  echo "❌ Failed to run create-dealer-api-key.sh"
  echo "Error output:"
  echo "$output"
  exit 1
fi

# Display the output
echo "$output"

# Extract the API key from the output (it's in the line "Secret: <key>")
echo "🔍 Extracting API key from output..."
api_key=$(echo "$output" | grep "^Secret: " | sed 's/^Secret: //' | tr -d '\r\n' | xargs || echo "")

if [ -z "$api_key" ]; then
  echo "❌ Failed to extract API key from script output"
  echo ""
  echo "Debug: Looking for line starting with 'Secret:'"
  echo "Lines containing 'Secret':"
  echo "$output" | grep -i "secret" || echo "  (none found)"
  echo ""
  echo "Full output:"
  echo "$output"
  exit 1
fi

# Export to current shell environment
export GALOY_API_KEY="${api_key}"

# Write to file for persistence across sessions
echo "export GALOY_API_KEY=\"${api_key}\"" > "${OUTPUT_FILE}"

echo ""
echo "✅ API key set in environment: GALOY_API_KEY=${api_key}"
echo "✅ API key saved to ${OUTPUT_FILE}"
