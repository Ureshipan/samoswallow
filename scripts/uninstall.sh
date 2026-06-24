#!/bin/sh
# samoswallow uninstaller.
#
# Usage:
#   sudo ./scripts/uninstall.sh           # remove binary, service, config
#   sudo ./scripts/uninstall.sh --purge   # also delete state (SQLite db, repos)
#
# Note: this does NOT remove Docker or Caddy — they may be used by other things.
set -eu

PREFIX="${PREFIX:-/usr/local/bin}"
CONFIG_DIR="/etc/samoswallow"
STATE_DIR="/var/lib/samoswallow"
SERVICE="/etc/systemd/system/swallowd.service"
PURGE=0

[ "${1:-}" = "--purge" ] && PURGE=1

log() { printf '\033[1;32m[samoswallow]\033[0m %s\n' "$*"; }
err() { printf '\033[1;31m[samoswallow]\033[0m %s\n' "$*" >&2; }

if [ "$(id -u)" -ne 0 ]; then
    err "must run as root (use sudo)"
    exit 1
fi

if systemctl list-unit-files | grep -q '^swallowd.service'; then
    log "stopping and disabling service"
    systemctl disable --now swallowd || true
    rm -f "$SERVICE"
    systemctl daemon-reload
fi

log "removing binary"
rm -f "$PREFIX/swallowd"

log "removing config"
rm -rf "$CONFIG_DIR"

if [ "$PURGE" -eq 1 ]; then
    log "purging state at $STATE_DIR"
    rm -rf "$STATE_DIR"
else
    log "keeping state at $STATE_DIR (use --purge to remove)"
fi

log "done."
