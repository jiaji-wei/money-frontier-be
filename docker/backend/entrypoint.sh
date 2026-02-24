#!/bin/sh
set -eu

RUNTIME_DIR="${RUNTIME_DIR:-/runtime}"
ANVIL_RPC_URL="${ANVIL_RPC_URL:-http://anvil:8545}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"
DEPLOY_OUTPUT_FILE="${RUNTIME_DIR}/deploy-output.json"
DB_FILE="${RUNTIME_DIR}/ticket.dev.db"

mkdir -p "${RUNTIME_DIR}"

wait_for_deploy_output() {
  i=0
  while [ ! -f "${DEPLOY_OUTPUT_FILE}" ]; do
    i=$((i + 1))
    if [ "${i}" -gt 60 ]; then
      echo "deploy output not found: ${DEPLOY_OUTPUT_FILE}" >&2
      exit 1
    fi
    sleep 1
  done
}

wait_for_deploy_output

PROXY_ADDR="$(jq -r '.proxy // empty' "${DEPLOY_OUTPUT_FILE}")"
if [ -z "${PROXY_ADDR}" ]; then
  echo "missing proxy address in ${DEPLOY_OUTPUT_FILE}" >&2
  exit 1
fi

export BIND_ADDR="${BIND_ADDR:-0.0.0.0:8080}"
export DATABASE_URL="${DATABASE_URL:-sqlite://${DB_FILE}}"
export JWT_SECRET="${JWT_SECRET:-local-dev-secret}"
export JWT_TTL_DAYS="${JWT_TTL_DAYS:-3650}"
export SIGNIN_CHALLENGE_TTL_SECS="${SIGNIN_CHALLENGE_TTL_SECS:-300}"
export MAIL_FROM="${MAIL_FROM:-noreply@tickets.local}"
export MAIL_PROVIDER="${MAIL_PROVIDER:-console}"
export MAIL_WEBHOOK_URL="${MAIL_WEBHOOK_URL:-}"
export MAIL_API_KEY="${MAIL_API_KEY:-}"
export MAIL_MAX_RETRIES="${MAIL_MAX_RETRIES:-3}"
export MAIL_RETRY_BACKOFF_MS="${MAIL_RETRY_BACKOFF_MS:-300}"
export MAIL_ALERT_WEBHOOK_URL="${MAIL_ALERT_WEBHOOK_URL:-}"
export MAIL_ALERT_API_KEY="${MAIL_ALERT_API_KEY:-}"
export INDEXER_POLL_INTERVAL_SECS="${INDEXER_POLL_INTERVAL_SECS:-2}"
export INDEXER_BATCH_SIZE="${INDEXER_BATCH_SIZE:-200}"
export INDEXER_REORG_ROLLBACK_BLOCKS="${INDEXER_REORG_ROLLBACK_BLOCKS:-32}"
export SIGNIN_CLEANUP_INTERVAL_SECS="${SIGNIN_CLEANUP_INTERVAL_SECS:-600}"
export SIGNIN_CLEANUP_RETENTION_SECS="${SIGNIN_CLEANUP_RETENTION_SECS:-86400}"
export APP_CHAINS_JSON="${APP_CHAINS_JSON:-[{\"chain_id\":${ANVIL_CHAIN_ID},\"rpc_url\":\"${ANVIL_RPC_URL}\",\"sale_contract\":\"${PROXY_ADDR}\",\"start_block\":null,\"confirmations\":0}]}"

echo "backend runtime config"
echo "  rpc: ${ANVIL_RPC_URL}"
echo "  chain_id: ${ANVIL_CHAIN_ID}"
echo "  sale_contract: ${PROXY_ADDR}"
echo "  database: ${DATABASE_URL}"

exec /opt/ticket-backend/ticket-backend
