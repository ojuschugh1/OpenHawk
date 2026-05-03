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
publish hawk-bus
publish hawk-compress
publish hawk-memory
publish hawk-nest
publish hawk-sync
publish hawk-verify
publish hawk-watch
publish hawk-core

# tier 2 — depend on tier 1
publish hawk-savepoint
publish hawk-vault
publish hawk-sdk-rust

# tier 3 — depends on tier 1 + 2
publish hawk-ui

# tier 4 — the CLI binary (depends on everything)
publish hawk-cli

echo ""
echo "all crates published."
echo "install with: cargo install hawk-cli"
