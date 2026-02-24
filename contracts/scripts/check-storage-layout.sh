#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE_FILE="${ROOT_DIR}/storage-layout/TicketSale.v1.json"
TMP_FILE="$(mktemp)"
trap 'rm -f "${TMP_FILE}"' EXIT

cd "${ROOT_DIR}"
forge inspect TicketSale storage-layout --json > "${TMP_FILE}"

if [[ ! -f "${BASELINE_FILE}" ]]; then
  echo "Baseline file not found: ${BASELINE_FILE}" >&2
  echo "Run scripts/update-storage-layout.sh first." >&2
  exit 1
fi

if diff -u "${BASELINE_FILE}" "${TMP_FILE}"; then
  echo "Storage layout unchanged."
else
  echo "Storage layout changed. Review diff before upgrade." >&2
  exit 1
fi
