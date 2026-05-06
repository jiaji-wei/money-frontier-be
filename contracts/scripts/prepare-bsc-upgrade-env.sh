#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  prepare-bsc-upgrade-env.sh --rpc-url <url> --proxy <address> [options]

Options:
  --output <path>                   Write the generated env draft to a file instead of stdout.
  --purchase-signer <address>       Target purchase signer to configure during the upgrade.
  --expected-default-admin <addr>   Expected DEFAULT_ADMIN_ROLE holder after upgrade.
  --expected-pauser <addr>          Expected PAUSER_ROLE holder after upgrade.
  --expected-treasury <addr>        Expected treasury after upgrade. Defaults to the live treasury when readable.
  -h, --help                        Show this help.

Inputs can also be provided through env vars:
  BSC_RPC_URL / RPC_URL
  TICKET_SALE_PROXY
  PURCHASE_SIGNER
  EXPECTED_DEFAULT_ADMIN
  EXPECTED_PAUSER
  EXPECTED_TREASURY
EOF
}

require_command() {
  local command_name="$1"
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Missing required command: ${command_name}" >&2
    exit 1
  fi
}

checksum_address() {
  local raw_value="$1"
  if [[ -z "${raw_value}" ]]; then
    return 0
  fi
  cast to-check-sum-address "${raw_value}"
}

read_address_call() {
  local target="$1"
  local signature="$2"
  if cast call "${target}" "${signature}" --rpc-url "${RPC_URL}" 2>/dev/null; then
    return 0
  fi
  return 1
}

require_command cast
require_command date

RPC_URL="${BSC_RPC_URL:-${RPC_URL:-}}"
TICKET_SALE_PROXY="${TICKET_SALE_PROXY:-}"
OUTPUT_FILE=""
PURCHASE_SIGNER_VALUE="${PURCHASE_SIGNER:-}"
EXPECTED_DEFAULT_ADMIN_VALUE="${EXPECTED_DEFAULT_ADMIN:-}"
EXPECTED_PAUSER_VALUE="${EXPECTED_PAUSER:-}"
EXPECTED_TREASURY_VALUE="${EXPECTED_TREASURY:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --rpc-url)
      RPC_URL="$2"
      shift 2
      ;;
    --proxy|--ticket-sale-proxy)
      TICKET_SALE_PROXY="$2"
      shift 2
      ;;
    --output)
      OUTPUT_FILE="$2"
      shift 2
      ;;
    --purchase-signer)
      PURCHASE_SIGNER_VALUE="$2"
      shift 2
      ;;
    --expected-default-admin)
      EXPECTED_DEFAULT_ADMIN_VALUE="$2"
      shift 2
      ;;
    --expected-pauser)
      EXPECTED_PAUSER_VALUE="$2"
      shift 2
      ;;
    --expected-treasury)
      EXPECTED_TREASURY_VALUE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ -z "${RPC_URL}" ]]; then
  echo "Missing RPC URL. Pass --rpc-url or set BSC_RPC_URL/RPC_URL." >&2
  exit 1
fi

if [[ -z "${TICKET_SALE_PROXY}" ]]; then
  echo "Missing TicketSale proxy. Pass --proxy or set TICKET_SALE_PROXY." >&2
  exit 1
fi

TICKET_SALE_PROXY="$(checksum_address "${TICKET_SALE_PROXY}")"

PROXY_ADMIN="$(checksum_address "$(cast admin "${TICKET_SALE_PROXY}" --rpc-url "${RPC_URL}")")"
LIVE_IMPLEMENTATION="$(checksum_address "$(cast implementation "${TICKET_SALE_PROXY}" --rpc-url "${RPC_URL}")")"
EXPECTED_PROXY_ADMIN_OWNER="$(checksum_address "$(cast call "${PROXY_ADMIN}" "owner()(address)" --rpc-url "${RPC_URL}")")"

LIVE_TREASURY=""
if LIVE_TREASURY_RAW="$(read_address_call "${TICKET_SALE_PROXY}" "treasury()(address)")"; then
  LIVE_TREASURY="$(checksum_address "${LIVE_TREASURY_RAW}")"
fi

LIVE_PURCHASE_SIGNER=""
PURCHASE_SIGNER_NOTE=""
if LIVE_PURCHASE_SIGNER_RAW="$(read_address_call "${TICKET_SALE_PROXY}" "purchase_signer()(address)")"; then
  LIVE_PURCHASE_SIGNER="$(checksum_address "${LIVE_PURCHASE_SIGNER_RAW}")"
else
  PURCHASE_SIGNER_NOTE="# Current implementation does not expose purchase_signer(); fill the target signer manually."
fi

if [[ -z "${PURCHASE_SIGNER_VALUE}" && -n "${LIVE_PURCHASE_SIGNER}" ]]; then
  PURCHASE_SIGNER_VALUE="${LIVE_PURCHASE_SIGNER}"
fi
if [[ -z "${PURCHASE_SIGNER_VALUE}" ]]; then
  PURCHASE_SIGNER_VALUE="__SET_TARGET_PURCHASE_SIGNER__"
fi

if [[ -n "${PURCHASE_SIGNER_VALUE}" && "${PURCHASE_SIGNER_VALUE}" != __SET_TARGET_PURCHASE_SIGNER__ ]]; then
  PURCHASE_SIGNER_VALUE="$(checksum_address "${PURCHASE_SIGNER_VALUE}")"
fi

if [[ -z "${EXPECTED_TREASURY_VALUE}" && -n "${LIVE_TREASURY}" ]]; then
  EXPECTED_TREASURY_VALUE="${LIVE_TREASURY}"
fi
if [[ -z "${EXPECTED_TREASURY_VALUE}" ]]; then
  EXPECTED_TREASURY_VALUE="__SET_EXPECTED_TREASURY__"
fi
if [[ "${EXPECTED_TREASURY_VALUE}" != __SET_EXPECTED_TREASURY__ ]]; then
  EXPECTED_TREASURY_VALUE="$(checksum_address "${EXPECTED_TREASURY_VALUE}")"
fi

if [[ -z "${EXPECTED_DEFAULT_ADMIN_VALUE}" ]]; then
  EXPECTED_DEFAULT_ADMIN_VALUE="__SET_EXPECTED_DEFAULT_ADMIN__"
elif [[ "${EXPECTED_DEFAULT_ADMIN_VALUE}" != __SET_EXPECTED_DEFAULT_ADMIN__ ]]; then
  EXPECTED_DEFAULT_ADMIN_VALUE="$(checksum_address "${EXPECTED_DEFAULT_ADMIN_VALUE}")"
fi

if [[ -z "${EXPECTED_PAUSER_VALUE}" ]]; then
  EXPECTED_PAUSER_VALUE="__SET_EXPECTED_PAUSER__"
elif [[ "${EXPECTED_PAUSER_VALUE}" != __SET_EXPECTED_PAUSER__ ]]; then
  EXPECTED_PAUSER_VALUE="$(checksum_address "${EXPECTED_PAUSER_VALUE}")"
fi

GENERATED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
OUTPUT_CONTENT="$(cat <<EOF
# Generated by scripts/prepare-bsc-upgrade-env.sh at ${GENERATED_AT}
# Live proxy snapshot:
#   ticket_sale_proxy=${TICKET_SALE_PROXY}
#   proxy_admin=${PROXY_ADMIN}
#   proxy_admin_owner=${EXPECTED_PROXY_ADMIN_OWNER}
#   live_implementation=${LIVE_IMPLEMENTATION}
${PURCHASE_SIGNER_NOTE}

RPC_URL=${RPC_URL}
PRIVATE_KEY=__SET_PROXY_ADMIN_OWNER_PRIVATE_KEY__
TICKET_SALE_PROXY=${TICKET_SALE_PROXY}
PROXY_ADMIN=${PROXY_ADMIN}
NEW_IMPLEMENTATION=0x0000000000000000000000000000000000000000
PURCHASE_SIGNER=${PURCHASE_SIGNER_VALUE}
UPGRADE_OUTPUT_FILE=./upgrade-output.json

# Optional preflight assertions for script/PreflightTicketSaleUpgrade.s.sol
EXPECTED_PROXY_ADMIN=${PROXY_ADMIN}
EXPECTED_PROXY_ADMIN_OWNER=${EXPECTED_PROXY_ADMIN_OWNER}
EXPECTED_IMPLEMENTATION=${LIVE_IMPLEMENTATION}
EXPECTED_PURCHASE_SIGNER=${PURCHASE_SIGNER_VALUE}

# Replace EXPECTED_IMPLEMENTATION with the upgraded implementation from UPGRADE_OUTPUT_FILE before running VerifyTicketSaleUpgrade.
EXPECTED_DEFAULT_ADMIN=${EXPECTED_DEFAULT_ADMIN_VALUE}
EXPECTED_PAUSER=${EXPECTED_PAUSER_VALUE}
EXPECTED_TREASURY=${EXPECTED_TREASURY_VALUE}
EOF
)"

if [[ -n "${OUTPUT_FILE}" ]]; then
  printf '%s\n' "${OUTPUT_CONTENT}" > "${OUTPUT_FILE}"
  echo "Wrote upgrade env draft to ${OUTPUT_FILE}"
else
  printf '%s\n' "${OUTPUT_CONTENT}"
fi
