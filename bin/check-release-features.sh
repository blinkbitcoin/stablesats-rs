#!/usr/bin/env bash
set -euo pipefail

target=$1
shift

# Resolve only the application build graph, excluding dev-dependencies.
features=$(cargo tree --locked -p stablesats --target "$target" "$@" --edges normal,build --prefix none --format '{p} {f}')
if [[ "$features" == *test-support* ]]; then
  echo "Release dependency graph enables test-support" >&2
  exit 1
fi
