#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

REMOTE_HOST="${REMOTE_HOST:-ec2-54-248-46-44.ap-northeast-1.compute.amazonaws.com}"
REMOTE_USER="${REMOTE_USER:-ubuntu}"
REMOTE_PORT="${REMOTE_PORT:-22}"
REMOTE_DIR="${REMOTE_DIR:-/home/ubuntu/app/tickets}"
REMOTE_DATA_DIR="${REMOTE_DATA_DIR:-${REMOTE_DIR}/data}"
REMOTE_BACKEND_BIN_NAME="${REMOTE_BACKEND_BIN_NAME:-ticket-backend-1}"

BACKEND_BIN="${BACKEND_BIN:-${ROOT_DIR}/dist/prebuilt/linux-amd64/ticket-backend}"
BACKEND_ENV_FILE="${BACKEND_ENV_FILE:-${ROOT_DIR}/backend/.env}"
UPLOAD_ENV="${UPLOAD_ENV:-0}"

SYSTEMD_UNIT_FILE="${SYSTEMD_UNIT_FILE:-}"
UPLOAD_SYSTEMD_UNIT="${UPLOAD_SYSTEMD_UNIT:-0}"

UPLOAD_DB="${UPLOAD_DB:-0}"
DB_FILE="${DB_FILE:-}"
REMOTE_DB_FILE="${REMOTE_DB_FILE:-${REMOTE_DATA_DIR}/ticket.db}"

SSH_IDENTITY_FILE="${SSH_IDENTITY_FILE:-}"
DRY_RUN="${DRY_RUN:-0}"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing command: $1" >&2
    exit 1
  fi
}

usage() {
  cat <<'EOF'
Upload backend binary and optional runtime files to remote server via scp.

Optional:
  REMOTE_HOST                Target host (default: ec2-54-248-46-44.ap-northeast-1.compute.amazonaws.com)
  REMOTE_USER                SSH user (default: ubuntu)
  REMOTE_PORT                SSH port (default: 22)
  REMOTE_DIR                 Remote app directory (default: /home/ubuntu/app/tickets)
  REMOTE_DATA_DIR            Remote data directory (default: $REMOTE_DIR/data)
  REMOTE_BACKEND_BIN_NAME    Remote backend binary filename (default: ticket-backend-1)
  BACKEND_BIN                Local backend binary path (default: ./dist/prebuilt/linux-amd64/ticket-backend)
  BACKEND_ENV_FILE           Local env file path
  UPLOAD_ENV                 1 to upload env file, else 0 (default: 0)
  UPLOAD_DB                  1 to upload sqlite db file, else 0 (default: 0)
  DB_FILE                    Local sqlite db file path (auto-resolve from BACKEND_ENV_FILE if empty)
  REMOTE_DB_FILE             Remote sqlite db file path (default: $REMOTE_DATA_DIR/ticket.db)
  UPLOAD_SYSTEMD_UNIT        1 to upload systemd unit file, else 0 (default)
  SYSTEMD_UNIT_FILE          Local systemd unit file path (required if UPLOAD_SYSTEMD_UNIT=1)
  SSH_IDENTITY_FILE          SSH private key path
  DRY_RUN                    1 to print commands only, else 0 (default)

Example:
  ./scripts/deploy-backend-scp.sh
EOF
}

resolve_db_file() {
  if [[ -n "${DB_FILE}" ]]; then
    return
  fi

  if [[ ! -f "${BACKEND_ENV_FILE}" ]]; then
    return
  fi

  local database_url
  database_url="$(sed -n 's/^DATABASE_URL=//p' "${BACKEND_ENV_FILE}" | tail -n1)"
  database_url="${database_url%\'}"
  database_url="${database_url#\'}"
  if [[ -z "${database_url}" ]]; then
    return
  fi

  local sqlite_path=""
  if [[ "${database_url}" == sqlite://* ]]; then
    sqlite_path="${database_url#sqlite://}"
  elif [[ "${database_url}" == sqlite:* ]]; then
    sqlite_path="${database_url#sqlite:}"
  fi
  sqlite_path="${sqlite_path%%\?*}"
  sqlite_path="${sqlite_path%/}"

  if [[ -z "${sqlite_path}" || "${sqlite_path}" == ":memory:" || "${sqlite_path}" == "memory:" ]]; then
    return
  fi

  if [[ "${sqlite_path}" == /* ]]; then
    DB_FILE="${sqlite_path}"
    return
  fi

  local env_dir
  env_dir="$(cd "$(dirname "${BACKEND_ENV_FILE}")" && pwd)"
  DB_FILE="${env_dir}/${sqlite_path}"
}

run_cmd() {
  if [[ "${DRY_RUN}" == "1" ]]; then
    echo "[dry-run] $*"
    return
  fi
  "$@"
}

main() {
  require_cmd ssh
  require_cmd scp

  if [[ -z "${REMOTE_HOST}" ]]; then
    echo "REMOTE_HOST is required" >&2
    usage
    exit 1
  fi

  if [[ ! -f "${BACKEND_BIN}" ]]; then
    echo "backend binary not found: ${BACKEND_BIN}" >&2
    exit 1
  fi

  if [[ -z "${REMOTE_BACKEND_BIN_NAME}" || "${REMOTE_BACKEND_BIN_NAME}" == */* ]]; then
    echo "REMOTE_BACKEND_BIN_NAME must be a non-empty filename without slashes" >&2
    exit 1
  fi

  if [[ "${UPLOAD_ENV}" == "1" && ! -f "${BACKEND_ENV_FILE}" ]]; then
    echo "backend env file not found: ${BACKEND_ENV_FILE}" >&2
    exit 1
  fi

  if [[ "${UPLOAD_SYSTEMD_UNIT}" == "1" && ! -f "${SYSTEMD_UNIT_FILE}" ]]; then
    echo "SYSTEMD_UNIT_FILE is required and must exist when UPLOAD_SYSTEMD_UNIT=1" >&2
    exit 1
  fi

  if [[ "${UPLOAD_DB}" == "1" ]]; then
    resolve_db_file
    if [[ -z "${DB_FILE}" ]]; then
      echo "failed to resolve DB_FILE from BACKEND_ENV_FILE, set DB_FILE explicitly" >&2
      exit 1
    fi
    if [[ ! -f "${DB_FILE}" ]]; then
      echo "db file not found: ${DB_FILE}" >&2
      exit 1
    fi
  fi

  local ssh_target="${REMOTE_USER}@${REMOTE_HOST}"
  local remote_backend_bin="${REMOTE_DIR}/${REMOTE_BACKEND_BIN_NAME}"
  local ssh_opts=(-p "${REMOTE_PORT}" -o StrictHostKeyChecking=accept-new)
  local scp_opts=(-P "${REMOTE_PORT}" -o StrictHostKeyChecking=accept-new)

  if [[ -n "${SSH_IDENTITY_FILE}" ]]; then
    ssh_opts+=(-i "${SSH_IDENTITY_FILE}")
    scp_opts+=(-i "${SSH_IDENTITY_FILE}")
  fi

  echo "target: ${ssh_target}"
  echo "remote dir: ${REMOTE_DIR}"
  echo "remote binary: ${remote_backend_bin}"
  echo "binary: ${BACKEND_BIN}"
  if [[ "${UPLOAD_ENV}" == "1" ]]; then
    echo "env: ${BACKEND_ENV_FILE}"
  fi
  if [[ "${UPLOAD_DB}" == "1" ]]; then
    echo "db: ${DB_FILE}"
  fi

  run_cmd ssh "${ssh_opts[@]}" "${ssh_target}" \
    "mkdir -p '${REMOTE_DIR}' '${REMOTE_DATA_DIR}'"

  run_cmd scp "${scp_opts[@]}" "${BACKEND_BIN}" "${ssh_target}:${remote_backend_bin}"
  if [[ "${UPLOAD_ENV}" == "1" ]]; then
    run_cmd scp "${scp_opts[@]}" "${BACKEND_ENV_FILE}" "${ssh_target}:${REMOTE_DIR}/backend.env"
  fi
  if [[ "${UPLOAD_DB}" == "1" ]]; then
    run_cmd scp "${scp_opts[@]}" "${DB_FILE}" "${ssh_target}:${REMOTE_DB_FILE}"
  fi

  run_cmd ssh "${ssh_opts[@]}" "${ssh_target}" \
    "chmod +x '${remote_backend_bin}'"

  if [[ "${UPLOAD_SYSTEMD_UNIT}" == "1" ]]; then
    run_cmd scp "${scp_opts[@]}" "${SYSTEMD_UNIT_FILE}" "${ssh_target}:${REMOTE_DIR}/ticket-backend.service"
    echo "systemd unit uploaded to: ${REMOTE_DIR}/ticket-backend.service"
  fi

  echo "upload complete"
  echo "next steps on server:"
  echo "  cd ${REMOTE_DIR}"
  if [[ "${UPLOAD_ENV}" == "1" ]]; then
    echo "  set -a && source ./backend.env && set +a"
  else
    echo "  source the existing backend env for this deployment"
  fi
  echo "  ./${REMOTE_BACKEND_BIN_NAME}"
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

main "$@"
