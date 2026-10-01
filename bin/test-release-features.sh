#!/usr/bin/env bash
set -euo pipefail

target=x86_64-apple-darwin
bin/check-release-features.sh "$target"
output=$(mktemp)
trap 'rm -f "$output"' EXIT
if bin/check-release-features.sh "$target" --features okex-client/test-support >"$output" 2>&1; then
  echo "Release guard accepted test-support" >&2
  exit 1
fi
# A resolution/network failure is not evidence that the guard rejected the feature.
grep -q 'Release dependency graph enables test-support' "$output"
echo "Release guard accepted defaults and rejected test-support"
