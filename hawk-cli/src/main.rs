use clap::{Parser, Subcommand};

mod setup;

// ── Sub-command argument structs ──────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum VaultCommand {
    Set { key: String, value: String },
    Get { key: String },
    Rm { key: String },
    List,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    Show,
    Set { key: String, value: String },
    /// Display configured LLM providers and their availability
    Llm,
}

#[derive(Debug, Subcommand)]
pub enum BusCommand {
    Inspect,
}

#[derive(Debug, Subcommand)]
pub enum WatchCommand {
    Start,
    Stop,
    Alerts,
    Report,
    /// Run ghostdep on a project directory to detect phantom/unused dependencies
    Scan {
        /// Agent name to associate findings with
        #[arg(long, default_value = "unknown")]
        agent: String,
        /// Project directory to scan (default: current directory)
        #[arg(long, default_value = ".")]
        path: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum StatsCommand {
    Show,
    Tokens,
    Cost,
}

#[derive(Debug, Subcommand)]
pub enum TalonCommand {
    List,
    Install { name: String },
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
pub enum NestCommand {
    Search { query: String },
    Install { name: String },
    Publish { path: String },
}

#[derive(Debug, Subcommand)]
pub enum SdkCommand {
    Info,
    /// Generate a minimal agent project scaffold
    Init {
        /// Target language: rust, python, typescript
        language: String,
        /// Agent name
        #[arg(long, default_value = "my-agent")]
        name: String,
        /// Output directory (defaults to current directory)
        #[arg(long)]
        output: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum SyncCommand {
    Enable { shared_secret: String },
    /// Mark an agent or memory namespace for sync
    Select { item: String },
    Status,
    Peers,
    /// Set conflict resolution strategy
    Resolve {
        #[arg(long)]
        strategy: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum PatternsCommand {
    List,
    Reset,
    Accept { pattern_id: String },
    Decline { pattern_id: String },
}

#[derive(Debug, Subcommand)]
pub enum HealingCommand {
    Status,
    History { agent_id: u32 },
}

// ── Top-level command enum ────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "hawk", about = "OpenHawk Agent OS")]
pub enum HawkCommand {
    /// Run an agent by command string
    Run { agent_command: String },
    /// Stop a running agent
    Stop { process_id: u32 },
    /// Pause a running agent
    Pause { process_id: u32 },
    /// Resume a paused agent
    Resume { process_id: u32 },
    /// List all managed agents
    Ps,
    /// Roll back to a snapshot
    Undo { snapshot_id: Option<String> },
    /// Show diff for a snapshot
    Diff { snapshot_id: String },
    /// Open HawkEye TUI (Phase 2)
    Eye,
    /// Verify agent claims — uses claimcheck binary when available
    Verify {
        session_id: String,
        /// Path to a transcript file (.jsonl or .md) for claimcheck verification
        #[arg(long)]
        transcript: Option<String>,
        /// Project directory for claimcheck (default: current directory)
        #[arg(long)]
        project_dir: Option<String>,
        /// Git baseline ref for claimcheck (e.g. HEAD~3, main)
        #[arg(long)]
        baseline: Option<String>,
        /// Re-run tests to verify test claims (claimcheck --retest)
        #[arg(long, default_value = "false")]
        retest: bool,
    },
    /// Replay a session (Phase 2)
    Replay {
        session_id: String,
        #[arg(long)]
        step: Option<usize>,
    },
    /// Manage secrets vault
    #[command(subcommand)]
    Vault(VaultCommand),
    /// Inspect the message bus (Phase 2)
    #[command(subcommand)]
    Bus(BusCommand),
    /// Watch for API drift and phantom deps (Phase 2)
    #[command(subcommand)]
    Watch(WatchCommand),
    /// Show resource stats (Phase 2)
    #[command(subcommand)]
    Stats(StatsCommand),
    /// Manage configuration
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Manage Talons (Phase 2)
    #[command(subcommand)]
    Talon(TalonCommand),
    /// HawkNest marketplace (Phase 2)
    #[command(subcommand)]
    Nest(NestCommand),
    /// SDK utilities (Phase 2)
    #[command(subcommand)]
    Sdk(SdkCommand),
    /// Cross-device sync (Phase 2)
    #[command(subcommand)]
    Sync(SyncCommand),
    /// Orchestrate a multi-agent task (Phase 2)
    Orchestrate { task_description: String },
    /// Trust an agent for the current session
    Trust { agent_name: String },
    /// Manage detected patterns (Phase 2)
    #[command(subcommand)]
    Patterns(PatternsCommand),
    /// Self-healing status and history (Phase 2)
    #[command(subcommand)]
    Healing(HealingCommand),
    /// Install or update all companion tools (sqz, ghostdep, claimcheck, etch, aura)
    Setup {
        /// Install without prompting for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
        /// Force reinstall even if already installed
        #[arg(long)]
        force: bool,
        /// Only install specific tools (comma-separated: sqz,ghostdep,claimcheck,etch,aura)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Skip specific tools
        #[arg(long, value_delimiter = ',')]
        skip: Vec<String>,
    },
}

// ── Formatting helpers ────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

fn format_uptime(d: std::time::Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn print_ps(agents: &[hawk_core::types::AgentStatus]) {
    if agents.is_empty() {
        println!("No agents running.");
        return;
    }
    println!("{:<6} {:<20} {:<10} {}", "PID", "NAME", "STATE", "UPTIME");
    for a in agents {
        println!(
            "{:<6} {:<20} {:<10} {}",
            a.pid,
            a.name,
            format!("{:?}", a.state),
            format_uptime(a.uptime),
        );
    }
}

fn print_diff(diffs: &[hawk_savepoint::FileDiff]) {
    if diffs.is_empty() {
        println!("No changes.");
        return;
    }
    for d in diffs {
        let prefix = match d.change_type {
            hawk_savepoint::ChangeType::Added => "A",
            hawk_savepoint::ChangeType::Modified => "M",
            hawk_savepoint::ChangeType::Deleted => "D",
        };
        println!("{prefix}  {}", d.path);
    }
}

fn print_token_stats(stats: &hawk_compress::CompressionStats) {
    println!(
        "{:<8} {:<20} {:<20} {:<15}",
        "PID", "TOKENS PROCESSED", "TOKENS SAVED", "REDUCTION %"
    );
    println!("{}", "-".repeat(65));

    let mut pids: Vec<u32> = stats.per_agent.keys().copied().collect();
    pids.sort();

    for pid in &pids {
        let s = &stats.per_agent[pid];
        let pct = if s.tokens_processed > 0 {
            (s.tokens_saved as f64 / s.tokens_processed as f64) * 100.0
        } else {
            0.0
        };
        println!(
            "{:<8} {:<20} {:<20} {:.1}%",
            pid, s.tokens_processed, s.tokens_saved, pct
        );
    }

    println!("{}", "-".repeat(65));
    let total_pct = if stats.total_tokens_processed > 0 {
        (stats.total_tokens_saved as f64 / stats.total_tokens_processed as f64) * 100.0
    } else {
        0.0
    };
    println!(
        "{:<8} {:<20} {:<20} {:.1}%",
        "TOTAL",
        stats.total_tokens_processed,
        stats.total_tokens_saved,
        total_pct
    );
    println!("Cache entries: {}", stats.cache_entries);
}

fn ping_provider(endpoint: &str) -> &'static str {
    // Real implementation would send an HTTP HEAD/GET to the endpoint.
    // We check reachability heuristically: local endpoints are assumed
    // available; remote ones are marked unknown without a live network call.
    if endpoint.contains("localhost") || endpoint.contains("127.0.0.1") {
        "available"
    } else {
        "unknown"
    }
}

fn print_llm_providers(providers: &[hawk_core::config::LlmProvider]) {
    if providers.is_empty() {
        println!("No LLM providers configured.");
        println!("Add providers to hawk.toml under [llm.providers].");
        return;
    }
    println!(
        "{:<4} {:<16} {:<42} {}",
        "PRI", "NAME", "ENDPOINT", "STATUS"
    );
    println!("{}", "-".repeat(70));
    let mut sorted: Vec<&hawk_core::config::LlmProvider> = providers.iter().collect();
    sorted.sort_by_key(|p| p.priority);
    for p in sorted {
        let status = ping_provider(&p.endpoint);
        println!("{:<4} {:<16} {:<42} {}", p.priority, p.name, p.endpoint, status);
    }
}

fn print_cost_stats(
    all_stats: &[(u32, hawk_core::token_tracker::AgentTokenStats)],
    trend_map: &std::collections::HashMap<u32, Vec<hawk_core::token_tracker::DailyTokenUsage>>,
) {
    if all_stats.is_empty() {
        println!("No token usage recorded.");
        return;
    }
    println!(
        "{:<8} {:<16} {:<16} {:<16} {}",
        "PID", "PROMPT TOKENS", "COMPL TOKENS", "TOTAL TOKENS", "EST COST ($)"
    );
    println!("{}", "-".repeat(72));
    for (pid, s) in all_stats {
        println!(
            "{:<8} {:<16} {:<16} {:<16} {:.6}",
            pid,
            s.total_prompt_tokens,
            s.total_completion_tokens,
            s.total_tokens,
            s.estimated_cost,
        );
        if let Some(trend) = trend_map.get(pid) {
            if !trend.is_empty() {
                println!("  7-day trend:");
                for day in trend {
                    println!("    {} — {} tokens  ${:.6}", day.date, day.tokens, day.cost);
                }
            }
        }
    }
}

fn db_path() -> std::path::PathBuf {
    dirs_next::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("hawk")
        .join("hawk.db")
}

/// Derive claims from recorded session actions (each action becomes a claim).
fn load_claims_from_session(
    db: &rusqlite::Connection,
    session_id: &str,
) -> anyhow::Result<Vec<hawk_verify::AgentClaim>> {
    let mut stmt = db.prepare(
        "SELECT action_type, timestamp, payload FROM session_actions WHERE session_id = ?1 ORDER BY step_number ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![session_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut claims = Vec::new();
    for row in rows {
        let (action_type, timestamp, payload) = row?;
        let resource = extract_resource(&action_type, &payload);
        claims.push(hawk_verify::AgentClaim {
            action_type,
            resource,
            claimed_at: timestamp,
        });
    }
    Ok(claims)
}

fn extract_resource(action_type: &str, payload: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
        let keys: &[&str] = match action_type {
            "api_call" => &["url", "resource"],
            "command_exec" => &["command", "resource"],
            _ => &["path", "resource"],
        };
        for key in keys {
            if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
                return s.to_string();
            }
        }
    }
    String::new()
}

fn print_verification_report(report: &hawk_verify::VerificationReport) {
    println!("Session: {}", report.session_id);
    println!("Status: {}", report.overall_status);

    // show claimcheck truth score when available
    if let Some(ref score) = report.truth_score {
        println!("Truth score: {score}");
        if let Some(ref cc) = report.claimcheck_raw {
            println!(
                "Claims: {} total, {} passed, {} failed, {} unverifiable",
                cc.summary.total, cc.summary.pass, cc.summary.fail, cc.summary.unverifiable
            );
        }
    }

    println!();
    if report.claims.is_empty() {
        println!("No claims recorded.");
        return;
    }
    println!("Claims:");
    for result in &report.claims {
        let tag = match &result.verdict {
            hawk_verify::ClaimVerdict::Pass => "[PASS]",
            hawk_verify::ClaimVerdict::Fail => "[FAIL]",
            hawk_verify::ClaimVerdict::Inconclusive { .. } => "[INCONCLUSIVE]",
        };
        let detail = match &result.verdict {
            hawk_verify::ClaimVerdict::Inconclusive { reason } => format!(" ({reason})"),
            _ => String::new(),
        };
        println!(
            "  {} {}: {}{}",
            tag, result.claim.action_type, result.claim.resource, detail
        );
        for d in &result.discrepancies {
            println!("    Reason: {d}");
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn print_banner() {
    println!(
        " ██████╗ ██████╗ ███████╗███╗   ██╗██╗  ██╗ █████╗ ██╗    ██╗██╗  ██╗"
    );
    println!(
        "██╔═══██╗██╔══██╗██╔════╝████╗  ██║██║  ██║██╔══██╗██║    ██║██║ ██╔╝"
    );
    println!(
        "██║   ██║██████╔╝█████╗  ██╔██╗ ██║███████║███████║██║ █╗ ██║█████╔╝ "
    );
    println!(
        "██║   ██║██╔═══╝ ██╔══╝  ██║╚██╗██║██╔══██║██╔══██║██║███╗██║██╔═██╗ "
    );
    println!(
        "╚██████╔╝██║     ███████╗██║ ╚████║██║  ██║██║  ██║╚███╔███╔╝██║  ██╗"
    );
    println!(
        " ╚═════╝ ╚═╝     ╚══════╝╚═╝  ╚═══╝╚═╝  ╚═╝╚═╝  ╚═╝ ╚══╝╚══╝ ╚═╝  ╚═╝"
    );
    println!();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    print_banner();

    // Show first-run message if companion tools are missing
    setup::maybe_first_run();

    let cmd = HawkCommand::parse();
    run(cmd).await
}

async fn run(cmd: HawkCommand) -> anyhow::Result<()> {
    match cmd {
        HawkCommand::Run { agent_command } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let manager = hawk_core::agent_manager::AgentManager::new(db);
            let manifest = hawk_core::manifest::AgentManifest {
                info: hawk_core::manifest::AgentInfo {
                    name: agent_command.clone(),
                    version: "0.0.0".to_string(),
                    description: String::new(),
                    framework: String::new(),
                    entry_command: agent_command,
                },
                permissions: Default::default(),
                resources: Default::default(),
                llm: Default::default(),
                talon_requirements: Default::default(),
                capabilities: Default::default(),
            };
            let pid = manager.spawn(manifest).await?;
            println!("Agent started: pid={pid}");
        }

        HawkCommand::Stop { process_id } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let manager = hawk_core::agent_manager::AgentManager::new(db);
            let result = manager.stop(process_id).await?;
            if result.forced {
                println!("Agent {process_id} force-killed.");
            } else {
                println!("Agent {process_id} stopped.");
            }
        }

        HawkCommand::Pause { process_id } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let manager = hawk_core::agent_manager::AgentManager::new(db);
            manager.pause(process_id)?;
            println!("Agent {process_id} paused.");
        }

        HawkCommand::Resume { process_id } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let manager = hawk_core::agent_manager::AgentManager::new(db);
            manager.resume(process_id)?;
            println!("Agent {process_id} resumed.");
        }

        HawkCommand::Ps => {
            let db = hawk_core::db::init_database(&db_path())?;
            let manager = hawk_core::agent_manager::AgentManager::new(db);
            print_ps(&manager.list());
        }

        HawkCommand::Undo { snapshot_id } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let snap_dir = hawk_core::agent_manager::snapshot_dir();
            let engine = hawk_savepoint::SnapshotEngine::new(db, snap_dir);
            let result = match snapshot_id {
                Some(id) => engine.rollback(&id)?,
                None => anyhow::bail!("No snapshot-id provided. Usage: hawk undo <snapshot-id>"),
            };
            println!(
                "Rolled back to snapshot {} ({} files restored).",
                result.snapshot_id, result.files_restored
            );
        }

        HawkCommand::Diff { snapshot_id } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let snap_dir = hawk_core::agent_manager::snapshot_dir();
            let engine = hawk_savepoint::SnapshotEngine::new(db, snap_dir);
            let diffs = engine.diff(&snapshot_id)?;
            print_diff(&diffs);
        }

        HawkCommand::Vault(vault_cmd) => {
            use hawk_vault::{AuthCredential, SecretsVault, Vault};
            let mut vault = Vault::new(Vault::default_path());
            let token = vault.authenticate(AuthCredential::SystemKeychain)?;
            match vault_cmd {
                VaultCommand::Set { key, value } => {
                    vault.set(&key, value.as_bytes(), &token)?;
                    println!("Vault: set {key}");
                }
                VaultCommand::Get { key } => {
                    let bytes = vault.get(&key, &token)?;
                    // inject into environment — never print the value
                    std::env::set_var(&key, String::from_utf8_lossy(&bytes).as_ref());
                    println!("Vault: {key} injected into environment.");
                }
                VaultCommand::Rm { key } => {
                    vault.delete(&key, &token)?;
                    println!("Vault: removed {key}");
                }
                VaultCommand::List => {
                    let keys = vault.list_keys();
                    if keys.is_empty() {
                        println!("Vault is empty.");
                    } else {
                        for k in &keys {
                            println!("{k}");
                        }
                    }
                }
            }
        }

        HawkCommand::Config(config_cmd) => {
            use hawk_core::config_engine::{ConfigScope, LayeredConfig};
            let config = LayeredConfig::load(None)?;
            match config_cmd {
                ConfigCommand::Show => {
                    let keys = [
                        "core.log_level",
                        "core.session_retention_days",
                        "core.pattern_retention_days",
                        "privacy.mode",
                        "savepoint.auto_snapshot",
                        "savepoint.max_snapshots_per_agent",
                        "healing.max_retries",
                        "healing.enabled",
                    ];
                    for key in &keys {
                        match config.get_effective(key) {
                            Some(v) => println!("{key} = {}", v.value),
                            None => println!("{key} = (not set)"),
                        }
                    }
                }
                ConfigCommand::Set { key, value } => {
                    config.set(&key, &value, ConfigScope::Global)?;
                    println!("Config: {key} = {value}");
                }
                ConfigCommand::Llm => {
                    print_llm_providers(&config.merged().llm.providers);
                }
            }
        }

        HawkCommand::Bus(bus_cmd) => match bus_cmd {
            BusCommand::Inspect => {
                let bus = hawk_bus::MessageBus::new();
                let info = bus.inspect();
                print!("{}", hawk_bus::format_inspection(&info));
            }
        },

        HawkCommand::Verify { session_id, transcript, project_dir, baseline, retest } => {
            use hawk_verify::{claimcheck_available, VerificationEngine};
            let db = hawk_core::db::init_database(&db_path())?;
            let engine = VerificationEngine::new(db);

            // Use real claimcheck binary when a transcript file is provided
            if let Some(ref transcript_path) = transcript {
                let t = std::path::PathBuf::from(transcript_path);
                let p = project_dir.as_deref().unwrap_or(".");
                let proj = std::path::PathBuf::from(p);

                if !claimcheck_available() {
                    eprintln!("claimcheck not found. Install it to verify real transcripts:");
                    eprintln!("  cargo install --git https://github.com/ojuschugh1/claimcheck");
                    eprintln!("Falling back to SQLite session_actions verification.\n");
                    let claims = load_claims_from_session(&engine.db, &session_id)?;
                    let report = engine.verify_session(&session_id, claims)?;
                    print_verification_report(&report);
                } else {
                    let report = engine.verify_with_claimcheck(
                        &session_id,
                        &t,
                        &proj,
                        baseline.as_deref(),
                        retest,
                        None,
                    )?;
                    print_verification_report(&report);
                }
            } else {
                // No transcript — use SQLite session_actions fallback
                if claimcheck_available() {
                    println!("tip: pass --transcript <file.jsonl> to use the real claimcheck binary");
                    println!("     claimcheck checks files on disk, git history, and lockfiles.\n");
                }
                let claims = load_claims_from_session(&engine.db, &session_id)
                    .unwrap_or_default();
                if claims.is_empty() {
                    println!("Session: {session_id}");
                    println!("Status: no actions recorded");
                    println!();
                    println!("No claims recorded.");
                } else {
                    let report = engine.verify_session(&session_id, claims)?;
                    print_verification_report(&report);
                }
            }
        }

        HawkCommand::Stats(stats_cmd) => match stats_cmd {
            StatsCommand::Show => {
                println!("not yet implemented");
            }
            StatsCommand::Tokens => {
                use hawk_compress::{CompressionEngine, SqzEngine, sqz_available, sqz_stats_raw, sqz_gain_raw};
                let engine = SqzEngine::new();
                let stats = engine.get_stats();

                // if sqz is installed, show its own stats (richer, from its SQLite db)
                if sqz_available() {
                    println!("sqz is installed — showing real compression stats:\n");
                    if let Some(raw) = sqz_stats_raw() {
                        println!("{raw}");
                    }
                    println!();
                    if let Some(gain) = sqz_gain_raw() {
                        println!("Daily savings:\n{gain}");
                    }
                } else {
                    println!("sqz not found — showing hawk-compress fallback stats.");
                    println!("Install sqz for real compression: curl -fsSL https://raw.githubusercontent.com/ojuschugh1/sqz/main/install.sh | sh\n");
                    print_token_stats(&stats);
                }
            }
            StatsCommand::Cost => {
                use hawk_core::token_tracker::TokenTracker;
                use std::collections::HashMap;

                let db = hawk_core::db::init_database(&db_path())?;
                let tracker = TokenTracker::new(db);
                let all_stats = tracker.get_all_stats()?;

                let mut trend_map: HashMap<u32, Vec<hawk_core::token_tracker::DailyTokenUsage>> =
                    HashMap::new();
                for (pid, _) in &all_stats {
                    let trend = tracker.get_7day_trend(*pid)?;
                    trend_map.insert(*pid, trend);
                }

                print_cost_stats(&all_stats, &trend_map);
            }
        },

        HawkCommand::Watch(watch_cmd) => match watch_cmd {
            WatchCommand::Report => {
                let db = hawk_core::db::init_database(&db_path())?;
                let engine = hawk_watch::WatchEngine::new(db);
                let report = engine.generate_report()?;
                print!("{}", hawk_watch::format_report(&report));
            }
            WatchCommand::Scan { agent, path } => {
                use hawk_watch::{ghostdep_available, etch_available};
                let db = hawk_core::db::init_database(&db_path())?;
                let engine = hawk_watch::WatchEngine::new(db);
                let project_path = std::path::PathBuf::from(&path);

                if ghostdep_available() {
                    println!("Running ghostdep on {}...", project_path.display());
                    let count = engine.run_ghostdep_scan(&agent, &project_path)?;
                    if count == 0 {
                        println!("No phantom or unused dependencies found.");
                    } else {
                        println!("Found {count} dependency issue(s). Run 'hawk watch report' to see details.");
                    }
                } else {
                    println!("ghostdep not found. Install it to enable dependency scanning:");
                    println!("  curl -fsSL https://raw.githubusercontent.com/ojuschugh1/ghostdep/main/install.sh | sh");
                }

                if etch_available() {
                    println!("\nRunning etch test on {}...", project_path.display());
                    let count = engine.run_etch_scan(&project_path)?;
                    if count == 0 {
                        println!("No API drift detected.");
                    } else {
                        println!("Found {count} API drift(s). Run 'hawk watch report' to see details.");
                    }
                } else {
                    println!("\netch not found. Install it to enable API drift detection:");
                    println!("  go install github.com/ojuschugh1/etch/cmd/etch@latest");
                }
            }
            _ => println!("not yet implemented"),
        },

        HawkCommand::Eye => {
            hawk_ui::run()?;
        }

        HawkCommand::Sdk(sdk_cmd) => match sdk_cmd {
            SdkCommand::Info => {
                println!("hawk-sdk-rust v{}", env!("CARGO_PKG_VERSION"));
                println!("Supported languages: rust, python, typescript");
            }
            SdkCommand::Init { language, name, output } => {
                use hawk_sdk_rust::scaffold;
                use std::fs;
                use std::path::PathBuf;

                let result = scaffold::generate(&language, &name);
                match result {
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                    Ok(files) => {
                        let base: PathBuf = output
                            .map(PathBuf::from)
                            .unwrap_or_else(|| PathBuf::from("."));
                        let agent_dir = base.join(&name);
                        for (rel_path, content) in &files.files {
                            let full = agent_dir.join(rel_path);
                            if let Some(parent) = full.parent() {
                                fs::create_dir_all(parent)?;
                            }
                            fs::write(&full, content)?;
                            println!("  created {}", full.display());
                        }
                        println!("Scaffold generated in {}/", agent_dir.display());
                    }
                }
            }
        },

        HawkCommand::Orchestrate { task_description } => {
            use hawk_core::orchestrator::Orchestrator;
            use hawk_core::db::init_database;

            let db = init_database(&db_path())?;
            let bus = hawk_bus::MessageBus::new();
            let mut orchestrator = Orchestrator::with_bus(bus.clone());

            // Load registered agents from the database and register their capabilities.
            {
                let mut stmt = db.prepare(
                    "SELECT pid, name FROM agents WHERE state = 'Running'",
                )?;
                let rows = stmt.query_map([], |row| {
                    Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (pid, name) = row?;
                    orchestrator.register_agent(pid, name, vec![]);
                }
            }

            println!("Orchestrating: {task_description}");
            let plan = orchestrator
                .orchestrate(&task_description)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            println!("Sub-tasks ({}):", plan.subtasks.len());
            for (i, st) in plan.subtasks.iter().enumerate() {
                let agent = st
                    .assigned_agent
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "unassigned".to_string());
                println!("  [{i}] {} → agent {agent}", st.description);
            }
            if !plan.dependencies.is_empty() {
                println!("Dependencies:");
                for (dep, dependent) in &plan.dependencies {
                    println!("  [{dep}] must complete before [{dependent}]");
                }
            }

            println!("\nExecuting plan...");
            let report = orchestrator
                .execute_plan(plan)
                .map_err(|e| anyhow::anyhow!("{e}"))?;

            for (i, st) in report.plan.subtasks.iter().enumerate() {
                let status = match &st.status {
                    hawk_core::orchestrator::SubTaskStatus::Completed => "✓ completed".to_string(),
                    hawk_core::orchestrator::SubTaskStatus::Failed(e) => format!("✗ failed: {e}"),
                    hawk_core::orchestrator::SubTaskStatus::Running => "⟳ running".to_string(),
                    hawk_core::orchestrator::SubTaskStatus::Pending => "· pending".to_string(),
                };
                println!("  [{i}] {status}");
            }
            println!("\n{}", report.summary);
        }

        HawkCommand::Replay { session_id, step } => {
            let db = hawk_core::db::init_database(&db_path())?;
            let recorder = hawk_core::session_recorder::SessionRecorder::new(db);
            match step {
                None => {
                    let log = recorder.get_log(&session_id)?;
                    if log.is_empty() {
                        println!("No actions recorded for session {session_id}.");
                    } else {
                        println!("Session: {session_id}");
                        println!("{}", "-".repeat(72));
                        for action in &log {
                            println!(
                                "Step {:>4}  {}  pid={:<6}  {}",
                                action.step_number,
                                action.timestamp,
                                action.agent_pid,
                                action.action_type,
                            );
                        }
                    }
                }
                Some(n) => {
                    let state = recorder.get_state_at_step(&session_id, n as u32)?;
                    println!("Session: {session_id}  (state at step {n})");
                    println!("{}", "-".repeat(72));
                    if state.actions_up_to_step.is_empty() {
                        println!("No actions up to step {n}.");
                    } else {
                        for action in &state.actions_up_to_step {
                            println!(
                                "Step {:>4}  {}  pid={:<6}  {}",
                                action.step_number,
                                action.timestamp,
                                action.agent_pid,
                                action.action_type,
                            );
                        }
                        println!("{}", "-".repeat(72));
                        let last = state.actions_up_to_step.last().unwrap();
                        println!("Context at step {n}:");
                        println!("  Last action : {}", last.action_type);
                        println!("  Payload     : {}", last.payload);
                    }
                }
            }
        }

        HawkCommand::Nest(nest_cmd) => {
            use hawk_nest::{NestClient, make_signature};
            let db = hawk_core::db::init_database(&db_path())?;
            let client = NestClient::new(db);
            match nest_cmd {
                NestCommand::Search { query } => {
                    let results = client.search(&query)?;
                    if results.is_empty() {
                        println!("No packages found for '{query}'.");
                    } else {
                        println!(
                            "{:<24} {:<10} {:<16} {:<12} {:<10} {}",
                            "NAME", "VERSION", "AUTHOR", "TYPE", "DOWNLOADS", "COMPATIBILITY"
                        );
                        println!("{}", "-".repeat(90));
                        for p in &results {
                            println!(
                                "{:<24} {:<10} {:<16} {:<12} {:<10} {}",
                                p.name,
                                p.version,
                                p.author,
                                format!("{:?}", p.package_type),
                                p.download_count,
                                p.compatibility,
                            );
                            println!("  {}", p.description);
                        }
                    }
                }
                NestCommand::Install { name } => {
                    let results = client.search(&name)?;
                    let listing = results
                        .into_iter()
                        .find(|p| p.name == name)
                        .ok_or_else(|| anyhow::anyhow!("Package '{name}' not found in index."))?;
                    let sig = make_signature(&name, &listing.version);
                    match client.install(&name, &listing, &sig) {
                        Ok(()) => println!("Installed '{name}' v{}.", listing.version),
                        Err(hawk_nest::NestError::AlreadyInstalled(_)) => {
                            println!("'{name}' is already installed.");
                        }
                        Err(e) => return Err(anyhow::anyhow!("{e}")),
                    }
                }
                NestCommand::Publish { path } => {
                    let pkg_path = std::path::PathBuf::from(&path);
                    match client.publish(&pkg_path) {
                        Ok(()) => println!("Package at '{path}' validated and published."),
                        Err(e) => {
                            eprintln!("Publish failed: {e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
        }

        HawkCommand::Sync(sync_cmd) => {
            use hawk_sync::{ConflictStrategy, SyncEngine, SyncItem};
            let mut engine = SyncEngine::new();
            match sync_cmd {
                SyncCommand::Enable { shared_secret } => {
                    engine.enable(&shared_secret).map_err(|e| anyhow::anyhow!("{e}"))?;
                    println!("Sync enabled.");
                }
                SyncCommand::Select { item } => {
                    let sync_item = if item.starts_with("memory:") {
                        SyncItem::MemoryNamespace(item["memory:".len()..].to_string())
                    } else {
                        SyncItem::Agent(item.clone())
                    };
                    engine.select_for_sync(sync_item);
                    println!("Selected '{item}' for sync.");
                }
                SyncCommand::Status => {
                    let peers = engine.discover_peers();
                    if peers.is_empty() {
                        println!("No paired devices found.");
                        println!("Pending changes: {}", engine.get_queued_count());
                    } else {
                        println!(
                            "{:<24} {:<24} {:<14} {}",
                            "DEVICE ID", "LAST SYNC", "STATUS", "PENDING"
                        );
                        println!("{}", "-".repeat(70));
                        for p in &peers {
                            let last = p.last_sync.as_deref().unwrap_or("never");
                            let status = match p.status {
                                hawk_sync::PeerStatus::Connected => "Connected",
                                hawk_sync::PeerStatus::Disconnected => "Disconnected",
                            };
                            println!(
                                "{:<24} {:<24} {:<14} {}",
                                p.device_id, last, status, p.pending_changes
                            );
                        }
                    }
                }
                SyncCommand::Peers => {
                    let peers = engine.discover_peers();
                    if peers.is_empty() {
                        println!("No peers discovered on LAN.");
                    } else {
                        for p in &peers {
                            println!("{}", p.device_id);
                        }
                    }
                }
                SyncCommand::Resolve { strategy } => {
                    let s = match strategy.as_str() {
                        "manual" => ConflictStrategy::Manual,
                        "last-write" | "last-writer-wins" => ConflictStrategy::LastWriterWins,
                        "merge" => ConflictStrategy::Merge,
                        other => {
                            eprintln!("Unknown strategy '{other}'. Use: manual, last-write, merge");
                            std::process::exit(1);
                        }
                    };
                    engine.set_conflict_strategy(s);
                    println!("Conflict strategy set to '{strategy}'.");
                }
            }
        }

        HawkCommand::Trust { .. } => {
            println!("not yet implemented");
        }

        HawkCommand::Healing(healing_cmd) => {
            use hawk_core::self_healer::SelfHealer;
            let db = hawk_core::db::init_database(&db_path())?;
            let healer = SelfHealer::new(db, 3);
            match healing_cmd {
                HealingCommand::Status => {
                    println!("not yet implemented");
                }
                HealingCommand::History { agent_id } => {
                    let events = healer
                        .get_history(agent_id)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    if events.is_empty() {
                        println!("No healing events for agent {agent_id}.");
                    } else {
                        println!(
                            "{:<4} {:<28} {:<20} {:<18} {:<8} {}",
                            "ID", "TIMESTAMP", "ERROR", "ADJUSTMENT", "ATTEMPT", "OUTCOME"
                        );
                        println!("{}", "-".repeat(90));
                        for ev in &events {
                            println!(
                                "{:<4} {:<28} {:<20} {:<18} {:<8} {}",
                                ev.id,
                                ev.timestamp,
                                truncate(&ev.original_error, 18),
                                ev.adjustment,
                                ev.attempt_number,
                                ev.outcome,
                            );
                        }
                    }
                }
            }
        }

        HawkCommand::Patterns(patterns_cmd) => {
            use hawk_core::pattern_detector::PatternDetector;
            use hawk_core::config_engine::LayeredConfig;

            let db = hawk_core::db::init_database(&db_path())?;
            let retention = LayeredConfig::load(None)
                .ok()
                .map(|c| c.merged().core.pattern_retention_days)
                .unwrap_or(90);
            let detector = PatternDetector::new(db, retention);

            match patterns_cmd {
                PatternsCommand::List => {
                    let patterns = detector.list_patterns()
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    if patterns.is_empty() {
                        println!("No patterns detected.");
                    } else {
                        println!(
                            "{:<38} {:<8} {:<28} {}",
                            "ID", "COUNT", "LAST OCCURRENCE", "STATUS"
                        );
                        println!("{}", "-".repeat(82));
                        for p in &patterns {
                            println!(
                                "{:<38} {:<8} {:<28} {}",
                                p.id,
                                p.occurrence_count,
                                p.last_occurrence,
                                p.status,
                            );
                            let seq_display = p.action_sequence.join(" → ");
                            println!("  {seq_display}");
                        }
                    }
                }
                PatternsCommand::Accept { pattern_id } => {
                    let manifest = detector.accept_pattern(&pattern_id)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    println!("Pattern {pattern_id} accepted. Generated Agent_Manifest:");
                    println!("{manifest}");
                }
                PatternsCommand::Decline { pattern_id } => {
                    detector.decline_pattern(&pattern_id)
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    println!("Pattern {pattern_id} declined. Use 'hawk patterns reset' to re-enable.");
                }
                PatternsCommand::Reset => {
                    let count = detector.reset_declined()
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    println!("Re-enabled {count} declined pattern(s).");
                }
            }
        }

        HawkCommand::Talon(talon_cmd) => {
            use hawk_core::talon::{Capability, TalonRegistry, make_signature};
            let registry = TalonRegistry::new();
            match talon_cmd {
                TalonCommand::Install { name } => {
                    // Simulate download from HawkNest: derive a valid signature for the
                    // installed version. In production this would come from the registry.
                    let version = "0.1.0";
                    let sig = make_signature(&name, version);
                    match registry.install(&name, version, &sig, vec![]) {
                        Ok(()) => {
                            registry.load(&name).ok();
                            println!("Talon '{name}' installed and loaded.");
                        }
                        Err(hawk_core::talon::TalonError::InvalidSignature(_)) => {
                            eprintln!(
                                "SECURITY WARNING: signature verification failed for '{name}'. Installation rejected."
                            );
                        }
                        Err(e) => eprintln!("Error: {e}"),
                    }
                }
                TalonCommand::List => {
                    let mut talons = registry.list();
                    if talons.is_empty() {
                        println!("No Talons installed.");
                    } else {
                        talons.sort_by(|a, b| a.name.cmp(&b.name));
                        println!(
                            "{:<20} {:<10} {:<12} {}",
                            "NAME", "VERSION", "STATUS", "CAPABILITIES"
                        );
                        println!("{}", "-".repeat(60));
                        for t in &talons {
                            let status = match &t.status {
                                hawk_core::talon::TalonStatus::Loaded => "Loaded".to_string(),
                                hawk_core::talon::TalonStatus::Unloaded => "Unloaded".to_string(),
                                hawk_core::talon::TalonStatus::Failed(r) => {
                                    format!("Failed({})", r)
                                }
                            };
                            let caps: Vec<&str> =
                                t.capabilities.iter().map(|c| c.name.as_str()).collect();
                            println!(
                                "{:<20} {:<10} {:<12} {}",
                                t.name,
                                t.version,
                                status,
                                caps.join(", ")
                            );
                        }
                    }
                }
                TalonCommand::Remove { name } => {
                    println!("Talon '{name}' removed.");
                }
            }
        }

        HawkCommand::Setup { yes, force, only, skip } => {
            let opts = setup::SetupOptions {
                skip_installed: !force,
                force_update: force,
                yes,
                only,
                skip,
            };
            setup::run_setup(opts)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<HawkCommand, clap::Error> {
        HawkCommand::try_parse_from(args)
    }

    #[test]
    fn run_parses_command_string() {
        let cmd = parse(&["hawk", "run", "python agent.py"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Run { agent_command } if agent_command == "python agent.py"));
    }

    #[test]
    fn run_requires_argument() {
        assert!(parse(&["hawk", "run"]).is_err());
    }

    #[test]
    fn stop_parses_pid() {
        let cmd = parse(&["hawk", "stop", "42"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Stop { process_id: 42 }));
    }

    #[test]
    fn stop_rejects_non_numeric_pid() {
        assert!(parse(&["hawk", "stop", "abc"]).is_err());
    }

    #[test]
    fn stop_requires_argument() {
        assert!(parse(&["hawk", "stop"]).is_err());
    }

    #[test]
    fn pause_parses_pid() {
        let cmd = parse(&["hawk", "pause", "7"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Pause { process_id: 7 }));
    }

    #[test]
    fn resume_parses_pid() {
        let cmd = parse(&["hawk", "resume", "99"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Resume { process_id: 99 }));
    }

    #[test]
    fn ps_parses_no_args() {
        let cmd = parse(&["hawk", "ps"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Ps));
    }

    #[test]
    fn undo_with_snapshot_id() {
        let cmd = parse(&["hawk", "undo", "snap-abc-123"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Undo { snapshot_id: Some(ref id) } if id == "snap-abc-123"));
    }

    #[test]
    fn undo_without_snapshot_id() {
        let cmd = parse(&["hawk", "undo"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Undo { snapshot_id: None }));
    }

    #[test]
    fn diff_parses_snapshot_id() {
        let cmd = parse(&["hawk", "diff", "snap-xyz"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Diff { snapshot_id } if snapshot_id == "snap-xyz"));
    }

    #[test]
    fn diff_requires_argument() {
        assert!(parse(&["hawk", "diff"]).is_err());
    }

    #[test]
    fn vault_set_parses_key_value() {
        let cmd = parse(&["hawk", "vault", "set", "MY_KEY", "my-value"]).unwrap();
        assert!(matches!(
            cmd,
            HawkCommand::Vault(VaultCommand::Set { ref key, ref value })
            if key == "MY_KEY" && value == "my-value"
        ));
    }

    #[test]
    fn vault_get_parses_key() {
        let cmd = parse(&["hawk", "vault", "get", "MY_KEY"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Vault(VaultCommand::Get { ref key }) if key == "MY_KEY"));
    }

    #[test]
    fn vault_rm_parses_key() {
        let cmd = parse(&["hawk", "vault", "rm", "MY_KEY"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Vault(VaultCommand::Rm { ref key }) if key == "MY_KEY"));
    }

    #[test]
    fn vault_list_parses() {
        let cmd = parse(&["hawk", "vault", "list"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Vault(VaultCommand::List)));
    }

    #[test]
    fn vault_set_requires_both_args() {
        assert!(parse(&["hawk", "vault", "set", "KEY_ONLY"]).is_err());
    }

    #[test]
    fn config_show_parses() {
        let cmd = parse(&["hawk", "config", "show"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Config(ConfigCommand::Show)));
    }

    #[test]
    fn config_set_parses_key_value() {
        let cmd = parse(&["hawk", "config", "set", "core.log_level", "debug"]).unwrap();
        assert!(matches!(
            cmd,
            HawkCommand::Config(ConfigCommand::Set { ref key, ref value })
            if key == "core.log_level" && value == "debug"
        ));
    }

    #[test]
    fn config_set_requires_both_args() {
        assert!(parse(&["hawk", "config", "set", "core.log_level"]).is_err());
    }

    #[test]
    fn config_llm_parses() {
        let cmd = parse(&["hawk", "config", "llm"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Config(ConfigCommand::Llm)));
    }

    #[test]
    fn print_llm_providers_empty_no_panic() {
        print_llm_providers(&[]);
    }

    #[test]
    fn print_llm_providers_with_entries_no_panic() {
        let providers = vec![
            hawk_core::config::LlmProvider {
                name: "openai".to_string(),
                endpoint: "https://api.openai.com/v1".to_string(),
                priority: 1,
            },
            hawk_core::config::LlmProvider {
                name: "ollama".to_string(),
                endpoint: "http://localhost:11434".to_string(),
                priority: 2,
            },
        ];
        print_llm_providers(&providers);
    }

    #[test]
    fn ping_provider_local_is_available() {
        assert_eq!(ping_provider("http://localhost:11434"), "available");
        assert_eq!(ping_provider("http://127.0.0.1:8080"), "available");
    }

    #[test]
    fn ping_provider_remote_is_unknown() {
        assert_eq!(ping_provider("https://api.openai.com/v1"), "unknown");
    }

    #[test]
    fn eye_parses() {
        let cmd = parse(&["hawk", "eye"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Eye));
    }

    #[test]
    fn verify_parses_session_id() {
        let cmd = parse(&["hawk", "verify", "sess-001"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Verify { ref session_id, .. } if session_id == "sess-001"));
    }

    #[test]
    fn replay_parses_session_id() {
        let cmd = parse(&["hawk", "replay", "sess-002"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Replay { session_id, step: None } if session_id == "sess-002"));
    }

    #[test]
    fn replay_parses_step_flag() {
        let cmd = parse(&["hawk", "replay", "sess-003", "--step", "5"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Replay { session_id, step: Some(5) } if session_id == "sess-003"));
    }

    #[test]
    fn orchestrate_parses_task_description() {
        let cmd = parse(&["hawk", "orchestrate", "research quantum computing"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Orchestrate { task_description } if task_description == "research quantum computing"));
    }

    #[test]
    fn trust_parses_agent_name() {
        let cmd = parse(&["hawk", "trust", "my-agent"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Trust { agent_name } if agent_name == "my-agent"));
    }

    #[test]
    fn healing_history_parses_agent_id() {
        let cmd = parse(&["hawk", "healing", "history", "42"]).unwrap();
        assert!(matches!(
            cmd,
            HawkCommand::Healing(HealingCommand::History { agent_id: 42 })
        ));
    }

    #[test]
    fn healing_history_requires_agent_id() {
        assert!(parse(&["hawk", "healing", "history"]).is_err());
    }

    #[test]
    fn healing_status_parses() {
        let cmd = parse(&["hawk", "healing", "status"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Healing(HealingCommand::Status)));
    }

    #[test]
    fn unknown_subcommand_produces_error() {
        let err = parse(&["hawk", "nonexistent"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn no_subcommand_produces_error() {
        let err = parse(&["hawk"]).unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn watch_report_parses() {
        let cmd = parse(&["hawk", "watch", "report"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Watch(WatchCommand::Report)));
    }

    #[test]
    fn talon_list_parses() {
        let cmd = parse(&["hawk", "talon", "list"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Talon(TalonCommand::List)));
    }

    #[test]
    fn talon_install_parses_name() {
        let cmd = parse(&["hawk", "talon", "install", "browser-talon"]).unwrap();
        assert!(
            matches!(cmd, HawkCommand::Talon(TalonCommand::Install { ref name }) if name == "browser-talon")
        );
    }

    #[test]
    fn talon_install_requires_name() {
        assert!(parse(&["hawk", "talon", "install"]).is_err());
    }

    #[test]
    fn talon_remove_parses_name() {
        let cmd = parse(&["hawk", "talon", "remove", "old-talon"]).unwrap();
        assert!(
            matches!(cmd, HawkCommand::Talon(TalonCommand::Remove { ref name }) if name == "old-talon")
        );
    }

    #[test]
    fn format_uptime_zero() {
        assert_eq!(format_uptime(std::time::Duration::from_secs(0)), "00:00:00");
    }

    #[test]
    fn format_uptime_one_hour() {
        assert_eq!(
            format_uptime(std::time::Duration::from_secs(3600 + 23 * 60 + 45)),
            "01:23:45"
        );
    }

    #[test]
    fn print_ps_empty_no_panic() {
        print_ps(&[]);
    }

    #[test]
    fn print_diff_empty_no_panic() {
        print_diff(&[]);
    }

    // ── sync CLI parsing tests ────────────────────────────────────────────────

    #[test]
    fn sync_enable_parses_secret() {
        let cmd = parse(&["hawk", "sync", "enable", "my-secret"]).unwrap();
        assert!(matches!(
            cmd,
            HawkCommand::Sync(SyncCommand::Enable { ref shared_secret })
            if shared_secret == "my-secret"
        ));
    }

    #[test]
    fn sync_enable_requires_secret() {
        assert!(parse(&["hawk", "sync", "enable"]).is_err());
    }

    #[test]
    fn sync_select_parses_item() {
        let cmd = parse(&["hawk", "sync", "select", "my-agent"]).unwrap();
        assert!(matches!(
            cmd,
            HawkCommand::Sync(SyncCommand::Select { ref item })
            if item == "my-agent"
        ));
    }

    #[test]
    fn sync_select_requires_item() {
        assert!(parse(&["hawk", "sync", "select"]).is_err());
    }

    #[test]
    fn sync_status_parses() {
        let cmd = parse(&["hawk", "sync", "status"]).unwrap();
        assert!(matches!(cmd, HawkCommand::Sync(SyncCommand::Status)));
    }

    #[test]
    fn sync_resolve_parses_strategy() {
        let cmd = parse(&["hawk", "sync", "resolve", "--strategy", "manual"]).unwrap();
        assert!(matches!(
            cmd,
            HawkCommand::Sync(SyncCommand::Resolve { ref strategy })
            if strategy == "manual"
        ));
    }

    #[test]
    fn sync_resolve_requires_strategy_flag() {
        assert!(parse(&["hawk", "sync", "resolve"]).is_err());
    }
}
