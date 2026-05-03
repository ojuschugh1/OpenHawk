#!/usr/bin/env bash
# mock_hawk.sh — deterministic demo output for VHS recordings
# Mirrors the real openhawk binary: no banner on normal commands.

cmd="$*"

case "$cmd" in

  "vault set OPENAI_API_KEY sk-proj-abc123")
    echo "Vault: set OPENAI_API_KEY" ;;

  "vault list")
    echo "OPENAI_API_KEY" ;;

  "vault get OPENAI_API_KEY")
    echo "Vault: OPENAI_API_KEY injected into environment." ;;

  "vault rm OPENAI_API_KEY")
    echo "Vault: removed OPENAI_API_KEY" ;;

  run\ *)
    echo "Agent started: pid=42981" ;;

  "ps")
    printf "%-8s %-24s %-10s %-10s %-8s %s\n" "PID" "NAME" "STATE" "UPTIME" "CPU%" "MEM"
    printf "%-8s %-24s %-10s %-10s %-8s %s\n" "42981" "python research_a..." "Running" "00:01:23" "2.1%" "48MB"
    ;;

  orchestrate\ *)
    task="${cmd#orchestrate }"
    task="${task%\"}"; task="${task#\"}"
    echo "Orchestrating: $task"
    echo "Sub-tasks (3):"
    echo "  [0] research quantum computing → agent 42981"
    echo "  [1] write a summary            → agent 42981"
    echo "  [2] review it                  → agent 42981"
    echo "Dependencies:"
    echo "  [0] must complete before [2]"
    echo "  [1] must complete before [2]"
    echo ""
    sleep 0.3; echo "Executing plan..."
    sleep 0.25; echo "  [0] ✓ completed"
    sleep 0.25; echo "  [1] ✓ completed"
    sleep 0.25; echo "  [2] ✓ completed"
    echo ""
    echo "All 3 sub-tasks completed successfully."
    ;;

  "undo")
    echo "Rolled back to snapshot snap-a1b2c3 (3 files restored)."
    echo ""
    echo "M  research_notes.txt"
    echo "D  temp_output.json"
    echo "M  summary.md"
    ;;

  "healing history 42981")
    printf "%-4s %-28s %-24s %-8s %s\n" "ID" "TIMESTAMP" "ADJUSTMENT" "ATTEMPT" "OUTCOME"
    printf "%s\n" "----------------------------------------------------------------------"
    printf "%-4s %-28s %-24s %-8s %s\n" "1" "2026-05-03T14:01:22+00:00" "reduce_context+rollback" "1" "Success"
    ;;

  "verify sess-abc123")
    echo "Session: sess-abc123"
    echo "Status: Verified"
    echo ""
    echo "Claims:"
    echo "  [PASS] file_write:  /tmp/research_output.txt"
    echo "  [PASS] api_call:    https://api.openai.com/v1/chat"
    echo "  [PASS] file_write:  /tmp/summary.txt"
    ;;

  "stats tokens")
    echo "sqz is installed — showing real compression stats:"
    echo ""
    echo "┌─────────────────────────┬──────────────────┐"
    echo "│     sqz compression stats                 │"
    echo "├─────────────────────────┼──────────────────┤"
    echo "│ Total compressions      │              262 │"
    echo "│ Tokens saved            │            29986 │"
    echo "│ Avg reduction           │            72.1% │"
    echo "└─────────────────────────┴──────────────────┘"
    ;;

  "config show")
    echo "core.log_level = info"
    echo "core.session_retention_days = 30"
    echo "privacy.mode = standard"
    echo "savepoint.auto_snapshot = true"
    echo "healing.max_retries = 3"
    echo "healing.enabled = true"
    ;;

  "watch report")
    echo "Watch Report — generated at 2026-05-03T14:07:15+00:00"
    echo "============================================================"
    echo ""
    echo "API Drifts (0)"
    echo "  (none)"
    echo ""
    echo "Phantom Dependencies (0)"
    echo "  (none)"
    ;;

  "setup")
    printf "%-14s %-12s %s\n" "TOOL" "STATUS" "DESCRIPTION"
    printf "%s\n" "----------------------------------------------------------------------"
    printf "%-14s %-12s %s\n" "sqz"        "installed" "LLM token compression"
    printf "%-14s %-12s %s\n" "ghostdep"   "installed" "Phantom dependency detector"
    printf "%-14s %-12s %s\n" "claimcheck" "installed" "Agent claim verifier"
    printf "%-14s %-12s %s\n" "etch"       "missing"   "API drift detector"
    printf "%-14s %-12s %s\n" "aura"       "installed" "Persistent cross-session memory"
    echo ""
    echo "Installing etch... done (4s)"
    echo "Setup complete: 1 installed, 0 failed."
    ;;

  *)
    echo "openhawk: unknown command '$cmd'" ;;

esac
