<p align="center"><pre>
 ██████╗ ██████╗ ███████╗███╗   ██╗██╗  ██╗ █████╗ ██╗    ██╗██╗  ██╗
██╔═══██╗██╔══██╗██╔════╝████╗  ██║██║  ██║██╔══██╗██║    ██║██║ ██╔╝
██║   ██║██████╔╝█████╗  ██╔██╗ ██║███████║███████║██║ █╗ ██║█████╔╝ 
██║   ██║██╔═══╝ ██╔══╝  ██║╚██╗██║██╔══██║██╔══██║██║███╗██║██╔═██╗ 
╚██████╔╝██║     ███████╗██║ ╚████║██║  ██║██║  ██║╚███╔███╔╝██║  ██╗
 ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═══╝╚═╝  ╚═╝╚═╝  ╚═╝ ╚══╝╚══╝ ╚═╝  ╚═╝
</pre></p>

<p align="center"><strong>local-first Agent OS — manage AI agents like real processes</strong></p>

<p align="center">
  <a href="https://crates.io/crates/openhawk"><img src="https://img.shields.io/crates/v/openhawk?logo=rust&logoColor=white&label=crates.io&color=e6522c" alt="crates.io"></a>
  <a href="https://github.com/ojuschugh1/openhawk/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT"></a>
</p>

OpenHawk is a local-first Agent Operating System built in Rust. It manages AI agents as first-class OS processes, providing filesystem safety through Copy-on-Write snapshots, inter-agent communication over a JSON-RPC bus, per-agent permission sandboxing, encrypted secrets management, and a TUI dashboard for real-time observability.

It works with any agent framework: CrewAI, LangGraph, AutoGen, custom scripts, or anything that runs as a process.

---

INSTALL

The fastest way — install directly from crates.io (no git clone needed):

    cargo install openhawk

This puts the `hawk` binary in `~/.cargo/bin/`. Make sure that's on your PATH:

    export PATH="$HOME/.cargo/bin:$PATH"

Verify it works:

    hawk --help

First run will check for companion tools and prompt you to install them:

    hawk setup --yes

That installs sqz, ghostdep, claimcheck, etch, and aura into `~/.local/bin/`.

---

INSTALL FROM SOURCE

If you want to build from source or contribute:

Prerequisites:
- Rust 1.75+: https://rustup.rs
- Git

    git clone https://github.com/ojuschugh1/openhawk
    cd openhawk
    cargo build --release
    cargo install --path hawk-cli

---

COMPANION TOOLS

OpenHawk integrates with these tools for full functionality. Run `hawk setup --yes` to install them all automatically, or install individually:

    sqz         token compression (60-92% savings on repeated reads)
    ghostdep    phantom dependency detector
    claimcheck  agent claim verifier
    etch        API drift detector
    aura        persistent cross-session memory

Check what's installed:

    hawk setup

Install missing tools:

    hawk setup --yes

Install a specific tool:

    hawk setup --only sqz --yes

Force reinstall all:

    hawk setup --force --yes

---

PLATFORMS

- macOS 12+ (Apple Silicon and Intel)
- Linux (kernel 5.10+, x86_64 and aarch64)
- Windows 10+ (via WSL2 recommended)

---

QUICK START

    # spawn an agent
    hawk run "python my_agent.py"

    # list running agents
    hawk ps

    # store a secret
    hawk vault set OPENAI_API_KEY sk-proj-abc123

    # orchestrate a multi-agent task
    hawk orchestrate "research quantum computing and write a summary then review it"

    # scaffold a new agent project
    hawk sdk init python --name my-agent --output ~/projects

    # see token savings from sqz
    hawk stats tokens

---

CONFIGURATION

OpenHawk reads configuration from ~/.hawk/config.toml. Create one:

    mkdir -p ~/.hawk
    cat > ~/.hawk/config.toml << 'EOF'
    [core]
    log_level = "info"
    session_retention_days = 30
    pattern_retention_days = 90

    [privacy]
    mode = "standard"

    [llm]
    providers = [
        { name = "ollama", endpoint = "http://localhost:11434", priority = 1 },
    ]

    [savepoint]
    auto_snapshot = true
    max_snapshots_per_agent = 50

    [healing]
    max_retries = 3
    enabled = true
    EOF

View effective config:

    hawk config show

Output:

    core.log_level = info
    core.session_retention_days = 30
    core.pattern_retention_days = 90
    privacy.mode = standard
    savepoint.auto_snapshot = true
    savepoint.max_snapshots_per_agent = 50
    healing.max_retries = 3
    healing.enabled = true

Set a value:

    hawk config set privacy.mode air-gapped

Show LLM providers:

    hawk config llm

---

AGENT LIFECYCLE

Spawn an agent (any command):

    hawk run "python research_agent.py"
    hawk run "node my-agent/dist/index.js"
    hawk run "sleep 300"

Output:

    Agent started: pid=42981

List running agents:

    hawk ps

Output:

    PID    NAME                 STATE      UPTIME
    42981  python research_a... Running    00:01:23

Pause, resume, stop:

    hawk pause 42981
    hawk resume 42981
    hawk stop 42981

---

FILESYSTEM SNAPSHOTS (PERCH)

OpenHawk creates a snapshot before every agent task using OS-native Copy-on-Write (APFS reflinks on macOS, Btrfs COW on Linux, file-copy fallback elsewhere).

Roll back to a snapshot:

    hawk undo <snapshot-id>

View what changed since a snapshot:

    hawk diff <snapshot-id>

Output:

    M  src/main.rs
    A  src/new_file.rs
    D  src/old_file.rs

---

SECRETS VAULT

Secrets are encrypted with AES-256-GCM and stored locally. Keys are derived from your system keychain or a passphrase via Argon2.

Store a secret:

    hawk vault set OPENAI_API_KEY sk-proj-abc123

List stored keys (values are never shown):

    hawk vault list

Output:

    OPENAI_API_KEY

Inject into an agent's environment (never printed):

    hawk vault get OPENAI_API_KEY

Output:

    Vault: OPENAI_API_KEY injected into environment.

Delete a secret:

    hawk vault rm OPENAI_API_KEY

---

MULTI-AGENT ORCHESTRATION

Decompose a complex task across agents. Use "and" for parallel, "then" for sequential.

    hawk orchestrate "research quantum computing and write a summary then review it"

Output:

    Orchestrating: research quantum computing and write a summary then review it
    Sub-tasks (3):
      [0] research quantum computing -> agent 42981
      [1] write a summary -> agent 42981
      [2] review it -> agent 42981
    Dependencies:
      [0] must complete before [2]
      [1] must complete before [2]

    Executing plan...
      [0] completed
      [1] completed
      [2] completed

    All 3 sub-tasks completed successfully.

---

SDK SCAFFOLDING

Generate a minimal agent project in your preferred language:

Rust:

    hawk sdk init rust --name my-research-agent --output ~/projects

Python:

    hawk sdk init python --name data-pipeline --output ~/projects

TypeScript:

    hawk sdk init typescript --name web-scraper --output ~/projects

Each scaffold includes:
- Agent_Manifest.toml (permissions, resources, capabilities)
- Entry point with hawk-bus client, handler registration, pub/sub loop
- Build config (Cargo.toml / requirements.txt / package.json)

Example generated Agent_Manifest.toml:

    [agent]
    name = "my-research-agent"
    version = "0.1.0"
    description = ""
    framework = "custom"
    entry_command = "cargo run"

    [permissions]
    filesystem = []
    network = []
    commands = []
    secrets = []

    [resources]
    cpu_percent = 25
    memory_mb = 256
    max_open_fds = 32

---

MESSAGE BUS

Agents communicate over a JSON-RPC 2.0 bus. Inspect active topics:

    hawk bus inspect

Output:

    No active topics. Pending messages (direct): 0

The bus supports:
- Pub/sub topic routing (deliver to all subscribers)
- Direct messaging by process ID
- Offline queuing (messages held until target agent resumes)
- Message expiration with configurable retention

---

MONITORING AND OBSERVABILITY

Watch report (API drift + phantom dependencies):

    hawk watch report

Output:

    Watch Report -- generated at 2026-05-03T13:12:09Z
    ============================================================

    API Drifts (0)
      (none)

    Phantom Dependencies (0)
      (none)

Token compression stats:

    hawk stats tokens

Cost tracking:

    hawk stats cost

---

CROSS-DEVICE SYNC

Sync agents and memory across devices over LAN. All data is encrypted with AES-256-GCM.

Enable sync:

    hawk sync enable "my-shared-secret-phrase"

Mark items for selective sync:

    hawk sync select my-research-agent
    hawk sync select memory:research-namespace

Check status:

    hawk sync status

Set conflict resolution strategy:

    hawk sync resolve --strategy last-write

Options: last-write, manual, merge

---

PATTERN DETECTION

OpenHawk detects repeated action sequences and offers to automate them.

List detected patterns:

    hawk patterns list

Accept a pattern (generates an Agent_Manifest):

    hawk patterns accept <pattern-id>

Decline (suppresses future offers):

    hawk patterns decline <pattern-id>

Re-enable declined patterns:

    hawk patterns reset

---

SELF-HEALING

When an agent fails, OpenHawk reverts to the last snapshot, adjusts parameters, and retries (up to the configured max_retries).

View healing history for an agent:

    hawk healing history <agent-pid>

---

TALON PLUGIN SYSTEM

Talons are community-contributed plugins (browser automation, Slack, GitHub, etc.).

Install a Talon:

    hawk talon install browser-talon

List installed Talons:

    hawk talon list

Talons are signature-verified before loading. A failing Talon is isolated and cannot crash the kernel.

---

HAWKNEST MARKETPLACE

Search for community packages:

    hawk nest search "browser automation"

Install a package:

    hawk nest install browser-talon

Publish your own:

    hawk nest publish ./my-agent/

Packages are validated (Agent_Manifest.toml required, semver enforced) and signature-verified.

---

HAWKEYE TUI DASHBOARD

Launch the full-screen terminal dashboard:

    hawk eye

Keyboard shortcuts:
- j/k or arrows: navigate agent list
- Enter: open agent detail
- u: undo (rollback) for selected agent
- /: search
- Tab: switch panels (Dashboard -> Alerts -> Orchestration Graph)
- q: quit

---

AIR-GAPPED MODE

Block all outbound network requests:

    hawk config set privacy.mode air-gapped

In air-gapped mode:
- All LLM requests route to local providers only (Ollama, llama.cpp)
- HawkNest operates from local cache only
- All outbound network attempts are denied and logged

Disable:

    hawk config set privacy.mode standard

---

SESSION REPLAY

Replay an agent session step by step:

    hawk replay <session-id>

Output:

    Session: sess-abc123
    ------------------------------------------------------------------------
    Step    1  2026-05-03T10:00:01Z  pid=42981  file_read
    Step    2  2026-05-03T10:00:02Z  pid=42981  api_call
    Step    3  2026-05-03T10:00:03Z  pid=42981  file_write

Jump to a specific step (shows full context at that point):

    hawk replay <session-id> --step 2

---

VERIFICATION

Verify that an agent actually completed what it claimed:

    hawk verify <session-id>

Output:

    Session: sess-abc123
    Status: Verified

    Claims:
      [PASS] file_write: /tmp/output.txt
      [PASS] api_call: https://api.openai.com/v1/chat

---

PROJECT STRUCTURE

    openhawk/
    |-- hawk-cli/          hawk binary (clap CLI)
    |-- hawk-core/         kernel: lifecycle, permissions, orchestration,
    |                      config, patterns, healing, LLM routing, sessions
    |-- hawk-savepoint/    filesystem snapshots (Perch)
    |-- hawk-vault/        AES-256-GCM encrypted secrets
    |-- hawk-bus/          JSON-RPC 2.0 message bus
    |-- hawk-memory/       shared memory (Aura bridge)
    |-- hawk-verify/       post-task verification (ClaimCheck bridge)
    |-- hawk-compress/     token compression (SQZ bridge)
    |-- hawk-watch/        API drift + dependency scanning (Etch/GhostDep)
    |-- hawk-ui/           ratatui TUI dashboard (HawkEye)
    |-- hawk-nest/         marketplace client (HawkNest)
    |-- hawk-sync/         cross-device sync
    |-- hawk-sdk-rust/     Rust SDK + Python/TypeScript binding stubs

---

AGENT MANIFEST FORMAT

Every agent declares its permissions and resource limits in a TOML manifest:

    [agent]
    name = "research-agent"
    version = "1.0.0"
    description = "Researches topics and produces summaries"
    framework = "langraph"
    entry_command = "python research_agent.py"

    [permissions]
    filesystem = ["~/projects/research/**", "/tmp/hawk-scratch/**"]
    network = ["https://api.openai.com/*", "https://scholar.google.com/*"]
    commands = ["curl", "python3"]
    secrets = ["OPENAI_API_KEY"]

    [resources]
    cpu_percent = 25
    memory_mb = 512
    max_open_fds = 64

    [llm]
    provider = "openai"
    privacy = "cloud"
    budget_tokens = 1000000

    [talons]
    required = ["browser-talon", "github-talon"]

    [capabilities]
    tags = ["research", "summarization", "web-search"]

---

RUNNING TESTS

Run the full test suite (490 tests):

    cd openhawk
    cargo test

Run tests for a specific crate:

    cargo test -p hawk-core
    cargo test -p hawk-savepoint
    cargo test -p hawk-vault
    cargo test -p hawk-bus

Run a specific test:

    cargo test -p hawk-core -- orchestrator::tests::then_produces_sequential_dependency

---

DEVELOPMENT

Build in debug mode:

    cargo build

Build in release mode:

    cargo build --release

Install locally from source:

    cargo install --path hawk-cli

Install from crates.io:

    cargo install openhawk

The binary is placed at ~/.cargo/bin/hawk.

---

DATA STORAGE

OpenHawk stores all persistent data locally:

    ~/Library/Application Support/hawk/hawk.db    SQLite database (macOS)
    ~/.local/share/hawk/hawk.db                   SQLite database (Linux)
    ~/.hawk/config.toml                           Global configuration
    ~/.hawk/vault.enc                             Encrypted secrets vault

No data leaves your machine unless you explicitly enable sync or use cloud LLM providers.

---

COMMAND REFERENCE

    hawk run <command>                    Spawn an agent process
    hawk stop <pid>                       Stop an agent (graceful, then force)
    hawk pause <pid>                      Suspend an agent
    hawk resume <pid>                     Resume a suspended agent
    hawk ps                               List managed agents

    hawk undo [snapshot-id]               Roll back to a snapshot
    hawk diff <snapshot-id>               Show file changes since snapshot

    hawk vault set <key> <value>          Store an encrypted secret
    hawk vault get <key>                  Inject secret into environment
    hawk vault rm <key>                   Delete a secret
    hawk vault list                       List key names

    hawk config show                      Show effective configuration
    hawk config set <key> <value>         Set a config value
    hawk config llm                       Show LLM provider status

    hawk orchestrate <task>               Decompose and execute a multi-agent task
    hawk trust <agent-name>               Bypass permission checks for session

    hawk verify <session-id>              Verify agent claims against evidence
    hawk replay <session-id> [--step N]   Replay a session

    hawk bus inspect                      Show bus topics and queue depths
    hawk watch report                     API drift and dependency report
    hawk stats tokens                     Token compression stats
    hawk stats cost                       Per-agent cost breakdown

    hawk talon install <name>             Install a plugin
    hawk talon list                       List installed plugins

    hawk nest search <query>              Search marketplace
    hawk nest install <name>              Install a package
    hawk nest publish <path>              Publish a package

    hawk sdk init <lang> --name <name>    Scaffold an agent project
    hawk sdk info                         Show SDK version

    hawk sync enable <secret>             Enable cross-device sync
    hawk sync select <item>               Mark item for sync
    hawk sync status                      Show sync status
    hawk sync resolve --strategy <s>      Set conflict resolution

    hawk patterns list                    Show detected patterns
    hawk patterns accept <id>             Accept and automate a pattern
    hawk patterns decline <id>            Suppress a pattern
    hawk patterns reset                   Re-enable declined patterns

    hawk healing history <pid>            Show self-healing events
    hawk healing status                   Show healing status

    hawk eye                              Launch TUI dashboard

---

LICENSE

MIT
