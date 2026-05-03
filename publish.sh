#!/usr/bin/env bash
# Publish all OpenHawk crates to crates.io in dependency order.
# Run from the openhawk/ directory.
# Safe to re-run — already-published crates are skipped automatically.

WAIT=60  # crates.io rate-limits new accounts; space them out

publish() {
  local crate=$1
  echo ""
  echo "publishing ${crate}..."

  output=$(cargo publish -p "$crate" 2>&1)
  code=$?

  if echo "$output" | grep -q "already exists"; then
    echo "${crate} already on crates.io, skipping."
    return 0
  fi

  if echo "$output" | grep -q "429 Too Many Requests"; then
    retry_after=$(echo "$output" | grep -o 'after [^a]*GMT' | head -1)
    echo ""
    echo "Rate limited by crates.io. Try again $retry_after"
    echo "Then just re-run: ./publish.sh"
    exit 1
  fi

  if [ $code -ne 0 ]; then
    echo "$output"
    echo ""
    echo "ERROR: ${crate} failed to publish (exit $code)"
    exit $code
  fi

  echo "$output"
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
