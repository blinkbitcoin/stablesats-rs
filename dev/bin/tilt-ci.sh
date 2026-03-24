#!/bin/bash
set -o pipefail

echo "    --> starting tilt ci in cwd $(pwd)"

echo "    --> building project"
make build

REPO_ROOT=$(git rev-parse --show-toplevel)

echo "    --> calculated repo root: ${REPO_ROOT}"

echo "    --> cleaning up existing tilt resources to avoid port conflicts"
tilt down --file "${REPO_ROOT}/Tiltfile" || true

echo "    --> setting honeycomb env vars to fake values"
export HONEYCOMB_API_KEY=your_honeycomb_key
export HONEYCOMB_DATASET=your_dataset_name

export HOST_PROJECT_PATH=$(pwd)
export COMPOSE_PROJECT_NAME=blink-quickstart
export GALOY_GRAPHQL_URI=http://localhost:4455/graphql
LOG_RESOURCE_FILTER=${TILT_LOG_RESOURCE_FILTER:-integration-tests}
LOG_RESOURCE_PREFIX=${LOG_RESOURCE_FILTER%%-*}
echo "        HONEYCOMB_API_KEY=${HONEYCOMB_API_KEY}"
echo "        HONEYCOMB_DATASET=${HONEYCOMB_DATASET}"
echo "        HOST_PROJECT_PATH=${HOST_PROJECT_PATH}"
echo "        COMPOSE_PROJECT_NAME=${COMPOSE_PROJECT_NAME}"
echo "        GALOY_GRAPHQL_URI=${GALOY_GRAPHQL_URI}"
echo "        TILT_LOG_RESOURCE_FILTER=${LOG_RESOURCE_FILTER}"
STATUS_POLL_INTERVAL_SECONDS=${STATUS_POLL_INTERVAL_SECONDS:-10}
LOG_FILE="${REPO_ROOT}/dev/.e2e-tilt.log"
declare -A PREV_STATUS

print_resource_status() {
  tilt get uiresources \
    -o custom-columns=NAME:.metadata.name,UPDATE:.status.updateStatus,RUNTIME:.status.runtimeStatus \
    --no-headers 2>/dev/null || true
}

print_status_loop() {
  while true; do
    while read -r name update runtime; do
      [[ -z "${name}" ]] && continue
      current_status="${update}/${runtime}"
      previous_status="${PREV_STATUS[${name}]}"
      if [[ "${current_status}" != "${previous_status}" ]]; then
        PREV_STATUS["${name}"]="${current_status}"
        if [[ "${update}" =~ ^(in_progress|ok|error)$ || "${runtime}" =~ ^(in_progress|ok|error)$ ]]; then
          printf "    --> %-24s %-12s %-12s\n" "${name}" "${update}" "${runtime}"
        fi
      fi
    done < <(print_resource_status)
    sleep "${STATUS_POLL_INTERVAL_SECONDS}"
  done
}

# Run tilt ci and capture its output and exit status
rm -f "${LOG_FILE}"
touch "${LOG_FILE}"

tail -n0 -F "${LOG_FILE}" \
  | awk -v filter="${LOG_RESOURCE_FILTER}" -v prefix="${LOG_RESOURCE_PREFIX}" '
      {
        line = $0
        sub(/^[[:space:]]+/, "", line)
        if (index(line, filter) == 1 || index(line, prefix) == 1) {
          print $0
          fflush()
        }
      }
    ' &
LOG_FILTER_PID=$!

print_status_loop &
STATUS_LOOP_PID=$!

tilt ci --file "${REPO_ROOT}/Tiltfile" \
  | tee "${LOG_FILE}" >/dev/null
status=${PIPESTATUS[0]}

kill "${LOG_FILTER_PID}" 2>/dev/null || true
wait "${LOG_FILTER_PID}" 2>/dev/null || true
kill "${STATUS_LOOP_PID}" 2>/dev/null || true
wait "${STATUS_LOOP_PID}" 2>/dev/null || true

if [[ $status -eq 0 ]]; then
  echo "    --> Tilt CI passed"
else
  echo "    --> Tilt CI failed with exit code $status"
fi

exit "$status"
