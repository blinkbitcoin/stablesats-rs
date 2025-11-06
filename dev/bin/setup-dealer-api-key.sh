#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUTPUT_FILE="${REPO_ROOT}/.api-key.env"

export HOST_PROJECT_PATH="${REPO_ROOT}"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-blink-quickstart}"

source "${REPO_ROOT}/vendor/blink-quickstart/bin/helpers.sh"

echo "🔐 Setting up Dealer API Key for integration tests"
echo ""

if ! api_key=$(create_dealer_api_key); then
  echo "❌ Failed to create dealer API key"
  exit 1
fi

if [ -z "$api_key" ] || [ "$api_key" == "null" ]; then
  echo "❌ Failed to extract API key"
  exit 1
fi

echo "✅ Successfully created Dealer API Key: ${api_key}"

export GALOY_API_KEY="${api_key}"

echo "export GALOY_API_KEY=\"${api_key}\"" > "${OUTPUT_FILE}"