#!/usr/bin/env bash
# One-line installer for the Hermes iroh bridge (publish-time entry point).
#
#   curl -fsSL https://<host>/iroh-bridge/install.sh | bash
#
# Fetches a prebuilt hermes-bridge binary for this platform (or builds from
# source if Rust is present), then launches it against the local dashboard.
set -euo pipefail

PREFIX="${HERMES_BRIDGE_PREFIX:-$HOME/.hermes-bridge}"
BIN="$PREFIX/hermes-bridge"
# TODO(publish): point at the GitHub release once the repo goes public.
RELEASE_URL="${HERMES_BRIDGE_RELEASE_URL:-}"

mkdir -p "$PREFIX"

fetch_prebuilt() {
  [ -n "$RELEASE_URL" ] || return 1
  local os arch asset
  os="$(uname -s)"; arch="$(uname -m)"
  asset="hermes-bridge-${os}-${arch}"
  echo "→ downloading $asset…"
  curl -fsSL "$RELEASE_URL/$asset" -o "$BIN" && chmod +x "$BIN"
}

build_from_source() {
  command -v cargo >/dev/null || { echo "✗ no prebuilt binary and Rust/cargo not found"; return 1; }
  echo "→ building from source…"
  local src="$PREFIX/src"
  if [ ! -d "$src/.git" ]; then
    git clone --depth 1 "${HERMES_BRIDGE_REPO:-https://github.com/REPLACE/iroh-hermes-bridge}" "$src"
  fi
  ( cd "$src/bridge" && cargo build --release )
  cp "$src/bridge/target/release/hermes-bridge" "$BIN"
}

if [ ! -x "$BIN" ]; then
  fetch_prebuilt || build_from_source
fi

echo "✓ installed at $BIN"
exec "$PREFIX/run-bridge.sh" 2>/dev/null || exec "$BIN"
