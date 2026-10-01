#!/usr/bin/env bash
set -euo pipefail

target=$1
shift
# Check exactly the package, target, and feature flags used for the build.
bin/check-release-features.sh "$target" "$@"
SQLX_OFFLINE=true cargo build --locked --release -p stablesats --target "$target" "$@"
