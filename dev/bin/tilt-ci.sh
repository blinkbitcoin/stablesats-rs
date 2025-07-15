#!/bin/bash

echo "    --> starting tilt ci in cwd $(pwd)"

echo "    --> building project"
make build

REPO_ROOT=$(git rev-parse --show-toplevel)

echo "    --> calculated repo root: ${REPO_ROOT}"

echo "    --> setting honeycomb env vars to fake values"
export HONEYCOMB_API_KEY=your_honeycomb_key
export HONEYCOMB_DATASET=your_dataset_name
echo "        HONEYCOMB_API_KEY=${HONEYCOMB_API_KEY}"
echo "        HONEYCOMB_DATASET=${HONEYCOMB_DATASET}"

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
