# OKX Trading Scripts

Simple shell scripts for interacting with the OKX API in sandbox mode.

## Purpose

These scripts are used to rebalance different token balances in the OKX sandbox account for testing purposes. The tests require sufficient BTC in the OKX sandbox account to function properly.

## Prerequisites

Set the following environment variables:
```bash
export OKEX_API_KEY="your_sandbox_api_key"
export OKEX_SECRET_KEY="your_sandbox_secret_key"
export OKEX_PASSPHRASE="your_sandbox_passphrase"
```

## Scripts

### `get_balances.sh`
Get current account balances.
```bash
./get_balances.sh
```

### `sell_okb.sh`
Sell OKB for BTC using market order.
```bash
./sell_okb.sh <amount>
# Example: ./sell_okb.sh 10
```

### `sell_usdt.sh`
Buy BTC with USDT using market order.
```bash
./sell_usdt.sh [amount]
# Example: ./sell_usdt.sh 100
# Default: 50 USDT if no amount specified
```

## Notes

- All scripts use **sandbox mode** (`x-simulated-trading: 1`)
- Scripts output JSON responses
- Requires `curl`, `jq`, `python3`, and `openssl`
- Used for rebalancing test account before running integration tests
