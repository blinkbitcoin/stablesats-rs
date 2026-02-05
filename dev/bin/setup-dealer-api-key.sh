#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
OUTPUT_FILE="${REPO_ROOT}/.api-key.env"

export HOST_PROJECT_PATH="${REPO_ROOT}"
export COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-blink-quickstart}"

source "${REPO_ROOT}/vendor/blink-quickstart/bin/helpers.sh"

echo "🔐 Setting up Dealer API Key for integration tests"
echo ""

# Wait for Oathkeeper to be ready (returns 200 on health endpoint)
MAX_RETRIES=30
RETRY_DELAY=2

echo "⏳ Waiting for Oathkeeper to be ready..."
for i in $(seq 1 $MAX_RETRIES); do
  if curl -sf http://localhost:4456/health/ready > /dev/null 2>&1; then
    echo "✅ Oathkeeper is ready"
    break
  fi
  if [ $i -eq $MAX_RETRIES ]; then
    echo "❌ Oathkeeper failed to become ready after $((MAX_RETRIES * RETRY_DELAY)) seconds"
    exit 1
  fi
  echo "  Attempt $i/$MAX_RETRIES - Oathkeeper not ready yet, waiting ${RETRY_DELAY}s..."
  sleep $RETRY_DELAY
done

# Retry API key creation with backoff
echo "🔑 Creating dealer API key..."
for i in $(seq 1 $MAX_RETRIES); do
  if api_key=$(create_dealer_api_key 2>&1); then
    if [ -n "$api_key" ] && [ "$api_key" != "null" ]; then
      break
    fi
  fi
  if [ $i -eq $MAX_RETRIES ]; then
    echo "❌ Failed to create dealer API key after $MAX_RETRIES attempts"
    exit 1
  fi
  echo "  Attempt $i/$MAX_RETRIES - API key creation failed, waiting ${RETRY_DELAY}s..."
  sleep $RETRY_DELAY
done

if [ -z "$api_key" ] || [ "$api_key" == "null" ]; then
  echo "❌ Failed to extract API key"
  exit 1
fi

echo "✅ Successfully created Dealer API Key: ${api_key}"

export GALOY_API_KEY="${api_key}"

echo "export GALOY_API_KEY=\"${api_key}\"" > "${OUTPUT_FILE}"
