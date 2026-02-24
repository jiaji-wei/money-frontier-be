#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEV_DIR="${ROOT_DIR}/.dev/local"
BACKEND_ENV_FILE="${DEV_DIR}/backend.env"
DEPLOY_OUTPUT_FILE="${DEV_DIR}/deploy-output.json"
SESSION_FILE="${DEV_DIR}/user-session.json"

DEFAULT_BUYER_PRIVATE_KEY="${BUYER_PRIVATE_KEY:-0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d}"
LOCAL_NO_PROXY_SUFFIX="127.0.0.1,localhost"

unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy
export NO_PROXY="${LOCAL_NO_PROXY_SUFFIX}${NO_PROXY:+,${NO_PROXY}}"
export no_proxy="${LOCAL_NO_PROXY_SUFFIX}${no_proxy:+,${no_proxy}}"

BACKEND_BASE_URL="${BACKEND_BASE_URL:-http://127.0.0.1:8080}"
CHAIN_ID=""
RPC_URL=""
SALE_CONTRACT=""
USDT_TOKEN=""
USDC_TOKEN=""

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing command: $1" >&2
    exit 1
  fi
}

load_context() {
  if [[ ! -f "${BACKEND_ENV_FILE}" ]]; then
    echo "missing backend env file: ${BACKEND_ENV_FILE}" >&2
    echo "run ./scripts/dev-up.sh first" >&2
    exit 1
  fi
  if [[ ! -f "${DEPLOY_OUTPUT_FILE}" ]]; then
    echo "missing deploy output file: ${DEPLOY_OUTPUT_FILE}" >&2
    echo "run ./scripts/dev-up.sh first" >&2
    exit 1
  fi

  set -a
  # shellcheck disable=SC1090
  source "${BACKEND_ENV_FILE}"
  set +a

  CHAIN_ID="$(jq -r '.[0].chain_id' <<< "${APP_CHAINS_JSON}")"
  RPC_URL="$(jq -r '.[0].rpc_url' <<< "${APP_CHAINS_JSON}")"
  SALE_CONTRACT="$(jq -r '.proxy' "${DEPLOY_OUTPUT_FILE}")"
  USDT_TOKEN="$(jq -r '.usdt' "${DEPLOY_OUTPUT_FILE}")"
  USDC_TOKEN="$(jq -r '.usdc' "${DEPLOY_OUTPUT_FILE}")"

  if [[ -z "${CHAIN_ID}" || -z "${RPC_URL}" || -z "${SALE_CONTRACT}" ]]; then
    echo "invalid local context, check ${BACKEND_ENV_FILE} and ${DEPLOY_OUTPUT_FILE}" >&2
    exit 1
  fi
}

session_get() {
  local key="$1"
  if [[ ! -f "${SESSION_FILE}" ]]; then
    return 0
  fi
  jq -r --arg key "${key}" '.[$key] // empty' "${SESSION_FILE}"
}

session_set() {
  local key="$1"
  local value="$2"
  local tmp_file
  tmp_file="$(mktemp)"

  if [[ -f "${SESSION_FILE}" ]]; then
    jq --arg key "${key}" --arg value "${value}" '.[$key] = $value' "${SESSION_FILE}" > "${tmp_file}"
  else
    jq -n --arg key "${key}" --arg value "${value}" '{($key): $value}' > "${tmp_file}"
  fi

  mv "${tmp_file}" "${SESSION_FILE}"
}

auth_token_or_fail() {
  local token
  token="$(session_get "jwt_token")"
  if [[ -z "${token}" ]]; then
    echo "missing jwt token, run: ./scripts/user-flow.sh signin" >&2
    exit 1
  fi
  echo "${token}"
}

json_post() {
  local path="$1"
  local payload="$2"
  local token="${3:-}"
  local response
  local body
  local status

  if [[ -n "${token}" ]]; then
    response="$(
      curl -sS -X POST \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer ${token}" \
        -d "${payload}" \
        -w $'\n%{http_code}' \
        "${BACKEND_BASE_URL}${path}"
    )"
  else
    response="$(
      curl -sS -X POST \
        -H "Content-Type: application/json" \
        -d "${payload}" \
        -w $'\n%{http_code}' \
        "${BACKEND_BASE_URL}${path}"
    )"
  fi

  body="$(sed '$d' <<< "${response}")"
  status="$(tail -n1 <<< "${response}")"

  if [[ "${status}" != 2* ]]; then
    echo "request failed: POST ${path} (status=${status})" >&2
    echo "${body}" >&2
    exit 1
  fi

  echo "${body}"
}

json_put() {
  local path="$1"
  local payload="$2"
  local token="$3"
  local response
  local body
  local status

  response="$(
    curl -sS -X PUT \
      -H "Content-Type: application/json" \
      -H "Authorization: Bearer ${token}" \
      -d "${payload}" \
      -w $'\n%{http_code}' \
      "${BACKEND_BASE_URL}${path}"
  )"

  body="$(sed '$d' <<< "${response}")"
  status="$(tail -n1 <<< "${response}")"

  if [[ "${status}" != 2* ]]; then
    echo "request failed: PUT ${path} (status=${status})" >&2
    echo "${body}" >&2
    exit 1
  fi

  echo "${body}"
}

json_get() {
  local path="$1"
  local token="$2"
  local response
  local body
  local status

  response="$(
    curl -sS \
      -H "Authorization: Bearer ${token}" \
      -w $'\n%{http_code}' \
      "${BACKEND_BASE_URL}${path}"
  )"

  body="$(sed '$d' <<< "${response}")"
  status="$(tail -n1 <<< "${response}")"

  if [[ "${status}" != 2* ]]; then
    echo "request failed: GET ${path} (status=${status})" >&2
    echo "${body}" >&2
    exit 1
  fi

  echo "${body}"
}

csv_to_array_literal() {
  local csv="$1"
  local compact
  compact="$(tr -d ' ' <<< "${csv}")"
  if [[ -z "${compact}" ]]; then
    echo "[]"
    return
  fi
  echo "[${compact}]"
}

resolve_payment_token() {
  local token="$1"
  case "${token}" in
    usdt|USDT)
      echo "${USDT_TOKEN}"
      ;;
    usdc|USDC)
      echo "${USDC_TOKEN}"
      ;;
    0x*)
      echo "${token}"
      ;;
    *)
      echo "unsupported payment token: ${token}" >&2
      exit 1
      ;;
  esac
}

command_signin() {
  local private_key="${1:-${DEFAULT_BUYER_PRIVATE_KEY}}"
  local wallet
  local challenge_resp
  local challenge_id
  local challenge_message
  local signature
  local signin_resp
  local token

  wallet="$(cast wallet address --private-key "${private_key}")"
  challenge_resp="$(json_post "/signin/challenge" "$(jq -nc --arg address "${wallet}" '{address: $address}')")"
  challenge_id="$(jq -r '.challenge_id' <<< "${challenge_resp}")"
  challenge_message="$(jq -r '.challenge_message' <<< "${challenge_resp}")"

  signature="$(cast wallet sign --private-key "${private_key}" "${challenge_message}")"
  signin_resp="$(
    json_post "/signin" "$(
      jq -nc \
        --arg address "${wallet}" \
        --arg challenge_id "${challenge_id}" \
        --arg signature "${signature}" \
        '{address: $address, challenge_id: $challenge_id, signature: $signature}'
    )"
  )"
  token="$(jq -r '.token' <<< "${signin_resp}")"

  mkdir -p "${DEV_DIR}"
  session_set "jwt_token" "${token}"
  session_set "wallet" "${wallet}"
  session_set "chain_id" "${CHAIN_ID}"

  echo "signin success"
  echo "wallet: ${wallet}"
  echo "token saved: ${SESSION_FILE}"
}

command_buy() {
  local payment_token_name="usdt"
  local levels_csv="1"
  local quantities_csv="1"
  local private_key="${DEFAULT_BUYER_PRIVATE_KEY}"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --token)
        payment_token_name="${2:-}"
        shift 2
        ;;
      --levels)
        levels_csv="${2:-}"
        shift 2
        ;;
      --quantities)
        quantities_csv="${2:-}"
        shift 2
        ;;
      --private-key)
        private_key="${2:-}"
        shift 2
        ;;
      *)
        echo "unknown option for buy: $1" >&2
        exit 1
        ;;
    esac
  done

  local payment_token
  local levels_array
  local quantities_array
  local tx_json
  local tx_hash

  payment_token="$(resolve_payment_token "${payment_token_name}")"
  levels_array="$(csv_to_array_literal "${levels_csv}")"
  quantities_array="$(csv_to_array_literal "${quantities_csv}")"

  tx_json="$(
    cast send "${SALE_CONTRACT}" \
      "purchase(address,uint8[],uint256[])" \
      "${payment_token}" \
      "${levels_array}" \
      "${quantities_array}" \
      --rpc-url "${RPC_URL}" \
      --private-key "${private_key}" \
      --json
  )"
  tx_hash="$(jq -r '.transactionHash // empty' <<< "${tx_json}")"

  if [[ -z "${tx_hash}" ]]; then
    echo "failed to parse transaction hash from cast send output" >&2
    echo "${tx_json}" >&2
    exit 1
  fi

  session_set "last_tx_hash" "${tx_hash}"
  session_set "last_payment_token" "${payment_token}"
  echo "purchase tx sent: ${tx_hash}"
}

command_notify() {
  local tx_hash="${1:-$(session_get "last_tx_hash")}"
  local chain_id="${2:-${CHAIN_ID}}"
  local token
  local resp

  if [[ -z "${tx_hash}" ]]; then
    echo "missing tx hash, run buy first or pass: notify <tx_hash>" >&2
    exit 1
  fi

  token="$(auth_token_or_fail)"
  resp="$(
    json_post "/tickets" "$(
      jq -nc --argjson chain_id "${chain_id}" --arg tx_hash "${tx_hash}" \
        '{chain_id: $chain_id, tx_hash: $tx_hash}'
    )" "${token}"
  )"
  echo "${resp}" | jq .
}

command_list() {
  local token
  local resp
  token="$(auth_token_or_fail)"
  resp="$(json_get "/tickets" "${token}")"
  echo "${resp}" | jq .
}

command_get() {
  local ticket_id="${1:-}"
  local token

  if [[ -z "${ticket_id}" ]]; then
    echo "usage: get <ticket_id>" >&2
    exit 1
  fi

  token="$(auth_token_or_fail)"
  json_get "/tickets/${ticket_id}" "${token}" | jq .
}

command_transfer_email() {
  local ticket_id="${1:-}"
  local email="${2:-}"
  local token

  if [[ -z "${ticket_id}" || -z "${email}" ]]; then
    echo "usage: transfer-email <ticket_id> <email>" >&2
    exit 1
  fi

  token="$(auth_token_or_fail)"
  json_put "/tickets/${ticket_id}" "$(jq -nc --arg to_email "${email}" '{to_email: $to_email}')" "${token}" | jq .
}

command_transfer_wallet() {
  local ticket_id="${1:-}"
  local to_wallet="${2:-}"
  local token

  if [[ -z "${ticket_id}" || -z "${to_wallet}" ]]; then
    echo "usage: transfer-wallet <ticket_id> <wallet_address>" >&2
    exit 1
  fi

  token="$(auth_token_or_fail)"
  json_put "/tickets/${ticket_id}" "$(jq -nc --arg to_wallet "${to_wallet}" '{to_wallet: $to_wallet}')" "${token}" | jq .
}

command_flow() {
  local private_key="${1:-${DEFAULT_BUYER_PRIVATE_KEY}}"
  local levels="${2:-1}"
  local quantities="${3:-1}"
  local token_name="${4:-usdt}"

  command_signin "${private_key}"
  command_buy --private-key "${private_key}" --token "${token_name}" --levels "${levels}" --quantities "${quantities}"
  command_notify
  command_list
}

command_status() {
  local wallet
  local chain
  local tx
  wallet="$(session_get "wallet")"
  chain="$(session_get "chain_id")"
  tx="$(session_get "last_tx_hash")"

  cat <<EOF
backend_base_url: ${BACKEND_BASE_URL}
chain_id: ${CHAIN_ID}
rpc_url: ${RPC_URL}
sale_contract: ${SALE_CONTRACT}
usdt: ${USDT_TOKEN}
usdc: ${USDC_TOKEN}
session_file: ${SESSION_FILE}
session.wallet: ${wallet}
session.chain_id: ${chain}
session.last_tx_hash: ${tx}
EOF
}

print_help() {
  cat <<EOF
Usage:
  ./scripts/user-flow.sh <command> [args...]

Commands:
  signin [private_key]
  buy [--token usdt|usdc|0xToken] [--levels 1,2] [--quantities 1,1] [--private-key 0x...]
  notify [tx_hash] [chain_id]
  list
  get <ticket_id>
  transfer-email <ticket_id> <email>
  transfer-wallet <ticket_id> <wallet_address>
  flow [private_key] [levels_csv] [quantities_csv] [token]
  status

Examples:
  ./scripts/user-flow.sh signin
  ./scripts/user-flow.sh buy --token usdt --levels 1,2 --quantities 1,3
  ./scripts/user-flow.sh notify
  ./scripts/user-flow.sh list
  ./scripts/user-flow.sh transfer-email <ticket_id> receiver@example.com
EOF
}

main() {
  require_cmd jq
  require_cmd curl
  require_cmd cast
  load_context

  local command="${1:-help}"
  shift || true

  case "${command}" in
    signin)
      command_signin "$@"
      ;;
    buy)
      command_buy "$@"
      ;;
    notify)
      command_notify "$@"
      ;;
    list)
      command_list "$@"
      ;;
    get)
      command_get "$@"
      ;;
    transfer-email)
      command_transfer_email "$@"
      ;;
    transfer-wallet)
      command_transfer_wallet "$@"
      ;;
    flow)
      command_flow "$@"
      ;;
    status)
      command_status
      ;;
    help|-h|--help)
      print_help
      ;;
    *)
      echo "unknown command: ${command}" >&2
      print_help
      exit 1
      ;;
  esac
}

main "$@"
