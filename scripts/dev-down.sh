#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEV_DIR="${ROOT_DIR}/.dev/local"
ANVIL_PID_FILE="${DEV_DIR}/anvil.pid"
BACKEND_PID_FILE="${DEV_DIR}/backend.pid"

process_alive() {
  local pid="$1"
  kill -0 "${pid}" >/dev/null 2>&1
}

stop_from_pid_file() {
  local name="$1"
  local pid_file="$2"

  if [[ ! -f "${pid_file}" ]]; then
    echo "${name}: no pid file"
    return
  fi

  local pid
  pid="$(cat "${pid_file}")"

  if process_alive "${pid}"; then
    echo "stopping ${name} (pid=${pid})"
    kill "${pid}" || true
  else
    echo "${name}: process already stopped"
  fi

  rm -f "${pid_file}"
}

main() {
  stop_from_pid_file "backend" "${BACKEND_PID_FILE}"
  stop_from_pid_file "anvil" "${ANVIL_PID_FILE}"
}

main "$@"
