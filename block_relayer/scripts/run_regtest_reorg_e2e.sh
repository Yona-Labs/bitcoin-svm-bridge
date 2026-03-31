#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

BITCOIN_E2E_RPC_PORT="${BITCOIN_E2E_RPC_PORT:-18443}"
BITCOIN_E2E_RPC_USER="${BITCOIN_E2E_RPC_USER:-test}"
BITCOIN_E2E_RPC_PASSWORD="${BITCOIN_E2E_RPC_PASSWORD:-test}"
BITCOIN_E2E_WALLET="${BITCOIN_E2E_WALLET:-bridge-test}"
BITCOIN_REGTEST_IMAGE="${BITCOIN_REGTEST_IMAGE:-ruimarinho/bitcoin-core:23.0}"
BITCOIN_REGTEST_CONTAINER="${BITCOIN_REGTEST_CONTAINER:-bitcoin-regtest-e2e}"
BITCOIN_E2E_RPC_URL="${BITCOIN_E2E_RPC_URL:-http://127.0.0.1:${BITCOIN_E2E_RPC_PORT}}"

cleanup() {
  docker rm -f "${BITCOIN_REGTEST_CONTAINER}" >/dev/null 2>&1 || true
}

wait_for_rpc() {
  local attempt
  for attempt in $(seq 1 60); do
    if curl -sS \
      --user "${BITCOIN_E2E_RPC_USER}:${BITCOIN_E2E_RPC_PASSWORD}" \
      --header 'content-type: text/plain;' \
      --data-binary '{"jsonrpc":"1.0","id":"ping","method":"getblockchaininfo","params":[]}' \
      "${BITCOIN_E2E_RPC_URL}" >/dev/null; then
      return 0
    fi
    sleep 1
  done
  return 1
}

echo "Starting regtest bitcoind container ${BITCOIN_REGTEST_CONTAINER} from ${BITCOIN_REGTEST_IMAGE}"
cleanup
trap cleanup EXIT

docker run -d --rm \
  --name "${BITCOIN_REGTEST_CONTAINER}" \
  -p "${BITCOIN_E2E_RPC_PORT}:18443" \
  "${BITCOIN_REGTEST_IMAGE}" \
  -regtest=1 \
  -server=1 \
  -rpcbind=0.0.0.0 \
  -rpcallowip=0.0.0.0/0 \
  -rpcuser="${BITCOIN_E2E_RPC_USER}" \
  -rpcpassword="${BITCOIN_E2E_RPC_PASSWORD}" \
  -fallbackfee=0.00001 \
  -txindex=1 \
  -printtoconsole=1 >/dev/null

echo "Waiting for bitcoin RPC at ${BITCOIN_E2E_RPC_URL}"
wait_for_rpc || {
  echo "bitcoind RPC did not become ready in time" >&2
  docker logs "${BITCOIN_REGTEST_CONTAINER}" || true
  exit 1
}

echo "Running regtest BTC-side E2E suite"
cd "${ROOT_DIR}"
BITCOIN_E2E_RPC_URL="${BITCOIN_E2E_RPC_URL}" \
BITCOIN_E2E_RPC_USER="${BITCOIN_E2E_RPC_USER}" \
BITCOIN_E2E_RPC_PASSWORD="${BITCOIN_E2E_RPC_PASSWORD}" \
BITCOIN_E2E_WALLET="${BITCOIN_E2E_WALLET}" \
cargo test regtest_ -- --ignored --nocapture --test-threads=1
