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

The position-operation and hedging tests use a separate local HTTP exchange
fixture for each test. They exercise the real OKX client request and response
handling without a funded demo account. The `okex-client` `test-support` feature
exposes an explicit fixture-client constructor and reusable HTTP fixture.
Application configuration has no endpoint override field. Release builds use default features; `check-release-features.sh`
rejects a release dependency graph that enables test support. The same rate-limiter
code runs for both clients: production shares a one-request-per-second budget per
endpoint across clients, while each fixture client has a higher quota.

External demo tests in `okex-client/tests/client.rs` are ignored in PR CI; local
preflight regression tests in the same file run normally. The
`OKX demo contract` workflow runs them sequentially each Monday, on main-branch
pushes changing the client or demo workflow, and through manual `workflow_dispatch`.
It uses the repository's demo-account `OKEX_*` secrets. A failed run opens an issue
assigned to the username in the repository variable `OKEX_DEMO_OWNER`, or comments
on the existing open issue labeled `okex-demo-alert`, with a link to the run attempt.
Set this variable to a repository collaborator before enabling the workflow; rotating
it assigns subsequent alerts to the new owner. Empty/invalid owners, unassignable
users, and assignments silently dropped by GitHub fail the alert job. The helper
creates the label if needed; renaming an alert issue preserves deduplication. This
explicitly notifies the owner through GitHub issue notifications; no
credentials or account data are copied into the alert. The owner must keep Actions
and issue notifications enabled and check/re-enable a schedule disabled by GitHub
inactivity policy; push/manual triggers remain available.

A separate `DEMO_ACCOUNT_PREFLIGHT` step checks authentication, account mode, funding
balance (at least 0.00002 BTC), and available trading margin for one contract plus
headroom and the transfer. Alerts distinguish preflight/setup failures from tests
that fail after preflight passes by reading step conclusions from the completed job
for the current run attempt through the Actions API. Failed-job outputs are not used.
Balance checks are prerequisites, not proof that a later API failure is a contract change.
The demo account must be configured for net mode, account level 2, and funded for
its position and transfer tests. The deposit/withdrawal tests additionally require
explicit address/amount environment variables and otherwise skip their bodies;
the scheduled workflow does not supply those variables. PR integration tests do
not receive OKX credentials. To run the demo checks locally:

```sh
cargo test --locked -p okex-client --test client -- --ignored --test-threads=1
```

Run the position and collateral tests without exchange credentials:
```bash
nix develop -c cargo test -p okex-client --test position_operations --locked
```
The hedging test only needs the migrated local database. Galoy and Bria connection
fixtures run on ephemeral local ports and reject unexpected API calls. No Galoy or
Bria environment variables or Tilt stack are required:
```sh
DATABASE_URL=postgres://user:password@localhost:5440/pg SQLX_OFFLINE=true cargo test -p hedging --test hedging
```
Its outer deadline is four 25-second phases plus an 80-second setup budget.

### Demo alert smoke checks

In the manual `OKX demo contract` workflow, select `alert_smoke_test=setup`,
`preflight`, or `contract` to deliberately fail that phase without installing Nix,
reading exchange credentials, contacting OKX, or creating/commenting on issues.
The alert job reads the failed job through the Actions API, asserts the expected
category and writes the result to the job summary. The overall run is intentionally
red; the alert job must be green. `off` runs the real demo tests.

Alert-script-only edits do not trigger demo trading. Run their Node regression tests
through `nix develop -c make check-code` (Node is included in the development shell).
The smoke check verifies failed-job classification, not actual issue notification delivery.

Verified on 2026-10-01 at `0c559cca`: all three deliberate failures produced a
successful alert job with the expected category, while real demo steps were skipped:
[setup](https://github.com/blinkbitcoin/stablesats-rs/actions/runs/36897928157),
[preflight](https://github.com/blinkbitcoin/stablesats-rs/actions/runs/36897840171), and
[contract](https://github.com/blinkbitcoin/stablesats-rs/actions/runs/36897749214).

## Database Configuration

The stablesats project uses different environment variables for database connections depending on the context:

### Migration vs Runtime vs Tests
- **`DATABASE_URL`**: Required by the complete suite, including isolated `#[sqlx::test]` databases, and by migrations (`cargo sqlx migrate run`). `make test-local` and `make test-local-ci` pass the same explicit port-5440 URL to migrations and nextest; they do not rely on Nix shell inheritance.
- **`PG_CON`**: Used by the main application runtime (passed via CLI)
- **`PG_HOST`/`PG_PORT`**: Fallback for older `DatabaseTestFixture` tests only; these do not configure `#[sqlx::test]`.

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
