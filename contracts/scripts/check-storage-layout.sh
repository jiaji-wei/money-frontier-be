#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE_FILE="${ROOT_DIR}/storage-layout/TicketSale.v1.json"
TMP_FILE="$(mktemp)"
NORMALIZED_BASELINE_FILE="$(mktemp)"
trap 'rm -f "${TMP_FILE}" "${NORMALIZED_BASELINE_FILE}"' EXIT
NORMALIZE_FILTER='
  def normalize_type_ids:
    if type == "string" then
      gsub("\\)[0-9]+_storage"; ")_storage")
    else
      .
    end;
  del(.. | .astId?)
  | walk(if type == "object" then with_entries(.key |= (gsub("\\)[0-9]+_storage"; ")_storage"))) else . end)
  | walk(normalize_type_ids)
'

cd "${ROOT_DIR}"
forge inspect TicketSale storage-layout --json | jq "${NORMALIZE_FILTER}" > "${TMP_FILE}"

if [[ ! -f "${BASELINE_FILE}" ]]; then
  echo "Baseline file not found: ${BASELINE_FILE}" >&2
  echo "Run scripts/update-storage-layout.sh first." >&2
  exit 1
fi

jq "${NORMALIZE_FILTER}" "${BASELINE_FILE}" > "${NORMALIZED_BASELINE_FILE}"

if diff -u "${NORMALIZED_BASELINE_FILE}" "${TMP_FILE}"; then
  echo "Storage layout unchanged."
else
  echo "Storage layout changed. Review diff before upgrade." >&2
  exit 1
fi
