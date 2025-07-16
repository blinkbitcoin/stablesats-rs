# About `stablesats`
Stablesats is a part of the blink OSS banking stack.
It enables users that deposit Bitcoin to hold a USD denominated value in their wallets.
It achieves this by identifying transactions that involve a hard-coded `dealer` ledger account in the blink ledger and calculating a target liability.
This liability is subsequently hedged via shorting perpetual swap contracts on the okex exchange.

## Design

The code is organized into multiple crates.
Some of the crates represent heplers or client libraries for the APIs we depend on and some of them represent logical units that can be run either in isolated processes or together with other units within the same process depending on config settings.

Communication between the (potentially distributed) processes happens via a pubsub system (currently Redis).
Like this we can run multiple copies of the processes to achieve high-availability, fault tolerance and scalability.

The main modules that can be run via the cli are:
- `okex-price`: Module that streams price information from okex onto the pubsub
- `price-server`: Module that exposes a grpc endpoint for clients to get up-to-date price information (cached from the pubsub messages coming from `okex-price`).
- `user_trades`: Module that identifies how much the total usd liability exists in the blink accounting ledger. It publishes the `SynthUsdLiabilityPayload` message for downstream trading modules to pick up.
- `hedging`: Module that executes trades on okex to match the target liability received from the pubsub.

## Dependencies in stablesats-rs

### 1. **blink-api (galoy-client)** - Banking backend integration  
**Purpose**: Connects to the Galoy banking backend (which powers Blink wallet) to monitor user transactions of the dealer-account and calculates the target liability balances.

**How transaction polling works**:
- **GraphQL endpoint**: Uses the `StablesatsTransactionsList` query against Blink's GraphQL API of the dealer account
- **Cursor-based pagination**: Uses `before` cursor parameter to fetch transactions in reverse chronological order (newest first)
- **Batch size**: Fetches 100 transactions per request
- **Continuous polling**: The `poll_galoy_transactions` job runs periodically to import new transactions
- **Unpaired transaction re-checking**: Separately polls for older unpaired transactions that may have been missed

**Transaction identification**:
- **Dealer account detection**: Identifies transactions involving the hardcoded "dealer" ledger account
- **USD conversion tracking**: Monitors when users deposit Bitcoin but want to hold USD value
- **Settlement currency**: Tracks both BTC and USD settlement amounts with exchange rates

### 2. **okx (okex-client)** - Derivatives exchange for hedging
**Purpose**: Connects to OKX exchange to execute the core hedging strategy through Bitcoin perpetual swap contracts.

**Role in hedging**:
- **Perpetual swaps**: Shorts BTC-USD-SWAP contracts to hedge Bitcoin price exposure
- **Position management**: Maintains short positions equivalent to user USD liability
- **Account balancing**: Manages transfers between funding and trading accounts
- **Price data**: Fetches real-time BTC prices for hedging calculations

### 3. **bria** - Bitcoin custody and on-chain operations
**Purpose**: Handles Bitcoin on-chain transactions and custody operations for the stablesats system.

**How it works for stablesats**: 
- **Funding adjustments**: Stablesats maintains Bitcoin in two locations - its own custody (managed by Bria) and on the OKX exchange for trading. When the OKX trading account runs low on Bitcoin capital needed for hedging operations, stablesats uses Bria to send Bitcoin on-chain from its custody to OKX deposit addresses. Conversely, when excess Bitcoin accumulates on OKX, it can be withdrawn back to stablesats custody via Bria to optimize capital allocation
- **Withdrawal operations**: Uses Bria to withdraw Bitcoin from stablesats custody to OKX exchange when more trading capital is needed

### How they work together
1. **User deposits Bitcoin** → Galoy-client detects transaction involving dealer account
2. **Calculate hedge requirement** → System determines how much USD exposure needs hedging  
3. **Execute hedge** → OKX-client shorts equivalent BTC perpetual swaps
4. **Manage funding** → Bria-client transfers Bitcoin between custody and OKX as needed
5. **Continuous rebalancing** → All three clients work together to maintain proper hedge ratios

This creates a complete system where users can hold stable USD value while the system manages Bitcoin price volatility through derivatives hedging and proper capital management.

## How to run `stablesats`
The stablesats command line interface (CLI) is an application that allows users to get price quotes, and runs configured processes.
To view the CLI commands and options, run
```
$ stablesats
```

To run the configured processes:
- Make a copy of the [stablesats](stablesats.yml) configuration file and rename the file. Ensure that this new configuration is not committed (add to global `.gitignore`) if contributing to the project.
- Uncomment the file and update the `galoy.api` and `galoy.phone_number` config values with values contained [here](https://github.com/GaloyMoney/galoy/blob/main/src/graphql/docs/README.md). Change the `okex.simulated` value to `true`.
- Run the CLI
```
$ stablesats -c $NEW_CONFIGURATION_FILE run
```
- For help on the `run` command
```
$ stablesats run --help
```

To get price quotes:
- Open a new terminal
- Request a quote for given price
```
$ stablesats price 10000
```
- For help on the `price` command

```
$ stablesats price --help
```
