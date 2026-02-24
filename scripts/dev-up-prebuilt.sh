#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

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

main() {
  local platform
  local default_backend_bin
  local backend_bin

  platform="$(normalize_os)-$(normalize_arch)"
  default_backend_bin="${ROOT_DIR}/dist/prebuilt/${platform}/ticket-backend"
  backend_bin="${BACKEND_BIN:-${default_backend_bin}}"

  if [[ ! -x "${backend_bin}" ]]; then
    cat >&2 <<EOF
missing prebuilt backend binary: ${backend_bin}

options:
1. ask backend team for dist/prebuilt/${platform}/ticket-backend
2. build locally with Rust: (cd backend && cargo build --release)
3. run ./scripts/package-frontend-kit.sh on a machine with Rust
EOF
    exit 1
  fi

  BACKEND_BIN="${backend_bin}" "${ROOT_DIR}/scripts/dev-up.sh" "$@"
}

main "$@"
