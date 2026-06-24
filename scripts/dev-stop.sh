#!/bin/sh
# Dev helper: stop the background swallowd started by dev-start.sh.
set -eu

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEV_DIR="$ROOT/.dev"
PID_FILE="$DEV_DIR/swallowd.pid"

log() { printf '\033[1;32m[dev]\033[0m %s\n' "$*"; }

if [ ! -f "$PID_FILE" ]; then
    log "no dev instance recorded (nothing to stop)."
    exit 0
fi

PID="$(cat "$PID_FILE")"
if kill -0 "$PID" 2>/dev/null; then
    log "stopping swallowd (pid $PID)"
    kill "$PID" 2>/dev/null || true
    # Give it a moment, then force if still alive.
    sleep 1
    kill -0 "$PID" 2>/dev/null && kill -9 "$PID" 2>/dev/null || true
    log "stopped."
else
    log "recorded pid $PID is not running."
fi
rm -f "$PID_FILE"
