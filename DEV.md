# `stablesats`
- [Dependencies](#dependencies)
    - [Tools](#tools)
- [Getting started](#getting-started)
    - [Local Development Mode](#local-development-mode)
- [How to run stablesats](#how-to-run-stablesats)
- [Testing](#testing)
- [Database Configuration](#database-configuration)
- [Check code](#check-code)
- [Contributing](#contributing)

In its current implementation, `stablesats` is coupled to and dependent on the [blink](https://github.com/blinkbitcoin/blink) backend to fetch user transactions on a bitcoin-based banking client, e.g. Bitcoin Beach Wallet. To get it running locally, you have to, among other dependencies, set up a local `blink` backend as well. This document will walk you through the set up.

## Dependencies
Last tested with the following tools and application:
### Tools
- Rust Compiler
```
$ rustc --version
rustc 1.63.0 (4b91a6ea7 2022-08-08)
```
- Cargo
```
$ cargo --version
cargo 1.63.0 (fd9c4297c 2022-07-01)
```
- Docker
```
$ docker --version
Docker version 20.10.18, build b40c2f6
```
- Direnv
```
$ direnv --version
2.32.1
```
- [Blink backend](https://github.com/blinkbitcoin/blink)

## Getting started
### Local Development Mode
1. Clone the [blink](https://github.com/blinkbitcoin/blink) backend and follow the instructions detailed in the documentation. Pay particular attention to the information presented [here](https://github.com/blinkbitcoin/blink/blob/main/src/graphql/docs/README.md) to get local developer access to the graphql API
2. Take note to shutdown the instance of the running stablesats container provisioned alongside blink backend. Get the container ID
```
$ cd /path/to/blink
$ docker compose ps
```
and stop/kill the container
```
$ docker stop $STABLESATS_CONTAINER_ID
```
3. Clone the [stablesats](https://github.com/blinkbitcoin/stablesats-rs) repository and change to its directory
```
cd stablesats
```
4. Load environment variables contained in `.envrc`. Create an [okx]() account and create trading API and secret keys. Populate the appropriate fields with the generated keys and passphrase, after this, export the variables to your environment by running
```
direnv allow
```
5. Take note to update the postgres port numbers of any of `user-trades-db` and `hedging-db` to ensure these databases run alongside the postgres database(s) on `blink`. Make the changes in [docker-compose.override](docker-compose.override.yml), in [user-trades/.env](.user-trades/.env) and/or [hedging/.env](.user-trades/.env) files

6. Run the local containers `stablesats` depends on
```
$ make reset-deps-local
```

Note that some times migrating the databases fails because they are starting up. If you encounter an error of the form:
```
error: error returned from database: the database system is starting up
make: *** [Makefile:41: setup-db] Error 1
```
run the migration command again
```
$ make setup-db
```
7. Build `stablesats`
```
$ make build
```
8. Run `stablesats`: See the section on [how to run](#how-to-run-stablesats) the application

## Testing
To run the integration tests, run the command
```
$ make test-in-ci
```
To run tests for a specific package
```
$ cargo test -p $PACKAGE_NAME
```
Example
```
$ cargo test -p okex-price
```

## Database Configuration

The stablesats project uses different environment variables for database connections depending on the context:

### Migration vs Runtime vs Tests
- **`DATABASE_URL`**: Used by SQLx for running database migrations (`cargo sqlx migrate run`)
- **`PG_CON`**: Used by the main application runtime (passed via CLI)
- **`PG_HOST`/`PG_PORT`**: Used by individual tests to construct connection strings

### Port Configuration
- **CI/Docker environment**: Database runs on port 5432 (container internal)
- **Local development**: Database runs on port 5440 (host port, mapped from container port 5432)

### Environment Variables for Local Testing
When running tests locally, ensure the correct port is used:
```bash
export PG_PORT=5440  # For local development
make test-local
```

The `docker-compose.override.yml` maps the stablesats-pg container port 5432 to host port 5440 to avoid conflicts with other PostgreSQL instances.

## Check code
To pass github actions, check that your code is formatted and linted properly
```
$ make check-code
```
## Contributing
We are open to and encourage contribution from the community. Please ensure you adhere to the following when creating a pull request:
- Have a [clean commit history](https://medium.com/@catalinaturlea/clean-git-history-a-step-by-step-guide-eefc0ad8696d)
- Use [good commit messages](https://tbaggery.com/2008/04/19/a-note-about-git-commit-messages.html)
- Resolve all conflicts
- Rebase often
