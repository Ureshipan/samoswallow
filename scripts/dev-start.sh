#!/bin/sh
# Dev helper: build and start swallowd in the background for local hacking.
# State, PID and logs live under ./.dev (gitignored). Re-running restarts cleanly.
#
# Override via env, e.g.:  SWALLOW_LISTEN=127.0.0.1:9000 ./scripts/dev-start.sh
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEV_DIR="$ROOT/.dev"
PID_FILE="$DEV_DIR/swallowd.pid"
LOG_FILE="$DEV_DIR/swallowd.log"

# Dev defaults (port 8088 to avoid clashing with whatever uses 8080).
: "${SWALLOW_LISTEN:=127.0.0.1:8088}"
: "${SWALLOW_DATABASE_URL:=sqlite://$DEV_DIR/state.db?mode=rwc}"
: "${SWALLOW_STATE_DIR:=$DEV_DIR/state}"
: "${SWALLOW_BASE_DOMAIN:=lvh.me}"
: "${SWALLOW_LOG:=info}"
export SWALLOW_LISTEN SWALLOW_DATABASE_URL SWALLOW_STATE_DIR SWALLOW_BASE_DOMAIN SWALLOW_LOG

log() { printf '\033[1;32m[dev]\033[0m %s\n' "$*"; }
err() { printf '\033[1;31m[dev]\033[0m %s\n' "$*" >&2; }

mkdir -p "$DEV_DIR"

# Stop a previous dev instance if one is recorded and still alive.
if [ -f "$PID_FILE" ]; then
    OLD="$(cat "$PID_FILE")"
    if kill -0 "$OLD" 2>/dev/null; then
        log "stopping previous dev instance (pid $OLD)"
        kill "$OLD" 2>/dev/null || true
        sleep 1
    fi
    rm -f "$PID_FILE"
fi

# Warn early if the chosen port is taken by something else.
PORT="${SWALLOW_LISTEN##*:}"
if ss -ltn 2>/dev/null | grep -q ":$PORT "; then
    err "port $PORT is already in use by another process."
    err "pick another:  SWALLOW_LISTEN=127.0.0.1:9000 $0"
    exit 1
fi

log "building (debug)…"
cargo build -p swallowd --quiet

log "starting swallowd on http://$SWALLOW_LISTEN"
nohup "$ROOT/target/debug/swallowd" >"$LOG_FILE" 2>&1 &
echo $! > "$PID_FILE"
sleep 1

if kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
    log "running (pid $(cat "$PID_FILE")). Logs: $LOG_FILE"
    log "open:  http://$SWALLOW_LISTEN/"
    log "stop:  ./scripts/dev-stop.sh"
else
    err "failed to start — last log lines:"
    tail -n 20 "$LOG_FILE" >&2 || true
    exit 1
fi
