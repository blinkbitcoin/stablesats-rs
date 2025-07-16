build: setup-db
	cargo build


start-db:
	docker-compose -f vendor/blink-quickstart/docker-compose.yml -f docker-compose.yml -f docker-compose.override.yml up -d stablesats-pg
	@echo "Waiting for database to be ready..."
	@sleep 5

setup-db: start-db
	cargo sqlx migrate run

stop-db:
	docker-compose -f vendor/blink-quickstart/docker-compose.yml -f docker-compose.yml -f docker-compose.override.yml down stablesats-pg

watch:
	RUST_BACKTRACE=full cargo watch -s 'cargo test -- --nocapture'

next-watch:
	cargo watch -s 'cargo nextest run'

check-code:
	SQLX_OFFLINE=true cargo fmt --check --all
	SQLX_OFFLINE=true cargo clippy --all-features
	SQLX_OFFLINE=true cargo audit

test-in-ci:
	./dev/bin/tilt-ci.sh

test-local: tilt-up-bg
	DATABASE_URL=postgres://user:password@localhost:5440/pg cargo sqlx migrate run
	export GALOY_GRAPHQL_URI="http://localhost:4455/graphql"
	export GALOY_PHONE_CODE="000000"
	PG_PORT=5440 SQLX_OFFLINE=true cargo nextest run --verbose --locked

tilt-up:
	tilt up

tilt-up-bg:
	tilt up &

cli-run:
	SQLX_OFFLINE=true cargo run --bin stablesats run

build-x86_64-unknown-linux-musl-release:
	SQLX_OFFLINE=true cargo build --release --locked --target x86_64-unknown-linux-musl

build-x86_64-apple-darwin-release:
	bin/osxcross-compile.sh

docker-down:
	docker compose down
