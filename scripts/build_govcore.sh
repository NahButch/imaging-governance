#!/usr/bin/env bash
# Build the deterministic `govcore` Rust core into an importable Python module.
#
# Usage:
#   scripts/build_govcore.sh            # maturin develop into the active venv (dev)
#   scripts/build_govcore.sh --release  # optimised develop build
#   scripts/build_govcore.sh wheel      # build a distributable wheel into dist/
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="$REPO_ROOT/crates/govcore"

if ! command -v maturin >/dev/null 2>&1; then
    echo "error: maturin not found. Install with: pip install maturin" >&2
    exit 1
fi

case "${1:-develop}" in
    wheel)
        maturin build --release --manifest-path "$CRATE_DIR/Cargo.toml" --out "$REPO_ROOT/dist"
        ;;
    --release)
        maturin develop --release --manifest-path "$CRATE_DIR/Cargo.toml"
        ;;
    develop|"")
        maturin develop --manifest-path "$CRATE_DIR/Cargo.toml"
        ;;
    *)
        echo "usage: build_govcore.sh [develop|--release|wheel]" >&2
        exit 2
        ;;
esac
