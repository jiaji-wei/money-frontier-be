#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="${ROOT_DIR:-/workspace}"
CONTRACTS_DIR="${ROOT_DIR}/contracts"
RUNTIME_DIR="${RUNTIME_DIR:-${ROOT_DIR}/.dev/docker}"
ANVIL_RPC_URL="${ANVIL_RPC_URL:-http://anvil:8545}"
ANVIL_CHAIN_ID="${ANVIL_CHAIN_ID:-31337}"

DEPLOYER_PRIVATE_KEY="${DEPLOYER_PRIVATE_KEY:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}"
BUYER_PRIVATE_KEY="${BUYER_PRIVATE_KEY:-0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d}"

LOCAL_SETUP_LOG_FILE="${RUNTIME_DIR}/local-setup.log"
DEPLOY_OUTPUT_FILE="${RUNTIME_DIR}/deploy-output.json"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing command: $1" >&2
    exit 1
  fi
}

extract_logged_address() {
  local key="$1"
  local log_file="$2"
  awk -v k="${key}" '$1 == k {print $2}' "${log_file}" | tail -n1
}

wait_for_rpc() {
  local max_attempts=60
  local i

  for i in $(seq 1 "${max_attempts}"); do
    if cast block-number --rpc-url "${ANVIL_RPC_URL}" >/dev/null 2>&1; then
      return
    fi
    sleep 1
  done

  echo "anvil rpc is not ready: ${ANVIL_RPC_URL}" >&2
  exit 1
}

main() {
  local deployer_address
  local buyer_address
  local usdt
  local usdc
  local implementation
  local proxy
  local proxy_admin

  require_cmd forge
  require_cmd cast

  mkdir -p "${RUNTIME_DIR}"
  rm -f "${LOCAL_SETUP_LOG_FILE}" "${DEPLOY_OUTPUT_FILE}"

  deployer_address="$(cast wallet address --private-key "${DEPLOYER_PRIVATE_KEY}")"
  buyer_address="$(cast wallet address --private-key "${BUYER_PRIVATE_KEY}")"

  wait_for_rpc

  echo "docker contracts init"
  echo "  rpc: ${ANVIL_RPC_URL}"
  echo "  chain_id: ${ANVIL_CHAIN_ID}"
  echo "  deployer: ${deployer_address}"
  echo "  buyer: ${buyer_address}"

  (
    cd "${CONTRACTS_DIR}"
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
{"chain_id":"${ANVIL_CHAIN_ID}","rpc_url":"${ANVIL_RPC_URL}","usdt":"${usdt}","usdc":"${usdc}","implementation":"${implementation}","proxy":"${proxy}","proxy_admin":"${proxy_admin}","deployer":"${deployer_address}","buyer":"${buyer_address}"}
EOF

  # Seed approvals for the default buyer to simplify frontend testing.
  cast send "${usdt}" "approve(address,uint256)" "${proxy}" \
    0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff \
    --rpc-url "${ANVIL_RPC_URL}" \
    --private-key "${BUYER_PRIVATE_KEY}" >/dev/null
  cast send "${usdc}" "approve(address,uint256)" "${proxy}" \
    0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff \
    --rpc-url "${ANVIL_RPC_URL}" \
    --private-key "${BUYER_PRIVATE_KEY}" >/dev/null

  echo "contracts init done"
  echo "  deploy_output: ${DEPLOY_OUTPUT_FILE}"
  echo "  proxy: ${proxy}"
  echo "  usdt: ${usdt}"
  echo "  usdc: ${usdc}"
}

main "$@"
