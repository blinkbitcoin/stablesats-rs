#!/bin/bash

if [[ $# -ne 1 ]]; then
    echo '{"error": "Usage: ./sell_okb.sh <okb_amount>"}'
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common_okex.sh"

check_env

okb_amount="$1"

payload='{"instId":"OKB-BTC","tdMode":"cash","side":"sell","ordType":"market","sz":"'$okb_amount'"}'

echo "Selling $okb_amount OKB for BTC..."

okx_api_call "POST" "/api/v5/trade/order" "$payload" | jq '.'
