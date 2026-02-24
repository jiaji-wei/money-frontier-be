#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_ROOT="${ROOT_DIR}/dist/prebuilt"
BIN_NAME="ticket-backend"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing command: $1" >&2
    exit 1
  fi
}

normalize_os() {
  local os
  os="$(uname -s)"
  case "${os}" in
    Darwin) echo "darwin" ;;
    Linux) echo "linux" ;;
    *) echo "unsupported-os-${os}" ;;
  esac
}

normalize_arch() {
  local arch
  arch="$(uname -m)"
  case "${arch}" in
    x86_64) echo "amd64" ;;
    arm64|aarch64) echo "arm64" ;;
    *) echo "unsupported-arch-${arch}" ;;
  esac
}

sha256_file() {
  local file="$1"
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "${file}" | awk '{print $1}'
    return
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "${file}" | awk '{print $1}'
    return
  fi
  echo "unavailable"
}

main() {
  local platform
  local out_dir
  local docs_dir
  local abi_dir
  local src_bin
  local out_bin
  local archive
  local checksum
  local abi_src

  require_cmd cargo
  require_cmd tar

  platform="$(normalize_os)-$(normalize_arch)"
  out_dir="${DIST_ROOT}/${platform}"
  docs_dir="${out_dir}/docs"
  abi_dir="${out_dir}/abi"

  echo "building backend release binary (${platform})"
  (
    cd "${ROOT_DIR}/backend"
    cargo build --release
  )

  src_bin="${ROOT_DIR}/backend/target/release/${BIN_NAME}"
  out_bin="${out_dir}/${BIN_NAME}"
  abi_src="${ROOT_DIR}/contracts/out/TicketSale.sol/TicketSale.json"

  mkdir -p "${docs_dir}" "${abi_dir}"
  cp "${src_bin}" "${out_bin}"
  chmod +x "${out_bin}"

  cp "${ROOT_DIR}/backend/docs/frontend-quickstart.md" "${docs_dir}/frontend-quickstart.md"
  cp "${ROOT_DIR}/backend/docs/openapi.yaml" "${docs_dir}/openapi.yaml"

  if [[ -f "${abi_src}" ]]; then
    cp "${abi_src}" "${abi_dir}/TicketSale.json"
  else
    echo "warning: ABI not found, skipped copy: ${abi_src}" >&2
  fi

  checksum="$(sha256_file "${out_bin}")"
  cat > "${out_dir}/README.txt" <<EOF
Prebuilt backend for frontend local integration (${platform})

Usage in repo root:
  ./scripts/dev-up-prebuilt.sh

Or explicitly:
  BACKEND_BIN=./dist/prebuilt/${platform}/${BIN_NAME} ./scripts/dev-up.sh

Local endpoints after startup:
  RPC: http://127.0.0.1:8545
  Chain ID: 31337
  Backend: http://127.0.0.1:8080

Test wallet (local only):
  Buyer address: 0x70997970C51812dc3A010C7d01b50e0d17dc79C8
  Buyer private key: 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

SHA256 (${BIN_NAME}): ${checksum}
EOF

  archive="${DIST_ROOT}/frontend-kit-${platform}.tar.gz"
  tar -C "${DIST_ROOT}" -czf "${archive}" "${platform}"

  cat <<EOF
frontend kit package ready
platform: ${platform}
binary: ${out_bin}
archive: ${archive}
checksum: ${checksum}
EOF
}

main "$@"
