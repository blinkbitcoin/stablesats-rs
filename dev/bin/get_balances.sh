#!/bin/bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common_okex.sh"

check_env

okx_api_call "GET" "/api/v5/account/balance" | jq '{
    balances: [.data[0].details[] | select((.availBal | tonumber) > 0) | {
        currency: .ccy,
        available: .availBal,
        total: .eq,
        usd_value: .eqUsd
    }],
    total_portfolio_usd: .data[0].totalEq
}'
