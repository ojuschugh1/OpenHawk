#!/usr/bin/env bash
# Publish all OpenHawk crates to crates.io in dependency order.
# Run from the openhawk/ directory.
set -e

WAIT=30  # seconds between publishes for crates.io indexing

publish() {
  echo ""
  echo "publishing $1..."
  cargo publish -p "$1"
  echo "$1 published. waiting ${WAIT}s for crates.io to index..."
  sleep $WAIT
}

# tier 1 — no local deps
publish openhawk-bus
publish openhawk-compress
publish openhawk-memory
publish openhawk-nest
publish openhawk-sync
publish openhawk-verify
publish openhawk-watch
publish openhawk-core

# tier 2 — depend on tier 1
publish openhawk-savepoint
publish openhawk-vault
publish openhawk-sdk

# tier 3 — depends on tier 1 + 2
publish openhawk-ui

# tier 4 — the CLI binary (depends on everything)
publish openhawk

echo ""
echo "all crates published."
echo "install with: cargo install openhawk"
