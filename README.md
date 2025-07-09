# About `stablesats`
Stablesats is a part of the galoy OSS banking stack.
It enables users that deposit Bitcoin to hold a USD denominated value in their wallets.
It achieves this by identifying transactions that involve a hard-coded `dealer` ledger account in the Galoy ledger and calculating a target liability.
This liability is subsequently hedged via shorting perpetual swap contracts on the okex exchange.

## Design

The code is organized into multiple crates.
Some of the crates represent heplers or client libraries for the APIs we depend on and some of them represent logical units that can be run either in isolated processes or together with other units within the same process depending on config settings.

Communication between the (potentially distributed) processes happens via a pubsub system (currently Redis).
Like this we can run multiple copies of the processes to achieve high-availability, fault tolerance and scalability.

The main modules that can be run via the cli are:
- `okex-price`: Module that streams price information from okex onto the pubsub
- `price-server`: Module that exposes a grpc endpoint for clients to get up-to-date price information (cached from the pubsub messages coming from `okex-price`).
- `user_trades`: Module that identifies how much the total usd liability exists in the galoy accounting ledger. It publishes the `SynthUsdLiabilityPayload` message for downstream trading modules to pick up.
- `hedging`: Module that executes trades on okex to match the target liability received from the pubsub.

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
