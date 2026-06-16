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

# An agent's shell is often non-login with a minimal PATH that omits Homebrew /
# rustup. Make an already-installed cargo findable before we decide we need one.
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HERMES_HOME="${HERMES_HOME:-$HOME/.hermes}"
SKILLS_DIR="$HERMES_HOME/skills"

echo "→ Hermes phone bridge installer"
echo "  repo:       $ROOT"
echo "  skills dir: $SKILLS_DIR"

# 1. Get the bridge binary: prefer a prebuilt release asset (no Rust needed),
#    fall back to building from source.
BIN="$ROOT/target/release/hermes-bridge"

os_tag() { case "$(uname -s)" in Darwin) echo darwin;; Linux) echo linux;; *) echo unknown;; esac; }
arch_tag() { case "$(uname -m)" in arm64|aarch64) echo arm64;; x86_64|amd64) echo x86_64;; *) echo unknown;; esac; }

repo_slug() {
  git -C "$ROOT" remote get-url origin 2>/dev/null \
    | sed -E 's#.*github\.com[:/]##; s#\.git$##' \
    | grep -E '^[^/]+/[^/]+$' || echo "coolthor/hermes-bridge"
}

download_prebuilt() {
  local asset="hermes-bridge-$(os_tag)-$(arch_tag)"
  local slug; slug="$(repo_slug)"
  [ "$asset" = "hermes-bridge-unknown-unknown" ] && return 1
  mkdir -p "$(dirname "$BIN")"
  echo "→ fetching prebuilt $asset from $slug..."
  # gh works for private + public repos; plain curl is the no-gh public path.
  if command -v gh >/dev/null 2>&1 \
     && gh release download --repo "$slug" --pattern "$asset" --output "$BIN" --clobber 2>/dev/null; then
    chmod +x "$BIN"; return 0
  fi
  if curl -fsSL "https://github.com/$slug/releases/latest/download/$asset" -o "$BIN" 2>/dev/null \
     && [ -s "$BIN" ]; then
    chmod +x "$BIN"; return 0
  fi
  rm -f "$BIN"; return 1
}

if [ ! -x "$BIN" ]; then
  if download_prebuilt; then
    echo "✓ installed prebuilt binary (no build needed)"
  elif command -v cargo >/dev/null 2>&1; then
    echo "→ no prebuilt for this platform — building from source (first build takes a few minutes)..."
    ( cd "$ROOT" && cargo build --release ) || { echo "✗ build failed"; exit 1; }
  else
    echo "✗ No prebuilt binary for this platform and no Rust toolchain to build one."
    echo "  Install Rust from https://rustup.rs and re-run, or open an issue for a prebuilt."
    exit 1
  fi
fi
echo "✓ bridge binary ready"

# 2. Install the connect-phone skill (symlink → updates follow git pull).
mkdir -p "$SKILLS_DIR"
ln -sfn "$ROOT/skills/connect-phone" "$SKILLS_DIR/connect-phone"
echo "✓ skill installed → $SKILLS_DIR/connect-phone"

# 3. Show the pairing QR now, so the very first run is end-to-end.
echo "→ starting bridge + showing pairing QR..."
"$ROOT/scripts/run-bridge.sh" --open || true

cat <<'EOF'

✓ Done. On your phone:
    open HermesApp → 連線 → 掃描 QR Code → scan the QR on screen.
Next time, just tell your Hermes: 「連接手機」 and it shows the QR again.
EOF
