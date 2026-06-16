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

# Device-management subcommands (1vN allowlist):
#   approve <code>  — confirm a pending phone by the code shown on its screen
#   list            — show paired devices (NodeId prefixes)
#   revoke <id|all> — remove a paired device (by NodeId prefix) or wipe all
# The bridge wrote `<fingerprint> <NodeId>` lines to pending when a phone PAIRed;
# `approve` maps the operator-typed code back to its NodeId. This human
# confirmation defeats a first-scanner — you only type the code on YOUR phone.
PENDING="$BRIDGE_DIR/pending"; ALLOWED="$BRIDGE_DIR/allowed"
case "${1:-}" in
  approve)
    CODE="$(printf '%s' "${2:-}" | tr -d '[:space:]-' | tr '[:lower:]' '[:upper:]')"
    [ -n "$CODE" ] || { echo "usage: run-bridge.sh approve <code>"; exit 1; }
    N="$(awk -v c="$CODE" 'toupper($1)==c' "$PENDING" 2>/dev/null | wc -l | tr -d ' ')"
    if [ "$N" = "0" ]; then
      echo "✗ no pending device with code $CODE — has the phone scanned yet? (code may have expired)"; exit 1
    elif [ "$N" != "1" ]; then
      echo "✗ $N devices share code $CODE — possible attack. Re-show the QR and re-pair instead of approving."; exit 1
    fi
    NODE="$(awk -v c="$CODE" 'toupper($1)==c {print $2; exit}' "$PENDING")"
    touch "$ALLOWED"; chmod 600 "$ALLOWED" 2>/dev/null
    grep -qxF "$NODE" "$ALLOWED" 2>/dev/null || echo "$NODE" >> "$ALLOWED"
    awk -v c="$CODE" 'toupper($1)!=c' "$PENDING" > "$PENDING.tmp" 2>/dev/null && mv "$PENDING.tmp" "$PENDING"
    echo "✓ approved device (code $CODE) — your phone will connect on its next retry."
    exit 0 ;;
  list)
    if [ ! -s "$ALLOWED" ]; then echo "(no paired devices)"; exit 0; fi
    echo "paired devices:"; n=0
    while IFS= read -r id; do [ -n "$id" ] || continue; n=$((n+1)); echo "  $n. ${id:0:16}…"; done < "$ALLOWED"
    exit 0 ;;
  revoke)
    ARG="${2:-}"; [ -n "$ARG" ] || { echo "usage: run-bridge.sh revoke <nodeid-prefix|all>"; exit 1; }
    if [ "$ARG" = "all" ]; then : > "$ALLOWED"; chmod 600 "$ALLOWED" 2>/dev/null; echo "✓ revoked all devices."; exit 0; fi
    if grep -q "^$ARG" "$ALLOWED" 2>/dev/null; then
      # NB: grep -v exits non-zero when it removes the LAST line (no output left),
      # so don't gate the mv on its exit code or the file won't update.
      grep -v "^$ARG" "$ALLOWED" > "$ALLOWED.tmp" 2>/dev/null || true
      mv "$ALLOWED.tmp" "$ALLOWED"; chmod 600 "$ALLOWED" 2>/dev/null
      echo "✓ revoked device(s) matching $ARG."
    else
      echo "✗ no device matching $ARG (use 'list' to see prefixes)."
    fi
    exit 0 ;;
esac

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

# Find the REAL dashboard — exclude our own supervisor, whose script text also
# contains 'hermes_cli.main dashboard' and would otherwise self-match.
dashboard_pid() {
  local p
  for p in $(pgrep -f 'hermes_cli.main dashboard'); do
    ps -o command= -p "$p" 2>/dev/null | grep -q 'hermes-bridge-supervisor' && continue
    echo "$p"; return 0
  done
  return 1
}

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
  # Prefer our own inherited env: when the AGENT (running inside the dashboard
  # process) invokes this skill, the token is already in the environment — no
  # cross-process read needed (the agent's shell sandbox may block `ps eww` from
  # reading another process's env, which silently yields an empty token).
  [ -n "${HERMES_DASHBOARD_SESSION_TOKEN:-}" ] && { printf '%s' "$HERMES_DASHBOARD_SESSION_TOKEN"; return 0; }
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
# Desktop restarts. Start it ONLY if one isn't already running — the agent's
# sandbox can block `pkill`, so we must NOT rely on killing old ones; instead
# never spawn a duplicate. Duplicates pile up and fight over the bridge, churning
# the pairing code (every restart = new code → the scanned QR goes stale →
# "ERR unauthorized"). A single long-lived supervisor adapts to port changes itself.
if pgrep -f 'hermes-bridge-supervisor' >/dev/null 2>&1; then
  echo "→ supervisor already running (reused — not spawning a duplicate)"
else
(
  exec -a hermes-bridge-supervisor bash -c '
    PORT="'"$PORT"'"; ROOT="'"$ROOT"'"; BIN="'"$BIN"'"; LOG="'"$LOG"'"
    dashboard_pid() { local p; for p in $(pgrep -f "hermes_cli.main dashboard"); do ps -o command= -p "$p" 2>/dev/null | grep -q "hermes-bridge-supervisor" && continue; echo "$p"; return 0; done; return 1; }
    current_port() { local d p; d="$(dashboard_pid)"; [ -n "$d" ] || return 1; for p in $(lsof -nP -p "$d" -iTCP -sTCP:LISTEN 2>/dev/null | grep -oE "127\.0\.0\.1:[0-9]+" | cut -d: -f2 | sort -u); do curl -s -m2 "http://127.0.0.1:$p/api/status" 2>/dev/null | grep -q "\"version\"" && { echo "$p"; return 0; }; done; return 1; }
    current_token() { [ -n "${HERMES_DASHBOARD_SESSION_TOKEN:-}" ] && { printf "%s" "$HERMES_DASHBOARD_SESSION_TOKEN"; return 0; }; local d; d="$(dashboard_pid)"; ps eww "$d" 2>/dev/null | tr " " "\n" | grep "^HERMES_DASHBOARD_SESSION_TOKEN=" | cut -d= -f2; }
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
fi
