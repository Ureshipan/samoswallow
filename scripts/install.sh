#!/bin/sh
# samoswallow installer.
# Installs the swallowd binary, a systemd unit, config and state dirs, and
# ensures Docker + Caddy are present. Designed to be re-runnable (idempotent).
#
# Usage:  sudo ./scripts/install.sh [/path/to/swallowd]
# If no binary path is given, falls back to ./target/release/swallowd.
set -eu

PREFIX="${PREFIX:-/usr/local/bin}"
CONFIG_DIR="/etc/samoswallow"
STATE_DIR="/var/lib/samoswallow"
SERVICE="/etc/systemd/system/swallowd.service"
BIN_SRC="${1:-./target/release/swallowd}"

log() { printf '\033[1;32m[samoswallow]\033[0m %s\n' "$*"; }
err() { printf '\033[1;31m[samoswallow]\033[0m %s\n' "$*" >&2; }

if [ "$(id -u)" -ne 0 ]; then
    err "must run as root (use sudo)"
    exit 1
fi

# --- dependencies ----------------------------------------------------------
if ! command -v docker >/dev/null 2>&1; then
    log "Docker not found — installing via get.docker.com"
    curl -fsSL https://get.docker.com | sh
fi

if ! command -v caddy >/dev/null 2>&1; then
    err "Caddy not found. Install it from https://caddyserver.com/docs/install"
    err "then re-run this script. (Auto-install will be added later.)"
fi

# --- binary ----------------------------------------------------------------
if [ ! -f "$BIN_SRC" ]; then
    err "binary not found at '$BIN_SRC'"
    err "build it first:  cargo build --release"
    exit 1
fi
log "installing binary to $PREFIX/swallowd"
install -Dm755 "$BIN_SRC" "$PREFIX/swallowd"

# --- directories & config --------------------------------------------------
mkdir -p "$CONFIG_DIR" "$STATE_DIR"
if [ ! -f "$CONFIG_DIR/swallowd.env" ]; then
    log "writing default config to $CONFIG_DIR/swallowd.env"
    cat > "$CONFIG_DIR/swallowd.env" <<EOF
SWALLOW_LISTEN=127.0.0.1:8080
SWALLOW_DATABASE_URL=sqlite://$STATE_DIR/state.db?mode=rwc
SWALLOW_BASE_DOMAIN=localhost
SWALLOW_LOG=info
# Set a password for the web UI (user: admin). If left unset, a random one is
# generated on first start and printed to the journal (journalctl -u swallowd).
#SWALLOW_ADMIN_PASSWORD=change-me
EOF
fi

# --- service ---------------------------------------------------------------
log "installing systemd unit"
cp packaging/swallowd.service "$SERVICE"
systemctl daemon-reload
systemctl enable --now swallowd

log "done. Check status with:  systemctl status swallowd"
log "API:  curl http://127.0.0.1:8080/healthz"
