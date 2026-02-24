#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEV_DIR="${ROOT_DIR}/.dev/local"
ANVIL_LOG="${DEV_DIR}/anvil.log"
BACKEND_LOG="${DEV_DIR}/backend.log"
ANVIL_PID_FILE="${DEV_DIR}/anvil.pid"
BACKEND_PID_FILE="${DEV_DIR}/backend.pid"
DEPLOY_OUTPUT_FILE="${DEV_DIR}/deploy-output.json"
BACKEND_ENV_FILE="${DEV_DIR}/backend.env"
LOCAL_SETUP_LOG_FILE="${DEV_DIR}/local-setup.log"

ANVIL_HOST="${ANVIL_HOST:-127.0.0.1}"
ANVIL_PORT="${ANVIL_PORT:-8545}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"
ANVIL_RPC_URL="${ANVIL_RPC_URL:-http://${ANVIL_HOST}:${ANVIL_PORT}}"

DEPLOYER_PRIVATE_KEY="${DEPLOYER_PRIVATE_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"
BUYER_PRIVATE_KEY="${BUYER_PRIVATE_KEY:-0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d}"
LOCAL_NO_PROXY_SUFFIX="127.0.0.1,localhost"

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
export NO_PROXY="${LOCAL_NO_PROXY_SUFFIX}${NO_PROXY:+,${NO_PROXY}}"
export no_proxy="${LOCAL_NO_PROXY_SUFFIX}${no_proxy:+,${no_proxy}}"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing command: $1" >&2
    exit 1
  fi
}

process_alive() {
  local pid="$1"
  kill -0 "${pid}" >/dev/null 2>&1
}

read_json_value() {
  local file="$1"
  local key="$2"

  if command -v jq >/dev/null 2>&1; then
    jq -r ".${key}" "${file}"
    return
  fi

  sed -n "s/.*\"${key}\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "${file}" | head -n1
}

extract_logged_address() {
  local key="$1"
  local log_file="$2"

  awk -v k="${key}" '$1 == k {print $2}' "${log_file}" | tail -n1
}

wait_for_rpc() {
  for _ in $(seq 1 30); do
    if cast block-number --rpc-url "${ANVIL_RPC_URL}" >/dev/null 2>&1; then
      return
    fi
    sleep 1
  done
  echo "anvil rpc is not ready: ${ANVIL_RPC_URL}" >&2
  exit 1
}

start_anvil() {
  mkdir -p "${DEV_DIR}"

  if [[ -f "${ANVIL_PID_FILE}" ]]; then
    local existing_pid
    existing_pid="$(cat "${ANVIL_PID_FILE}")"
    if process_alive "${existing_pid}"; then
      echo "anvil already running (pid=${existing_pid})"
      return
    fi
  fi

  echo "starting anvil on ${ANVIL_RPC_URL}"
  anvil \
    --host "${ANVIL_HOST}" \
    --port "${ANVIL_PORT}" \
    --chain-id "${ANVIL_CHAIN_ID}" \
    --block-time 1 \
    >"${ANVIL_LOG}" 2>&1 &

  local anvil_pid=$!
  echo "${anvil_pid}" > "${ANVIL_PID_FILE}"
  wait_for_rpc
}

deploy_contracts() {
  local deployer_address
  local buyer_address

  deployer_address="$(cast wallet address --private-key "${DEPLOYER_PRIVATE_KEY}")"
  buyer_address="$(cast wallet address --private-key "${BUYER_PRIVATE_KEY}")"

  echo "deployer address: ${deployer_address}"
  echo "buyer address: ${buyer_address}"
  echo "deploying local contracts"

  (
    cd "${ROOT_DIR}/contracts"
    OWNER="${deployer_address}" \
    PAUSER="${deployer_address}" \
    PROXY_ADMIN_OWNER="${deployer_address}" \
    TREASURY="${deployer_address}" \
    BUYER="${buyer_address}" \
    forge script script/LocalSetup.s.sol:LocalSetupScript \
      --rpc-url "${ANVIL_RPC_URL}" \
      --private-key "${DEPLOYER_PRIVATE_KEY}" \
      --broadcast
  ) | tee "${LOCAL_SETUP_LOG_FILE}"

  local usdt
  local usdc
  local implementation
  local proxy
  local proxy_admin

  usdt="$(extract_logged_address "local_usdt" "${LOCAL_SETUP_LOG_FILE}")"
  usdc="$(extract_logged_address "local_usdc" "${LOCAL_SETUP_LOG_FILE}")"
  implementation="$(extract_logged_address "ticket_sale_implementation" "${LOCAL_SETUP_LOG_FILE}")"
  proxy="$(extract_logged_address "ticket_sale_proxy" "${LOCAL_SETUP_LOG_FILE}")"
  proxy_admin="$(extract_logged_address "ticket_sale_proxy_admin" "${LOCAL_SETUP_LOG_FILE}")"

  if [[ -z "${usdt}" || -z "${usdc}" || -z "${implementation}" || -z "${proxy}" || -z "${proxy_admin}" ]]; then
    echo "failed to parse deployment logs: ${LOCAL_SETUP_LOG_FILE}" >&2
    exit 1
  fi

  cat > "${DEPLOY_OUTPUT_FILE}" <<EOF
{"usdt":"${usdt}","usdc":"${usdc}","implementation":"${implementation}","proxy":"${proxy}","proxy_admin":"${proxy_admin}"}
EOF

  if [[ ! -f "${DEPLOY_OUTPUT_FILE}" ]]; then
    echo "missing deploy output: ${DEPLOY_OUTPUT_FILE}" >&2
    exit 1
  fi
}

seed_allowances() {
  local proxy
  local usdt
  local usdc

  proxy="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "proxy")"
  usdt="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "usdt")"
  usdc="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "usdc")"

  if [[ -z "${proxy}" || -z "${usdt}" || -z "${usdc}" ]]; then
    echo "failed to parse deploy output addresses" >&2
    exit 1
  fi

  local max_uint256="0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"

  echo "approving sale contract for buyer wallet"
  cast send "${usdt}" "approve(address,uint256)" "${proxy}" "${max_uint256}" \
    --rpc-url "${ANVIL_RPC_URL}" \
    --private-key "${BUYER_PRIVATE_KEY}" >/dev/null
  cast send "${usdc}" "approve(address,uint256)" "${proxy}" "${max_uint256}" \
    --rpc-url "${ANVIL_RPC_URL}" \
    --private-key "${BUYER_PRIVATE_KEY}" >/dev/null
}

write_backend_env() {
  local proxy
  proxy="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "proxy")"
  if [[ -z "${proxy}" ]]; then
    echo "missing proxy address in deploy output" >&2
    exit 1
  fi

  cat > "${BACKEND_ENV_FILE}" <<EOF
BIND_ADDR=0.0.0.0:8080
DATABASE_URL=sqlite://ticket.dev.db
JWT_SECRET=local-dev-secret
JWT_TTL_DAYS=3650
SIGNIN_CHALLENGE_TTL_SECS=300
MAIL_FROM=noreply@tickets.local
MAIL_PROVIDER=console
MAIL_WEBHOOK_URL=
MAIL_API_KEY=
MAIL_MAX_RETRIES=3
MAIL_RETRY_BACKOFF_MS=300
MAIL_ALERT_WEBHOOK_URL=
MAIL_ALERT_API_KEY=
INDEXER_POLL_INTERVAL_SECS=2
INDEXER_BATCH_SIZE=200
INDEXER_REORG_ROLLBACK_BLOCKS=32
SIGNIN_CLEANUP_INTERVAL_SECS=600
SIGNIN_CLEANUP_RETENTION_SECS=86400
APP_CHAINS_JSON='[{"chain_id":31337,"rpc_url":"${ANVIL_RPC_URL}","sale_contract":"${proxy}","start_block":null,"confirmations":0}]'
EOF
}

start_backend() {
  touch "${ROOT_DIR}/backend/ticket.dev.db"

  if [[ -f "${BACKEND_PID_FILE}" ]]; then
    local existing_pid
    existing_pid="$(cat "${BACKEND_PID_FILE}")"
    if process_alive "${existing_pid}"; then
      echo "stopping existing backend process (pid=${existing_pid})"
      kill "${existing_pid}" || true
      sleep 1
    fi
  fi

  echo "starting backend"
  (
    cd "${ROOT_DIR}/backend"
    set -a
    # shellcheck disable=SC1090
    source "${BACKEND_ENV_FILE}"
    set +a
    cargo run
  ) >"${BACKEND_LOG}" 2>&1 &
  local backend_pid=$!
  echo "${backend_pid}" > "${BACKEND_PID_FILE}"

  for _ in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:8080/health" >/dev/null 2>&1; then
      return
    fi
    sleep 1
  done

  if process_alive "${backend_pid}"; then
    kill "${backend_pid}" || true
  fi
  rm -f "${BACKEND_PID_FILE}"
  echo "backend did not become healthy, check log: ${BACKEND_LOG}" >&2
  exit 1
}

print_summary() {
  local deployer_address
  local buyer_address
  local proxy
  local usdt
  local usdc

  deployer_address="$(cast wallet address --private-key "${DEPLOYER_PRIVATE_KEY}")"
  buyer_address="$(cast wallet address --private-key "${BUYER_PRIVATE_KEY}")"
  proxy="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "proxy")"
  usdt="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "usdt")"
  usdc="$(read_json_value "${DEPLOY_OUTPUT_FILE}" "usdc")"

  cat <<EOF
local test environment is ready
rpc: ${ANVIL_RPC_URL}
backend: http://127.0.0.1:8080
ticket sale proxy: ${proxy}
usdt: ${usdt}
usdc: ${usdc}
deployer: ${deployer_address}
buyer: ${buyer_address}

generated files:
- ${DEPLOY_OUTPUT_FILE}
- ${BACKEND_ENV_FILE}
- ${LOCAL_SETUP_LOG_FILE}
- ${ANVIL_LOG}
- ${BACKEND_LOG}

shutdown command:
./scripts/dev-down.sh
EOF
}

main() {
  require_cmd anvil
  require_cmd forge
  require_cmd cast
  require_cmd cargo
  require_cmd curl

  start_anvil
  deploy_contracts
  seed_allowances
  write_backend_env
  start_backend
  print_summary
}

main "$@"
