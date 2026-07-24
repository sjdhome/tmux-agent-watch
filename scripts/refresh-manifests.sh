#!/bin/sh
# Refresh vendored agent-detection manifests from a local herdr checkout.
# Usage: scripts/refresh-manifests.sh [path-to-herdr]
# After refreshing, `cargo test` is the drift gate: deny_unknown_fields
# rejects new TOML fields, and the region-coverage test rejects rules using
# regions this engine does not implement. Update NOTICE with the new herdr
# commit after a refresh.

set -eu

herdr="${1:-../herdr}"
dest="$(dirname "$0")/../src/detect/manifests"

if [ ! -d "$herdr/src/detect/manifests" ]; then
    echo "herdr checkout not found at $herdr" >&2
    exit 1
fi

cp "$herdr"/src/detect/manifests/*.toml "$dest"/
echo "copied $(ls "$dest"/*.toml | wc -l | tr -d ' ') manifests from $herdr"
echo "herdr commit: $(git -C "$herdr" rev-parse HEAD)"
echo "now run: cargo test"
