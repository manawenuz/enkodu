#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="yulia-worker"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ENV_DIR="${HOME}/.config/yulia-worker"
ENV_FILE="${ENV_DIR}/worker.env"
USER_UNIT_DIR="${HOME}/.config/systemd/user"
USER_UNIT_FILE="${USER_UNIT_DIR}/${SERVICE_NAME}.service"

log() {
  printf '\n==> %s\n' "$*"
}

ok() {
  printf '    OK  %s\n' "$*"
}

warn() {
  printf '    WARN %s\n' "$*" >&2
}

fail() {
  printf '    ERROR %s\n' "$*" >&2
  exit 1
}

have_cmd() {
  command -v "$1" >/dev/null 2>&1
}

find_source_binary() {
  local candidate

  if [[ "${1:-}" != "" ]]; then
    printf '%s\n' "$1"
    return 0
  fi

  for candidate in \
    "${YULIA_WORKER_BINARY:-}" \
    "${SCRIPT_DIR}/target/release/yulia-worker" \
    "${SCRIPT_DIR}/yulia-worker" \
    "$(command -v yulia-worker 2>/dev/null || true)"; do
    if [[ -n "$candidate" && -f "$candidate" && -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  return 1
}

build_release_binary() {
  if [[ -f "${SCRIPT_DIR}/Cargo.toml" ]] && have_cmd cargo; then
    printf '\n==> Building release binary\n' >&2
    cargo build --release --manifest-path "${SCRIPT_DIR}/Cargo.toml"
    printf '%s\n' "${SCRIPT_DIR}/target/release/yulia-worker"
    return 0
  fi

  return 1
}

install_binary() {
  local source_binary="$1"
  local target

  if [[ -w /usr/local/bin ]]; then
    target="/usr/local/bin/yulia-worker"
    install -m 0755 "$source_binary" "$target"
    printf '%s\n' "$target"
    return 0
  fi

  if have_cmd sudo && sudo -n true >/dev/null 2>&1; then
    target="/usr/local/bin/yulia-worker"
    sudo install -m 0755 "$source_binary" "$target"
    printf '%s\n' "$target"
    return 0
  fi

  target="${HOME}/bin/yulia-worker"
  mkdir -p "${HOME}/bin"
  install -m 0755 "$source_binary" "$target"
  printf '%s\n' "$target"
}

write_env_file() {
  mkdir -p "$ENV_DIR"

  if [[ -e "$ENV_FILE" ]]; then
    chmod 0600 "$ENV_FILE"
    ok "Existing ${ENV_FILE} kept unchanged"
    return 0
  fi

  umask 077
  cat >"$ENV_FILE" <<EOF
# yulia-worker configuration
# Lines starting with # are comments. Environment variables override these values.

QUEUE_URL=http://172.16.81.137:8090

# Worker bearer token. Must match AUTH_WORKER_TOKEN on the queue server.
# Leave empty if queue auth is disabled.
QUEUE_TOKEN=
# Compatibility alias. QUEUE_TOKEN wins if both are set.
AUTH_WORKER_TOKEN=

WORKER_NAME=$(hostname -s 2>/dev/null || hostname)

FFMPEG_PATH=ffmpeg
FFPROBE_PATH=ffprobe

WORK_DIR=/tmp/yulia-worker/jobs
LOG_DIR=${HOME}/.local/share/yulia-worker/logs
WORKER_ENV_FILE=${ENV_FILE}

# Encoder selection: leave empty for detection, or set one of:
# av1_qsv, av1_vaapi, av1_nvenc, libsvtav1
ENCODER=
ENCODE_QUALITY=28
ENCODE_PRESET=medium
AUDIO_CODEC=aac
AUDIO_BITRATE=192k
VAAPI_DEVICE=/dev/dri/renderD128

POLL_SECS=10
EOF
  chmod 0600 "$ENV_FILE"
  ok "Created ${ENV_FILE} (edit QUEUE_URL and QUEUE_TOKEN if needed)"
}

write_systemd_unit() {
  local binary_path="$1"

  mkdir -p "$USER_UNIT_DIR"
  cat >"$USER_UNIT_FILE" <<EOF
[Unit]
Description=Enkodu AV1 Worker
After=network.target

[Service]
ExecStart=${binary_path}
Restart=on-failure
RestartSec=30
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF
  ok "Wrote ${USER_UNIT_FILE}"
}

main() {
  local source_binary=""
  local installed_binary

  log "Resolving yulia-worker binary"
  if source_binary="$(find_source_binary "${1:-}")"; then
    ok "Using ${source_binary}"
  elif source_binary="$(build_release_binary)"; then
    ok "Built ${source_binary}"
  else
    fail "No yulia-worker binary found. Pass a binary path, set YULIA_WORKER_BINARY, place yulia-worker next to this script, or install Cargo."
  fi

  [[ -x "$source_binary" ]] || fail "Binary is not executable: ${source_binary}"

  log "Installing binary"
  installed_binary="$(install_binary "$source_binary")"
  ok "Installed ${installed_binary}"

  log "Creating config"
  write_env_file

  log "Writing systemd user unit"
  write_systemd_unit "$installed_binary"

  have_cmd systemctl || fail "systemctl not found; install systemd or start ${installed_binary} manually."

  log "Enabling and starting service"
  systemctl --user daemon-reload
  systemctl --user enable --now "${SERVICE_NAME}"
  ok "systemd user service enabled and started"

  log "Running diagnostics"
  if "$installed_binary" diagnostics; then
    ok "Diagnostics passed"
  else
    warn "Diagnostics failed. Edit ${ENV_FILE}, then run: systemctl --user restart ${SERVICE_NAME}"
    exit 1
  fi

  printf '\nInstall complete. Logs: journalctl --user -u %s -f\n' "$SERVICE_NAME"
}

main "$@"
