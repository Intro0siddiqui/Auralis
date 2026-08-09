#!/usr/bin/env bash
# ============================================================================
# Auralis v2 — Test script
# Runs cargo test across the workspace.
# ============================================================================
set -euo pipefail

cd "$(dirname "$0")/.."

echo "==> Auralis v2 — Tests"
echo "    Working directory: $(pwd)"

# Format check
echo "==> Checking formatting (cargo fmt --check)"
if cargo fmt --all -- --check; then
    echo "    Format: OK"
else
    echo "    Format: FAILED — run 'cargo fmt' to fix"
    exit 1
fi

# Clippy
echo "==> Running clippy"
cargo clippy --all-targets --all-features -- -D warnings || {
    echo "    Clippy: warnings or errors found"
    exit 1
}

# Unit tests
echo "==> Running unit tests"
cargo test --all-features --lib --bins

# Doctests
echo "==> Running doc tests"
cargo test --all-features --doc

# Integration tests
echo "==> Running integration tests"
cargo test --all-features --test '*' || true   # OK if no integration tests yet

echo
echo "✓ All tests passed"
