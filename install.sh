#!/usr/bin/env bash
# One-shot installer for the Hermes phone bridge.
#
# Easiest path — paste this into your own Hermes:
#   "Clone https://github.com/coolthor/hermes-bridge into ~/hermes-bridge,
#    run its install.sh, then show me the pairing QR so I can connect my phone."
#
# Or run it yourself:
#   git clone https://github.com/coolthor/hermes-bridge ~/hermes-bridge
#   bash ~/hermes-bridge/install.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HERMES_HOME="${HERMES_HOME:-$HOME/.hermes}"
SKILLS_DIR="$HERMES_HOME/skills"

echo "→ Hermes phone bridge installer"
echo "  repo:       $ROOT"
echo "  skills dir: $SKILLS_DIR"

# 1. Build the bridge (needs the Rust toolchain).
if [ ! -x "$ROOT/target/release/hermes-bridge" ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "✗ Rust toolchain not found. Install it from https://rustup.rs then re-run."
    exit 1
  fi
  echo "→ building hermes-bridge (first build can take a few minutes)…"
  ( cd "$ROOT" && cargo build --release ) || { echo "✗ build failed"; exit 1; }
fi
echo "✓ bridge binary ready"

# 2. Install the connect-phone skill (symlink → updates follow git pull).
mkdir -p "$SKILLS_DIR"
ln -sfn "$ROOT/skills/connect-phone" "$SKILLS_DIR/connect-phone"
echo "✓ skill installed → $SKILLS_DIR/connect-phone"

# 3. Show the pairing QR now, so the very first run is end-to-end.
echo "→ starting bridge + showing pairing QR…"
"$ROOT/scripts/run-bridge.sh" --open || true

cat <<'EOF'

✓ Done. On your phone:
    open HermesApp → 連線 → 掃描 QR Code → scan the QR on screen.
Next time, just tell your Hermes: 「連接手機」 and it shows the QR again.
EOF
