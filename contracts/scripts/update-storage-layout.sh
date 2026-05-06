#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${ROOT_DIR}/storage-layout"
OUTPUT_FILE="${OUTPUT_DIR}/TicketSale.v1.json"
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

mkdir -p "${OUTPUT_DIR}"
cd "${ROOT_DIR}"
forge inspect TicketSale storage-layout --json | jq "${NORMALIZE_FILTER}" > "${OUTPUT_FILE}"
echo "Updated storage layout baseline at ${OUTPUT_FILE}"
