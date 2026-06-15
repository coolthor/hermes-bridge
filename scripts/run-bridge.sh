#!/usr/bin/env bash
# Run hermes-bridge against the local Hermes dashboard, and KEEP it pointed at
# the right port — the Hermes Desktop dashboard picks a new port on every
# restart, so we supervise and re-launch the bridge when the port moves. The
# bridge has a persistent key (stable NodeId), so the phone reconnects with no
# re-scan.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$ROOT/target/release/hermes-bridge"
# Keep logs + QR in the owner-only bridge dir, NOT world-readable /tmp.
BRIDGE_DIR="$HOME/.hermes-bridge"
mkdir -p "$BRIDGE_DIR"; chmod 700 "$BRIDGE_DIR" 2>/dev/null || true
LOG="$BRIDGE_DIR/bridge.log"; : > "$LOG"; chmod 600 "$LOG" 2>/dev/null
QR_PNG="$BRIDGE_DIR/qr.png"

OPEN_QR=0
for a in "$@"; do [ "$a" = "--open" ] && OPEN_QR=1; done

open_qr() {
  [ "$OPEN_QR" = "1" ] && [ -f "$QR_PNG" ] || return 0
  if command -v open >/dev/null 2>&1; then open "$QR_PNG"
  elif command -v xdg-open >/dev/null 2>&1; then xdg-open "$QR_PNG"
  fi
}

if [ ! -x "$BIN" ]; then
  echo "building hermes-bridge…"
  ( cd "$ROOT" && cargo build --release )
fi

dashboard_pid() { pgrep -f 'hermes_cli.main dashboard' | head -1; }

current_port() {
  local dpid; dpid="$(dashboard_pid)"; [ -n "$dpid" ] || return 1
  local p
  for p in $(lsof -nP -p "$dpid" -iTCP -sTCP:LISTEN 2>/dev/null | grep -oE '127\.0\.0\.1:[0-9]+' | cut -d: -f2 | sort -u); do
    if curl -s -m2 "http://127.0.0.1:$p/api/status" 2>/dev/null | grep -q '"version"'; then
      echo "$p"; return 0
    fi
  done
  return 1
}

current_token() {
  local dpid; dpid="$(dashboard_pid)"; [ -n "$dpid" ] || return 1
  ps eww "$dpid" 2>/dev/null | tr ' ' '\n' | grep '^HERMES_DASHBOARD_SESSION_TOKEN=' | cut -d= -f2
}

start_bridge() {
  local port="$1" token="$2"
  pkill -x hermes-bridge 2>/dev/null; sleep 1
  env HERMES_DASHBOARD="127.0.0.1:$port" HERMES_SESSION_TOKEN="$token" nohup "$BIN" > "$LOG" 2>&1 &
}

PORT="$(current_port)" || {
  echo "✗ Hermes dashboard not running."
  echo "  Open Hermes Desktop, OR (no Desktop?) run:  hermes dashboard"
  exit 1
}
TOKEN="$(current_token)"
echo "→ dashboard at 127.0.0.1:$PORT  (token ${TOKEN:0:6}…)"
start_bridge "$PORT" "$TOKEN"

# Wait for the bridge to come up (readiness marker carries NO secret — the
# pairing code is never written to the log).
for _ in $(seq 1 20); do grep -q 'HERMES BRIDGE READY' "$LOG" 2>/dev/null && break; sleep 1; done
if grep -q 'HERMES BRIDGE READY' "$LOG" 2>/dev/null; then
  echo "✓ bridge running → 127.0.0.1:$PORT"
  echo "✓ pairing window open — QR: $QR_PNG"
  open_qr
else
  echo "✗ bridge failed to start — see $LOG"; cat "$LOG"; exit 1
fi

# Supervisor (forked to background): follow the dashboard port across Hermes
# Desktop restarts, so the script still returns the QR for the agent flow.
pkill -f 'hermes-bridge-supervisor' 2>/dev/null || true
(
  exec -a hermes-bridge-supervisor bash -c '
    PORT="'"$PORT"'"; ROOT="'"$ROOT"'"; BIN="'"$BIN"'"; LOG="'"$LOG"'"
    dashboard_pid() { pgrep -f "hermes_cli.main dashboard" | head -1; }
    current_port() { local d p; d="$(dashboard_pid)"; [ -n "$d" ] || return 1; for p in $(lsof -nP -p "$d" -iTCP -sTCP:LISTEN 2>/dev/null | grep -oE "127\.0\.0\.1:[0-9]+" | cut -d: -f2 | sort -u); do curl -s -m2 "http://127.0.0.1:$p/api/status" 2>/dev/null | grep -q "\"version\"" && { echo "$p"; return 0; }; done; return 1; }
    current_token() { local d; d="$(dashboard_pid)"; ps eww "$d" 2>/dev/null | tr " " "\n" | grep "^HERMES_DASHBOARD_SESSION_TOKEN=" | cut -d= -f2; }
    while true; do
      sleep 15
      np="$(current_port)" || continue
      if [ "$np" != "$PORT" ] || ! pgrep -x hermes-bridge >/dev/null; then
        PORT="$np"; tok="$(current_token)"
        pkill -x hermes-bridge 2>/dev/null; sleep 1
        env HERMES_DASHBOARD="127.0.0.1:$PORT" HERMES_SESSION_TOKEN="$tok" nohup "$BIN" >> "$LOG" 2>&1 &
        echo "⟳ $(date +%H:%M:%S) dashboard moved → $PORT, bridge restarted" >> "$LOG"
      fi
    done
  '
) >/dev/null 2>&1 &
disown 2>/dev/null || true
echo "→ supervisor running (auto-follows dashboard port)"
