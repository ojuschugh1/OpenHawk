#!/usr/bin/env bash
# Publish all OpenHawk crates to crates.io in dependency order.
# Run from the openhawk/ directory.
# Safe to re-run — already-published crates are skipped automatically.
set -e

WAIT=60  # crates.io rate-limits new accounts to ~10 crates/day; space them out

published() {
  # returns 0 (true) if the crate version already exists on crates.io
  local crate=$1
  local ver=$2
  curl -sf "https://crates.io/api/v1/crates/${crate}/${ver}" \
    -H "User-Agent: openhawk-publish-script" \
    -o /dev/null 2>/dev/null
}

publish() {
  local crate=$1
  local ver="0.1.0"
  echo ""
  echo "publishing ${crate}..."

  if published "$crate" "$ver"; then
    echo "${crate} v${ver} already on crates.io, skipping."
    return
  fi

  cargo publish -p "$crate"
  echo "${crate} published. waiting ${WAIT}s for crates.io to index..."
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

# tier 4 — the CLI binary
publish openhawk

echo ""
echo "all crates published."
echo "install with: cargo install openhawk"
