#!/usr/bin/env bash
# ============================================================================
# Auralis v2 — Build script
# Builds both the Rust backend and bundles the frontend + templates.
# ============================================================================
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)

echo "==> Auralis v2 — Build"
echo "    Working directory: $ROOT"

# --- Sanity checks ---------------------------------------------------------
[ -f Cargo.toml ] || { echo "ERROR: Cargo.toml not found"; exit 1; }
[ -d ui ]          || { echo "ERROR: ui/ directory not found"; exit 1; }
[ -d src/templates ] || { echo "ERROR: src/templates/ directory not found"; exit 1; }
[ -f tauri.conf.json ] || { echo "ERROR: tauri.conf.json not found"; exit 1; }

# --- Determine build profile ----------------------------------------------
PROFILE="${PROFILE:-release}"
PROFILE_FLAG=""
if [ "$PROFILE" = "release" ]; then
    PROFILE_FLAG="--release"
fi

echo "==> Profile: $PROFILE"

# --- Ensure targets are installed ----------------------------------------
echo "==> Checking rustup targets"
if rustup target list --installed 2>/dev/null | grep -q "x86_64-unknown-linux-gnu"; then
    echo "    linux x86_64 target: OK"
fi

# --- Build Rust backend ----------------------------------------------------
echo "==> Compiling Rust backend (this may take a few minutes on first run)"
cargo build $PROFILE_FLAG

# --- Copy resources --------------------------------------------------------
echo "==> Preparing distribution directory"
DIST_DIR="$ROOT/dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

# Copy UI assets (includes HTML partials)
cp -r "$ROOT/ui" "$DIST_DIR/ui"

# Copy the binary
BIN_NAME="auralis"
if [ -f "$ROOT/target/$PROFILE/$BIN_NAME" ]; then
    cp "$ROOT/target/$PROFILE/$BIN_NAME" "$DIST_DIR/$BIN_NAME"
    echo "    Binary: $DIST_DIR/$BIN_NAME"
fi

# Copy capabilities
mkdir -p "$DIST_DIR/capabilities"
cp "$ROOT/capabilities/"*.json "$DIST_DIR/capabilities/"

# --- Done ------------------------------------------------------------------
echo
echo "✓ Build complete"
echo "  Distribution: $DIST_DIR"
ls -la "$DIST_DIR" 2>/dev/null || true
