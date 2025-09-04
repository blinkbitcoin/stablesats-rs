#!/bin/bash

echo "    --> starting tilt ci in cwd $(pwd)"

echo "    --> building project"
make build

REPO_ROOT=$(git rev-parse --show-toplevel)

echo "    --> calculated repo root: ${REPO_ROOT}"

echo "    --> setting honeycomb env vars to fake values"
export HONEYCOMB_API_KEY=your_honeycomb_key
export HONEYCOMB_DATASET=your_dataset_name

export HOST_PROJECT_PATH=$(pwd)
export COMPOSE_PROJECT_NAME=blink-quickstart
export GALOY_GRAPHQL_URI=http://localhost:4455/graphql
echo "        HONEYCOMB_API_KEY=${HONEYCOMB_API_KEY}"
echo "        HONEYCOMB_DATASET=${HONEYCOMB_DATASET}"
echo "        HOST_PROJECT_PATH=${HOST_PROJECT_PATH}"
echo "        COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME}"
echo "        GALOY_GRAPHQL_URI=${GALOY_GRAPHQL_URI}"

# Run tilt ci and capture its output and exit status
tilt ci --file "${REPO_ROOT}/Tiltfile" \
  | tee "${REPO_ROOT}/dev/.e2e-tilt.log"
  #| grep -- '^\s*test-.* │\|^\s*bats.* │'
status=${PIPESTATUS[0]}

if [[ $status -eq 0 ]]; then
  echo "    --> Tilt CI passed"
else
  echo "    --> Tilt CI failed with exit code $status"
fi

echo "    --> Tilt CI integration test logs:"
cat ${REPO_ROOT}/dev/.e2e-tilt.log | grep "integration-… │"

exit "$status"
