#!/bin/bash

# Common functions for OKX API calls

# Check required environment variables
check_env() {
    if [[ -z "${OKEX_API_KEY:-}" || -z "${OKEX_SECRET_KEY:-}" || -z "${OKEX_PASSPHRASE:-}" ]]; then
        echo '{"error": "Missing OKEX_API_KEY, OKEX_SECRET_KEY, or OKEX_PASSPHRASE environment variables"}'
        exit 1
    fi
}

# Make OKX API call
okx_api_call() {
    local method="$1"
    local path="$2"
    local body="${3:-}"

    local timestamp signature pre_hash

    timestamp=$(python3 -c "from datetime import datetime; print(datetime.utcnow().strftime('%Y-%m-%dT%H:%M:%S.%f')[:-3] + 'Z')")
    pre_hash="${timestamp}${method}${path}${body}"
    signature=$(echo -n "$pre_hash" | openssl dgst -sha256 -hmac "$OKEX_SECRET_KEY" -binary | base64)

    curl -s -X "$method" "https://www.okx.com${path}" \
        -H "OK-ACCESS-KEY: $OKEX_API_KEY" \
        -H "OK-ACCESS-SIGN: $signature" \
        -H "OK-ACCESS-TIMESTAMP: $timestamp" \
        -H "OK-ACCESS-PASSPHRASE: $OKEX_PASSPHRASE" \
        -H "Content-Type: application/json" \
        -H "x-simulated-trading: 1" \
        ${body:+-d "$body"}
}
