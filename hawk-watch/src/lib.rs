// hawk-watch: Etch (API drift) + GhostDep (phantom deps) bridge
//
// Etch:     https://github.com/ojuschugh1/etch  (Go binary)
// GhostDep: https://github.com/ojuschugh1/ghostdep  (Rust binary)
//
// Both tools are called as external processes when available.
// The WatchEngine stores results in SQLite so they persist across sessions.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("lock poisoned")]
    LockPoisoned,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldChange {
    pub field: String,
    pub change_type: String, // "added", "removed", "modified"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum WatchAlert {
    ApiDrift {
        endpoint: String,
        field_changes: Vec<FieldChange>,
        timestamp: String,
    },
    PhantomDependency {
        agent_name: String,
        dependency: String,
        timestamp: String,
    },
}

impl WatchAlert {
    pub fn alert_type(&self) -> &'static str {
        match self {
            WatchAlert::ApiDrift { .. } => "api_drift",
            WatchAlert::PhantomDependency { .. } => "phantom_dep",
        }
    }

    pub fn timestamp(&self) -> &str {
        match self {
            WatchAlert::ApiDrift { timestamp, .. } => timestamp,
            WatchAlert::PhantomDependency { timestamp, .. } => timestamp,
        }
    }

    pub fn agent_name(&self) -> Option<&str> {
        match self {
            WatchAlert::PhantomDependency { agent_name, .. } => Some(agent_name),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchReport {
    pub api_drifts: Vec<WatchAlert>,
    pub phantom_deps: Vec<WatchAlert>,
    pub generated_at: String,
}

// ── GhostDep bridge ───────────────────────────────────────────────────────────
//
// ghostdep -p <path> -f json
//
// JSON output shape (array of findings):
// [
//   { "type": "phantom", "name": "axios", "file": "src/api.js", "line": 3, "confidence": "high" },
//   { "type": "unused",  "name": "lodash", "manifest": "package.json", "confidence": "high" }
// ]

/// Returns true if the `ghostdep` binary is on PATH.
pub fn ghostdep_available() -> bool {
    Command::new("ghostdep").arg("--version").output().is_ok()
}

#[derive(Debug, Deserialize)]
struct GhostDepFinding {
    #[serde(rename = "type")]
    finding_type: String, // "phantom" or "unused"
    name: String,
    confidence: Option<String>,
}

/// Run `ghostdep -p <project_path> -f json` and return phantom/unused dep names.
/// Returns an empty vec if ghostdep is not installed or the scan fails.
pub fn ghostdep_scan(project_path: &Path) -> Vec<String> {
    let output = Command::new("ghostdep")
        .args([
            "-p",
            &project_path.to_string_lossy(),
            "-f",
            "json",
            "--quiet",
        ])
        .output();

    let output = match output {
        Ok(o) if o.status.success() || !o.stdout.is_empty() => o,
        _ => return Vec::new(),
    };

    let findings: Vec<GhostDepFinding> = match serde_json::from_slice(&output.stdout) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };

    findings
        .into_iter()
        .filter(|f| {
            // report both phantom (missing from manifest) and unused (declared but never imported)
            (f.finding_type == "phantom" || f.finding_type == "unused")
                && f.confidence.as_deref().unwrap_or("high") == "high"
        })
        .map(|f| format!("[{}] {}", f.finding_type, f.name))
        .collect()
}

// ── Etch bridge ───────────────────────────────────────────────────────────────
//
// Etch runs as a local HTTP proxy. hawk-watch integrates with it by:
//   1. Calling `etch test --ci -f json` to get pending diffs
//   2. Parsing the JSON output into WatchAlert::ApiDrift entries
//
// Etch JSON output shape (from `etch test --ci --format json`):
// {
//   "summary": { "total": 3, "mismatches": 2 },
//   "results": [
//     {
//       "endpoint": "GET http://api.example.com/users",
//       "status": "mismatch",
//       "diffs": [
//         { "field": "body.users[0].role", "before": "admin", "after": "viewer", "severity": "warning" }
//       ]
//     }
//   ]
// }

/// Returns true if the `etch` binary is on PATH.
pub fn etch_available() -> bool {
    Command::new("etch").arg("version").output().is_ok()
}

#[derive(Debug, Deserialize)]
struct EtchResult {
    endpoint: String,
    #[serde(default)]
    diffs: Vec<EtchDiff>,
}

#[derive(Debug, Deserialize)]
struct EtchDiff {
    field: String,
    severity: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EtchReport {
    #[serde(default)]
    results: Vec<EtchResult>,
}

/// Run `etch test --ci` in the given directory and return parsed drift findings.
/// Returns an empty vec if etch is not installed, not initialised, or no diffs found.
pub fn etch_test(project_path: &Path) -> Vec<(String, Vec<FieldChange>)> {
    // etch test --ci exits 1 when mismatches are found, so we don't check status
    let output = Command::new("etch")
        .args(["test", "--ci", "--format", "json"])
        .current_dir(project_path)
        .output();

    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    // etch writes JSON to stdout even on exit 1
    let report: EtchReport = match serde_json::from_slice(&output.stdout) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    report
        .results
        .into_iter()
        .filter(|r| !r.diffs.is_empty())
        .map(|r| {
            let changes = r
                .diffs
                .into_iter()
                .map(|d| FieldChange {
                    field: d.field,
                    change_type: d.severity.unwrap_or_else(|| "modified".to_string()),
                })
                .collect();
            (r.endpoint, changes)
        })
        .collect()
}

// ── WatchEngine ───────────────────────────────────────────────────────────────

struct State {
    monitored_agents: Vec<u32>,
}

pub struct WatchEngine {
    pub db: Arc<Mutex<Connection>>,
    state: Arc<Mutex<State>>,
}

impl WatchEngine {
    pub fn new(db: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            state: Arc::new(Mutex::new(State {
                monitored_agents: Vec::new(),
            })),
        }
    }

    pub fn start_monitoring(&self, agents: &[u32]) -> Result<(), WatchError> {
        let mut state = self.state.lock().map_err(|_| WatchError::LockPoisoned)?;
        state.monitored_agents = agents.to_vec();
        Ok(())
    }

    pub fn stop_monitoring(&self) -> Result<(), WatchError> {
        let mut state = self.state.lock().map_err(|_| WatchError::LockPoisoned)?;
        state.monitored_agents.clear();
        Ok(())
    }

    /// Run `etch test` in `project_path` and store any API drift findings.
    /// Requires `etch` to be installed and a baseline to have been recorded.
    pub fn run_etch_scan(&self, project_path: &Path) -> Result<usize, WatchError> {
        let drifts = etch_test(project_path);
        let count = drifts.len();
        for (endpoint, field_changes) in drifts {
            self.emit_api_drift(&endpoint, field_changes)?;
        }
        Ok(count)
    }

    /// Run `ghostdep` on `project_path` and store any phantom/unused dep findings.
    /// Requires `ghostdep` to be installed.
    pub fn run_ghostdep_scan(
        &self,
        agent_name: &str,
        project_path: &Path,
    ) -> Result<usize, WatchError> {
        let findings = ghostdep_scan(project_path);
        let count = findings.len();
        for dep in findings {
            self.emit_phantom_dep(agent_name, &dep)?;
        }
        Ok(count)
    }

    pub fn emit_api_drift(
        &self,
        endpoint: &str,
        field_changes: Vec<FieldChange>,
    ) -> Result<(), WatchError> {
        let alert = WatchAlert::ApiDrift {
            endpoint: endpoint.to_owned(),
            field_changes,
            timestamp: Utc::now().to_rfc3339(),
        };
        self.store_alert(&alert)
    }

    pub fn emit_phantom_dep(&self, agent_name: &str, dependency: &str) -> Result<(), WatchError> {
        let alert = WatchAlert::PhantomDependency {
            agent_name: agent_name.to_owned(),
            dependency: dependency.to_owned(),
            timestamp: Utc::now().to_rfc3339(),
        };
        self.store_alert(&alert)
    }

    /// Legacy stub-compatible method — now calls the real ghostdep binary.
    pub fn scan_dependencies(&self, agent_name: &str) -> Result<(), WatchError> {
        // try to find the agent's project directory from the DB
        let project_path = self.agent_project_path(agent_name);
        let path = project_path.as_deref().unwrap_or(std::path::Path::new("."));
        self.run_ghostdep_scan(agent_name, path)?;
        Ok(())
    }

    pub fn get_alerts(&self) -> Result<Vec<WatchAlert>, WatchError> {
        let db = self.db.lock().map_err(|_| WatchError::LockPoisoned)?;
        let mut stmt = db.prepare(
            "SELECT alert_type, timestamp, agent_name, details_json FROM watch_alerts ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut alerts = Vec::new();
        for row in rows {
            let (alert_type, timestamp, agent_name, details_json) = row?;
            let alert = match alert_type.as_str() {
                "api_drift" => {
                    let d: ApiDriftDetails = serde_json::from_str(&details_json)?;
                    WatchAlert::ApiDrift {
                        endpoint: d.endpoint,
                        field_changes: d.field_changes,
                        timestamp,
                    }
                }
                "phantom_dep" => {
                    let d: PhantomDepDetails = serde_json::from_str(&details_json)?;
                    WatchAlert::PhantomDependency {
                        agent_name: agent_name.unwrap_or_default(),
                        dependency: d.dependency,
                        timestamp,
                    }
                }
                _ => continue,
            };
            alerts.push(alert);
        }
        Ok(alerts)
    }

    pub fn generate_report(&self) -> Result<WatchReport, WatchError> {
        let alerts = self.get_alerts()?;
        let mut api_drifts = Vec::new();
        let mut phantom_deps = Vec::new();
        for alert in alerts {
            match &alert {
                WatchAlert::ApiDrift { .. } => api_drifts.push(alert),
                WatchAlert::PhantomDependency { .. } => phantom_deps.push(alert),
            }
        }
        Ok(WatchReport {
            api_drifts,
            phantom_deps,
            generated_at: Utc::now().to_rfc3339(),
        })
    }

    fn store_alert(&self, alert: &WatchAlert) -> Result<(), WatchError> {
        let db = self.db.lock().map_err(|_| WatchError::LockPoisoned)?;
        let details_json = match alert {
            WatchAlert::ApiDrift {
                endpoint,
                field_changes,
                ..
            } => serde_json::to_string(&ApiDriftDetails {
                endpoint: endpoint.clone(),
                field_changes: field_changes.clone(),
            })?,
            WatchAlert::PhantomDependency { dependency, .. } => {
                serde_json::to_string(&PhantomDepDetails {
                    dependency: dependency.clone(),
                })?
            }
        };
        db.execute(
            "INSERT INTO watch_alerts (alert_type, timestamp, agent_name, details_json) VALUES (?1, ?2, ?3, ?4)",
            params![alert.alert_type(), alert.timestamp(), alert.agent_name(), details_json],
        )?;
        Ok(())
    }

    /// Try to look up the agent's working directory from the agents table.
    fn agent_project_path(&self, agent_name: &str) -> Option<std::path::PathBuf> {
        let db = self.db.lock().ok()?;
        let path: String = db
            .query_row(
                "SELECT manifest_path FROM agents WHERE name = ?1 ORDER BY started_at DESC LIMIT 1",
                params![agent_name],
                |row| row.get(0),
            )
            .ok()?;
        // manifest_path stores the entry_command; derive the directory from it
        let p = std::path::Path::new(&path);
        p.parent().map(|d| d.to_path_buf())
    }
}

#[derive(Serialize, Deserialize)]
struct ApiDriftDetails {
    endpoint: String,
    field_changes: Vec<FieldChange>,
}

#[derive(Serialize, Deserialize)]
struct PhantomDepDetails {
    dependency: String,
}

pub fn format_report(report: &WatchReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Watch Report — generated at {}\n",
        report.generated_at
    ));
    out.push_str(&"=".repeat(60));
    out.push('\n');

    out.push_str(&format!("\nAPI Drifts ({})\n", report.api_drifts.len()));
    if report.api_drifts.is_empty() {
        out.push_str("  (none)\n");
        if !etch_available() {
            out.push_str("  tip: install etch to enable real API drift detection\n");
            out.push_str("       go install github.com/ojuschugh1/etch/cmd/etch@latest\n");
        }
    } else {
        for alert in &report.api_drifts {
            if let WatchAlert::ApiDrift {
                endpoint,
                field_changes,
                timestamp,
            } = alert
            {
                out.push_str(&format!("  [{timestamp}] {endpoint}\n"));
                for fc in field_changes {
                    out.push_str(&format!("    {} {}\n", fc.change_type, fc.field));
                }
            }
        }
    }

    out.push_str(&format!(
        "\nPhantom Dependencies ({})\n",
        report.phantom_deps.len()
    ));
    if report.phantom_deps.is_empty() {
        out.push_str("  (none)\n");
        if !ghostdep_available() {
            out.push_str("  tip: install ghostdep to enable real dependency scanning\n");
            out.push_str("       cargo install --git https://github.com/ojuschugh1/ghostdep\n");
        }
    } else {
        for alert in &report.phantom_deps {
            if let WatchAlert::PhantomDependency {
                agent_name,
                dependency,
                timestamp,
            } = alert
            {
                out.push_str(&format!(
                    "  [{timestamp}] agent={agent_name} dep={dependency}\n"
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn engine() -> WatchEngine {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS watch_alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                alert_type TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                agent_name TEXT,
                details_json TEXT NOT NULL,
                acknowledged INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS agents (
                pid INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                state TEXT NOT NULL,
                manifest_path TEXT NOT NULL,
                started_at TEXT NOT NULL,
                session_id TEXT NOT NULL
            );",
        )
        .unwrap();
        WatchEngine::new(db)
    }

    #[test]
    fn emit_api_drift_stores_correct_alert_structure() {
        let e = engine();
        let changes = vec![
            FieldChange {
                field: "user_id".into(),
                change_type: "removed".into(),
            },
            FieldChange {
                field: "account_id".into(),
                change_type: "added".into(),
            },
        ];
        e.emit_api_drift("https://api.example.com/v2/users", changes.clone())
            .unwrap();
        let alerts = e.get_alerts().unwrap();
        assert_eq!(alerts.len(), 1);
        match &alerts[0] {
            WatchAlert::ApiDrift {
                endpoint,
                field_changes,
                ..
            } => {
                assert_eq!(endpoint, "https://api.example.com/v2/users");
                assert_eq!(field_changes, &changes);
            }
            _ => panic!("expected ApiDrift"),
        }
    }

    #[test]
    fn api_drift_alert_has_non_empty_timestamp() {
        let e = engine();
        e.emit_api_drift("https://api.example.com/health", vec![])
            .unwrap();
        let alerts = e.get_alerts().unwrap();
        match &alerts[0] {
            WatchAlert::ApiDrift { timestamp, .. } => assert!(!timestamp.is_empty()),
            _ => panic!("expected ApiDrift"),
        }
    }

    #[test]
    fn start_and_stop_monitoring_do_not_error() {
        let e = engine();
        e.start_monitoring(&[1, 2, 3]).unwrap();
        e.stop_monitoring().unwrap();
    }

    #[test]
    fn stop_monitoring_clears_agent_list() {
        let e = engine();
        e.start_monitoring(&[10, 20]).unwrap();
        e.stop_monitoring().unwrap();
        let state = e.state.lock().unwrap();
        assert!(state.monitored_agents.is_empty());
    }

    #[test]
    fn emit_phantom_dep_flags_unused_package() {
        let e = engine();
        e.emit_phantom_dep("research-agent", "unused-crate")
            .unwrap();
        let alerts = e.get_alerts().unwrap();
        assert_eq!(alerts.len(), 1);
        match &alerts[0] {
            WatchAlert::PhantomDependency {
                agent_name,
                dependency,
                ..
            } => {
                assert_eq!(agent_name, "research-agent");
                assert_eq!(dependency, "unused-crate");
            }
            _ => panic!("expected PhantomDependency"),
        }
    }

    #[test]
    fn scan_dependencies_does_not_panic_without_binary() {
        // ghostdep may or may not be installed — either way should not panic
        let e = engine();
        let _ = e.scan_dependencies("my-agent");
    }

    #[test]
    fn alerts_are_stored_in_sqlite_and_retrieved() {
        let e = engine();
        e.emit_api_drift("https://api.example.com/v1", vec![])
            .unwrap();
        e.emit_phantom_dep("agent-a", "ghost-lib").unwrap();
        assert_eq!(e.get_alerts().unwrap().len(), 2);
    }

    #[test]
    fn multiple_api_drift_alerts_stored_in_order() {
        let e = engine();
        e.emit_api_drift("https://api.example.com/a", vec![])
            .unwrap();
        e.emit_api_drift("https://api.example.com/b", vec![])
            .unwrap();
        let alerts = e.get_alerts().unwrap();
        assert_eq!(alerts.len(), 2);
        match &alerts[0] {
            WatchAlert::ApiDrift { endpoint, .. } => {
                assert_eq!(endpoint, "https://api.example.com/a")
            }
            _ => panic!(),
        }
    }

    #[test]
    fn get_alerts_returns_empty_when_no_alerts() {
        let e = engine();
        assert!(e.get_alerts().unwrap().is_empty());
    }

    #[test]
    fn generate_report_aggregates_api_drifts_and_phantom_deps() {
        let e = engine();
        e.emit_api_drift(
            "https://api.example.com/v1",
            vec![FieldChange {
                field: "id".into(),
                change_type: "modified".into(),
            }],
        )
        .unwrap();
        e.emit_phantom_dep("agent-b", "stale-dep").unwrap();
        e.emit_api_drift("https://api.example.com/v2", vec![])
            .unwrap();
        let report = e.generate_report().unwrap();
        assert_eq!(report.api_drifts.len(), 2);
        assert_eq!(report.phantom_deps.len(), 1);
        assert!(!report.generated_at.is_empty());
    }

    #[test]
    fn generate_report_empty_when_no_alerts() {
        let e = engine();
        let report = e.generate_report().unwrap();
        assert!(report.api_drifts.is_empty());
        assert!(report.phantom_deps.is_empty());
    }

    #[test]
    fn format_report_includes_endpoint_and_dep_info() {
        let e = engine();
        e.emit_api_drift(
            "https://api.example.com/users",
            vec![FieldChange {
                field: "email".into(),
                change_type: "removed".into(),
            }],
        )
        .unwrap();
        e.emit_phantom_dep("coder-agent", "unused-lib").unwrap();
        let report = e.generate_report().unwrap();
        let output = format_report(&report);
        assert!(output.contains("https://api.example.com/users"));
        assert!(output.contains("removed email"));
        assert!(output.contains("coder-agent"));
        assert!(output.contains("unused-lib"));
    }

    #[test]
    fn format_report_shows_none_when_empty() {
        let e = engine();
        let report = e.generate_report().unwrap();
        let output = format_report(&report);
        assert!(output.contains("(none)"));
    }

    #[test]
    fn field_change_types_are_preserved() {
        let e = engine();
        let changes = vec![
            FieldChange {
                field: "a".into(),
                change_type: "added".into(),
            },
            FieldChange {
                field: "b".into(),
                change_type: "removed".into(),
            },
            FieldChange {
                field: "c".into(),
                change_type: "modified".into(),
            },
        ];
        e.emit_api_drift("https://api.example.com/test", changes)
            .unwrap();
        let alerts = e.get_alerts().unwrap();
        if let WatchAlert::ApiDrift { field_changes, .. } = &alerts[0] {
            assert_eq!(field_changes[0].change_type, "added");
            assert_eq!(field_changes[1].change_type, "removed");
            assert_eq!(field_changes[2].change_type, "modified");
        }
    }

    #[test]
    fn etch_available_check_does_not_panic() {
        let _ = etch_available();
    }

    #[test]
    fn ghostdep_available_check_does_not_panic() {
        let _ = ghostdep_available();
    }

    #[test]
    fn ghostdep_scan_returns_empty_on_missing_binary_or_path() {
        // non-existent path — should return empty, not panic
        let result = ghostdep_scan(std::path::Path::new("/tmp/nonexistent-hawk-test-path"));
        assert!(result.is_empty());
    }

    #[test]
    fn etch_test_returns_empty_on_missing_binary_or_path() {
        let result = etch_test(std::path::Path::new("/tmp/nonexistent-hawk-test-path"));
        assert!(result.is_empty());
    }

    #[test]
    fn run_ghostdep_scan_on_real_project_does_not_panic() {
        let e = engine();
        // scan the openhawk workspace itself — ghostdep may or may not be installed
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let _ = e.run_ghostdep_scan("hawk-watch", workspace);
    }
}
