#!/usr/bin/env bash
# Stage and (optionally) push a Space subdir to Hugging Face as a Docker Space.
#
# Each Space is independently deployable. Because the deterministic core lives
# in the shared `crates/govcore` crate at the repo root, this script vendors a
# minimal workspace copy into `<space>/govcore_src/` so the Space's Dockerfile
# can build it from its own context.
#
# Usage:
#   scripts/deploy_space.sh 01-dicom-qc                      # stage only
#   scripts/deploy_space.sh 01-dicom-qc <hf-repo-url>        # stage + push
#
# Staging is written to build/deploy/<space>/ and is safe to inspect or
# `docker build` locally before pushing.
set -euo pipefail

SPACE="${1:?usage: deploy_space.sh <space-subdir> [hf-repo-url]}"
HF_URL="${2:-}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$REPO_ROOT/spaces/$SPACE"
[ -d "$SRC" ] || { echo "error: no such space: spaces/$SPACE" >&2; exit 1; }

STAGE="$REPO_ROOT/build/deploy/$SPACE"
rm -rf "$STAGE"
mkdir -p "$STAGE"

# Space files (exclude any previous staging artifact).
rsync -a --exclude 'govcore_src' "$SRC"/ "$STAGE"/

# Vendor a minimal workspace so the Dockerfile can build govcore.
mkdir -p "$STAGE/govcore_src/crates"
cp "$REPO_ROOT/Cargo.toml" "$STAGE/govcore_src/Cargo.toml"
rsync -a --exclude target "$REPO_ROOT/crates/govcore" "$STAGE/govcore_src/crates/"

echo "Staged Space '$SPACE' -> $STAGE"

if [ -n "$HF_URL" ]; then
    echo "Pushing to $HF_URL ..."
    TMP="$(mktemp -d)"
    git clone "$HF_URL" "$TMP"
    rsync -a --delete --exclude '.git' "$STAGE"/ "$TMP"/
    git -C "$TMP" add -A
    git -C "$TMP" commit -m "deploy $SPACE" || echo "nothing to commit"
    git -C "$TMP" push
    rm -rf "$TMP"
    echo "Pushed."
else
    echo "Dry run (no HF url given). To build locally:"
    echo "  docker build -t $SPACE $STAGE"
fi
