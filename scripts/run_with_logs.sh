#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOG_DIR="${ROOT_DIR}/output"
LOG_FILE="${LOG_DIR}/learnchain.log"

mkdir -p "${LOG_DIR}"

echo "[run_with_logs] Streaming output to ${LOG_FILE}" >&2
cd "${ROOT_DIR}"
exec cargo run "$@" 2>&1 | tee "${LOG_FILE}"
