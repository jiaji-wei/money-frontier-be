#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT_DIR="${ROOT_DIR}/storage-layout"
OUTPUT_FILE="${OUTPUT_DIR}/TicketSale.v1.json"

mkdir -p "${OUTPUT_DIR}"
cd "${ROOT_DIR}"
forge inspect TicketSale storage-layout --json > "${OUTPUT_FILE}"
echo "Updated storage layout baseline at ${OUTPUT_FILE}"
