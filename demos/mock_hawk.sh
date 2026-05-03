#!/usr/bin/env bash
# mock_hawk.sh — deterministic demo output for VHS recordings
# Never depends on real agent processes, always instant, never fails.

banner() {
cat << 'BANNER'
 ██████╗ ██████╗ ███████╗███╗   ██╗██╗  ██╗ █████╗ ██╗    ██╗██╗  ██╗
██╔═══██╗██╔══██╗██╔════╝████╗  ██║██║  ██║██╔══██╗██║    ██║██║ ██╔╝
██║   ██║██████╔╝█████╗  ██╔██╗ ██║███████║███████║██║ █╗ ██║█████╔╝ 
██║   ██║██╔═══╝ ██╔══╝  ██║╚██╗██║██╔══██║██╔══██║██║███╗██║██╔═██╗ 
╚██████╔╝██║     ███████╗██║ ╚████║██║  ██║██║  ██║╚███╔███╔╝██║  ██╗
 ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═══╝╚═╝  ╚═╝╚═╝  ╚═╝ ╚══╝╚══╝ ╚═╝  ╚═╝

BANNER
}

cmd="$*"

case "$cmd" in

  "vault set OPENAI_API_KEY sk-proj-abc123")
    banner; echo "Vault: set OPENAI_API_KEY" ;;

  "vault list")
    banner; echo "OPENAI_API_KEY" ;;

  "vault get OPENAI_API_KEY")
    banner; echo "Vault: OPENAI_API_KEY injected into environment." ;;

  "vault rm OPENAI_API_KEY")
    banner; echo "Vault: removed OPENAI_API_KEY" ;;

  run\ *)
    banner; echo "Agent started: pid=42981" ;;

  "ps")
    banner
    printf "%-8s %-24s %-10s %-10s %-8s %s\n" "PID" "NAME" "STATE" "UPTIME" "CPU%" "MEM"
    printf "%-8s %-24s %-10s %-10s %-8s %s\n" "42981" "python research_a..." "Running" "00:01:23" "2.1%" "48MB"
    ;;

  orchestrate\ *)
    banner
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
    echo ""; echo "All 3 sub-tasks completed successfully."
    ;;

  "undo")
    banner
    echo "Rolled back to snapshot snap-a1b2c3 (3 files restored)."
    echo ""
    echo "M  research_notes.txt"
    echo "D  temp_output.json"
    echo "M  summary.md"
    ;;

  "healing history 42981")
    banner
    printf "%-4s %-28s %-20s %-8s %s\n" "ID" "TIMESTAMP" "ADJUSTMENT" "ATTEMPT" "OUTCOME"
    printf "%s\n" "----------------------------------------------------------------------"
    printf "%-4s %-28s %-20s %-8s %s\n" "1" "2026-05-03T14:01:22+00:00" "reduce_context+rollback" "1" "Success"
    ;;

  "verify sess-abc123")
    banner
    echo "Session: sess-abc123"
    echo "Status: Verified"
    echo ""
    echo "Claims:"
    echo "  [PASS] file_write:  /tmp/research_output.txt"
    echo "  [PASS] api_call:    https://api.openai.com/v1/chat"
    echo "  [PASS] file_write:  /tmp/summary.txt"
    ;;

  "stats tokens")
    banner
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
    banner
    echo "core.log_level = info"
    echo "core.session_retention_days = 30"
    echo "privacy.mode = standard"
    echo "savepoint.auto_snapshot = true"
    echo "healing.max_retries = 3"
    echo "healing.enabled = true"
    ;;

  "watch report")
    banner
    echo "Watch Report — generated at 2026-05-03T14:07:15+00:00"
    echo "============================================================"
    echo ""
    echo "API Drifts (0)"
    echo "  (none)"
    echo ""
    echo "Phantom Dependencies (0)"
    echo "  (none)"
    ;;

  *)
    banner; echo "hawk: unknown command '$cmd'" ;;

esac
