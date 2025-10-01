#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common_okex.sh"

check_env

usdt_amount="${1:-50}"

payload='{"instId":"BTC-USDT","tdMode":"cash","side":"buy","ordType":"market","sz":"'$usdt_amount'"}'

echo "Buying BTC with $usdt_amount USDT..."

okx_api_call "POST" "/api/v5/trade/order" "$payload" | jq '.'
