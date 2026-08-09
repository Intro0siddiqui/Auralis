#!/usr/bin/env bash
# ============================================================================
# Auralis v2 — Dev script
# Launches the application in development mode with hot-reload of UI files.
# ============================================================================
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Auralis v2 — Dev mode"
echo "    Working directory: $(pwd)"

# Check for required tools
command -v cargo >/dev/null || { echo "ERROR: cargo not installed"; exit 1; }

# Optional: install tauri-cli if not present
if ! command -v cargo-tauri >/dev/null; then
    echo "==> tauri-cli not found, installing..."
    cargo install tauri-cli --version "^2.0" --locked
fi

# Run the app
echo "==> Launching in dev mode"
echo "    (UI changes in ./ui are picked up automatically)"
echo "    (Rust changes require recompile)"
echo
cargo tauri dev
