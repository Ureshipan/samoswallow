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

install_caddy() {
    if command -v caddy >/dev/null 2>&1; then
        return 0
    fi
    log "Caddy not found — installing"
    if command -v apk >/dev/null 2>&1; then
        apk add --no-cache caddy && return 0
    elif command -v apt-get >/dev/null 2>&1; then
        apt-get update -qq && apt-get install -y caddy && return 0
    elif command -v dnf >/dev/null 2>&1; then
        dnf install -y caddy && return 0
    fi
    # Fallback: official static build for the current architecture.
    arch="$(uname -m)"
    case "$arch" in
        x86_64|amd64) carch="amd64" ;;
        aarch64|arm64) carch="arm64" ;;
        *) err "unsupported arch '$arch' for Caddy auto-install"; return 1 ;;
    esac
    log "downloading static Caddy ($carch)"
    curl -fsSL "https://caddyserver.com/api/download?os=linux&arch=$carch" -o /usr/local/bin/caddy
    chmod +x /usr/local/bin/caddy
}

if ! install_caddy; then
    err "could not install Caddy automatically — install it manually and re-run"
    err "https://caddyserver.com/docs/install"
    exit 1
fi

# Disable a distro-shipped caddy.service so it doesn't fight for ports 80/443.
if systemctl list-unit-files 2>/dev/null | grep -q '^caddy.service'; then
    log "disabling distro caddy.service (samoswallow runs its own)"
    systemctl disable --now caddy 2>/dev/null || true
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

# --- services --------------------------------------------------------------
log "installing systemd units"
cp packaging/swallowd.service "$SERVICE"
cp packaging/swallow-caddy.service /etc/systemd/system/swallow-caddy.service
systemctl daemon-reload
systemctl enable --now swallow-caddy
systemctl enable --now swallowd

log "done. Check status with:  systemctl status swallowd"
log "API:  curl http://127.0.0.1:8080/healthz"
