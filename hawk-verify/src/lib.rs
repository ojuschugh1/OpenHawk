// hawk-verify: ClaimCheck bridge
//
// claimcheck: https://github.com/ojuschugh1/claimcheck
//
// claimcheck <transcript.jsonl> --json --project-dir <path>
//
// JSON output:
// {
//   "truth_score": "67%",
//   "summary": { "total": 4, "pass": 2, "fail": 1, "unverifiable": 1 },
//   "claims": [
//     { "claim_type": "File", "raw_text": "created src/auth.ts", "result": "PASS", "reason": null },
//     { "claim_type": "Package", "raw_text": "installed jsonwebtoken", "result": "FAIL",
//       "reason": "package not found in any lockfile" }
//   ]
// }
//
// Fallback: when claimcheck is not installed, the engine checks session_actions
// in SQLite (the original behaviour).

use std::path::Path;
use std::process::Command;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("database error: {0}")]
    Database(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("claimcheck error: {0}")]
    ClaimCheck(String),
}

impl From<rusqlite::Error> for VerifyError {
    fn from(e: rusqlite::Error) -> Self {
        VerifyError::Database(e.to_string())
    }
}

impl From<serde_json::Error> for VerifyError {
    fn from(e: serde_json::Error) -> Self {
        VerifyError::Serialization(e.to_string())
    }
}

// ── claimcheck binary bridge ──────────────────────────────────────────────────

/// Returns true if the `claimcheck` binary is on PATH.
pub fn claimcheck_available() -> bool {
    Command::new("claimcheck").arg("--help").output().is_ok()
}

/// Raw JSON output from `claimcheck --json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimCheckReport {
    pub truth_score: String,
    pub summary: ClaimCheckSummary,
    pub claims: Vec<ClaimCheckClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimCheckSummary {
    pub total: u32,
    pub pass: u32,
    pub fail: u32,
    pub unverifiable: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimCheckClaim {
    pub claim_type: String,
    pub raw_text: String,
    pub result: String, // "PASS", "FAIL", "UNVERIFIABLE"
    pub reason: Option<String>,
}

/// Run `claimcheck <transcript_path> --json --project-dir <project_dir>`.
/// Optionally pass `--baseline <ref>` and `--retest`.
pub fn run_claimcheck(
    transcript_path: &Path,
    project_dir: &Path,
    baseline: Option<&str>,
    retest: bool,
    test_cmd: Option<&str>,
) -> Result<ClaimCheckReport, VerifyError> {
    let mut cmd = Command::new("claimcheck");
    cmd.arg(transcript_path)
        .arg("--json")
        .arg("--project-dir")
        .arg(project_dir);

    if let Some(b) = baseline {
        cmd.arg("--baseline").arg(b);
    }
    if retest {
        cmd.arg("--retest");
        if let Some(tc) = test_cmd {
            cmd.arg("--test-cmd").arg(tc);
        }
    }

    let output = cmd.output().map_err(|e| VerifyError::ClaimCheck(e.to_string()))?;

    // claimcheck exits 0 even when claims fail — parse stdout regardless
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(VerifyError::ClaimCheck(format!(
            "claimcheck produced no output. stderr: {stderr}"
        )));
    }

    serde_json::from_str(&stdout).map_err(|e| {
        VerifyError::ClaimCheck(format!("failed to parse claimcheck JSON: {e}\noutput: {stdout}"))
    })
}

// ── Domain types (shared between claimcheck bridge and fallback engine) ───────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentClaim {
    pub action_type: String,
    pub resource: String,
    pub claimed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAction {
    pub step_number: i64,
    pub timestamp: String,
    pub action_type: String,
    pub agent_pid: i64,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvidence {
    pub session_id: String,
    pub actions: Vec<SessionAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "verdict", content = "reason")]
pub enum ClaimVerdict {
    Pass,
    Fail,
    Inconclusive { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimResult {
    pub claim: AgentClaim,
    pub verdict: ClaimVerdict,
    pub discrepancies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum VerificationStatus {
    Verified,
    Unverified,
    Inconclusive,
}

impl std::fmt::Display for VerificationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerificationStatus::Verified => write!(f, "Verified"),
            VerificationStatus::Unverified => write!(f, "Unverified"),
            VerificationStatus::Inconclusive => write!(f, "Inconclusive"),
        }
    }
}

/// Full verification report — produced either by claimcheck or the fallback engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationReport {
    pub session_id: String,
    pub overall_status: VerificationStatus,
    pub claims: Vec<ClaimResult>,
    /// Set when the real claimcheck binary was used.
    pub truth_score: Option<String>,
    /// Raw claimcheck output when available.
    pub claimcheck_raw: Option<ClaimCheckReport>,
}

// ── VerificationEngine ────────────────────────────────────────────────────────

pub struct VerificationEngine {
    pub db: Connection,
}

impl VerificationEngine {
    pub fn new(db: Connection) -> Self {
        Self { db }
    }

    /// Verify a session using the real claimcheck binary when available.
    ///
    /// `transcript_path` — path to a `.jsonl` or `.md` transcript file exported
    ///   from Claude Code, Cursor, or any supported tool.
    /// `project_dir` — the project root claimcheck should check against.
    /// `baseline` — git ref for the session window (e.g. "HEAD~3", "main").
    /// `retest` — re-run tests to verify test claims.
    pub fn verify_with_claimcheck(
        &self,
        session_id: &str,
        transcript_path: &Path,
        project_dir: &Path,
        baseline: Option<&str>,
        retest: bool,
        test_cmd: Option<&str>,
    ) -> Result<VerificationReport, VerifyError> {
        let cc = run_claimcheck(transcript_path, project_dir, baseline, retest, test_cmd)?;

        // Map claimcheck results into our domain types
        let claims: Vec<ClaimResult> = cc.claims.iter().map(|c| {
            let verdict = match c.result.as_str() {
                "PASS" => ClaimVerdict::Pass,
                "FAIL" => ClaimVerdict::Fail,
                _ => ClaimVerdict::Inconclusive {
                    reason: c.reason.clone().unwrap_or_else(|| "unverifiable".to_string()),
                },
            };
            let discrepancies = if let Some(ref r) = c.reason {
                vec![r.clone()]
            } else {
                vec![]
            };
            ClaimResult {
                claim: AgentClaim {
                    action_type: c.claim_type.clone(),
                    resource: c.raw_text.clone(),
                    claimed_at: String::new(),
                },
                verdict,
                discrepancies,
            }
        }).collect();

        let overall_status = if cc.summary.fail > 0 {
            VerificationStatus::Unverified
        } else if cc.summary.pass > 0 {
            VerificationStatus::Verified
        } else {
            VerificationStatus::Inconclusive
        };

        let report = VerificationReport {
            session_id: session_id.to_string(),
            overall_status,
            claims,
            truth_score: Some(cc.truth_score.clone()),
            claimcheck_raw: Some(cc),
        };

        self.store_report_full(&report)?;
        Ok(report)
    }

    /// Fallback: verify using session_actions in SQLite (no claimcheck binary needed).
    pub fn verify_session(
        &self,
        session_id: &str,
        claims: Vec<AgentClaim>,
    ) -> Result<VerificationReport, VerifyError> {
        let actions = self.load_actions(session_id)?;
        let evidence = SessionEvidence { session_id: session_id.to_string(), actions };

        let mut results = Vec::with_capacity(claims.len());
        for claim in claims {
            let verdict = self.verify_claim(&claim, &evidence);
            let discrepancies = match &verdict {
                ClaimVerdict::Fail => vec![format!(
                    "no recorded {} action for resource '{}'",
                    claim.action_type, claim.resource
                )],
                _ => vec![],
            };
            results.push(ClaimResult { claim, verdict, discrepancies });
        }

        let overall_status = derive_status(&results);
        let report = VerificationReport {
            session_id: session_id.to_string(),
            overall_status,
            claims: results,
            truth_score: None,
            claimcheck_raw: None,
        };

        self.store_report_full(&report)?;
        Ok(report)
    }

    pub fn verify_claim(&self, claim: &AgentClaim, evidence: &SessionEvidence) -> ClaimVerdict {
        if evidence.actions.is_empty() {
            return ClaimVerdict::Inconclusive { reason: "no evidence".to_string() };
        }
        let matched = evidence.actions.iter().any(|a| {
            a.action_type == claim.action_type && action_matches_resource(a, &claim.resource)
        });
        if matched { ClaimVerdict::Pass } else { ClaimVerdict::Fail }
    }

    fn load_actions(&self, session_id: &str) -> Result<Vec<SessionAction>, VerifyError> {
        let mut stmt = self.db.prepare(
            "SELECT step_number, timestamp, action_type, agent_pid, payload \
             FROM session_actions WHERE session_id = ?1 ORDER BY step_number ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionAction {
                step_number: row.get(0)?,
                timestamp: row.get(1)?,
                action_type: row.get(2)?,
                agent_pid: row.get(3)?,
                payload: row.get(4)?,
            })
        })?;
        let mut actions = Vec::new();
        for row in rows {
            actions.push(row?);
        }
        Ok(actions)
    }

    fn store_report_full(&self, report: &VerificationReport) -> Result<(), VerifyError> {
        let json = serde_json::to_string(report)?;
        self.db.execute(
            "INSERT INTO verification_reports (session_id, timestamp, overall_status, report_json) \
             VALUES (?1, datetime('now'), ?2, ?3)",
            params![report.session_id, report.overall_status.to_string(), json],
        )?;
        Ok(())
    }
}

fn action_matches_resource(action: &SessionAction, resource: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&action.payload) {
        for key in &["resource", "path", "url", "command"] {
            if let Some(val) = v.get(key).and_then(|x| x.as_str()) {
                if val == resource {
                    return true;
                }
            }
        }
    }
    false
}

fn derive_status(results: &[ClaimResult]) -> VerificationStatus {
    if results.is_empty() {
        return VerificationStatus::Inconclusive;
    }
    if results.iter().any(|r| r.verdict == ClaimVerdict::Fail) {
        return VerificationStatus::Unverified;
    }
    if results.iter().all(|r| r.verdict == ClaimVerdict::Pass) {
        VerificationStatus::Verified
    } else {
        VerificationStatus::Inconclusive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn
    }

    const SCHEMA: &str = "
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT,
            status TEXT NOT NULL DEFAULT 'Active'
        );
        CREATE TABLE IF NOT EXISTS session_actions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            step_number INTEGER NOT NULL,
            timestamp TEXT NOT NULL,
            action_type TEXT NOT NULL,
            agent_pid INTEGER NOT NULL,
            payload TEXT NOT NULL,
            UNIQUE(session_id, step_number)
        );
        CREATE TABLE IF NOT EXISTS verification_reports (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL REFERENCES sessions(id),
            timestamp TEXT NOT NULL,
            overall_status TEXT NOT NULL,
            report_json TEXT NOT NULL
        );
    ";

    fn insert_session(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO sessions (id, started_at) VALUES (?1, datetime('now'))",
            params![id],
        ).unwrap();
    }

    fn insert_action(conn: &Connection, session_id: &str, step: i64, action_type: &str, payload: &str) {
        conn.execute(
            "INSERT INTO session_actions \
             (session_id, step_number, timestamp, action_type, agent_pid, payload) \
             VALUES (?1, ?2, datetime('now'), ?3, 1, ?4)",
            params![session_id, step, action_type, payload],
        ).unwrap();
    }

    // ── claimcheck binary availability ────────────────────────────────────────

    #[test]
    fn claimcheck_available_check_does_not_panic() {
        let _ = claimcheck_available();
    }

    #[test]
    fn run_claimcheck_on_nonexistent_transcript_returns_error() {
        if !claimcheck_available() {
            return; // skip when not installed
        }
        let result = run_claimcheck(
            Path::new("/tmp/nonexistent-hawk-transcript.jsonl"),
            Path::new("."),
            None,
            false,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn run_claimcheck_with_real_transcript_when_installed() {
        if !claimcheck_available() {
            return;
        }
        // write a minimal Claude Code JSONL transcript to a temp file
        let dir = tempfile::TempDir::new().unwrap();
        let transcript = dir.path().join("session.jsonl");
        std::fs::write(
            &transcript,
            r#"{"role":"user","content":"Create a file"}
{"role":"assistant","content":"I created /tmp/hawk-test-claimcheck.txt with the content."}
"#,
        ).unwrap();

        let result = run_claimcheck(&transcript, dir.path(), None, false, None);
        // may succeed or fail depending on whether the file exists — just verify it runs
        match result {
            Ok(report) => {
                assert!(!report.truth_score.is_empty());
                // truth_score is either "N/A" or "X%"
            }
            Err(VerifyError::ClaimCheck(_)) => {
                // claimcheck ran but produced unexpected output — acceptable
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    // ── fallback engine (SQLite-based) ────────────────────────────────────────

    #[test]
    fn inconclusive_when_no_evidence() {
        let engine = VerificationEngine::new(in_memory_db());
        let claim = AgentClaim {
            action_type: "file_write".into(),
            resource: "/tmp/out.txt".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        };
        let evidence = SessionEvidence { session_id: "s1".into(), actions: vec![] };
        assert_eq!(
            engine.verify_claim(&claim, &evidence),
            ClaimVerdict::Inconclusive { reason: "no evidence".into() }
        );
    }

    #[test]
    fn pass_when_matching_action_exists() {
        let engine = VerificationEngine::new(in_memory_db());
        let claim = AgentClaim {
            action_type: "file_write".into(),
            resource: "/tmp/out.txt".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        };
        let evidence = SessionEvidence {
            session_id: "s1".into(),
            actions: vec![SessionAction {
                step_number: 1,
                timestamp: "2024-01-01T00:00:00Z".into(),
                action_type: "file_write".into(),
                agent_pid: 42,
                payload: r#"{"path":"/tmp/out.txt"}"#.into(),
            }],
        };
        assert_eq!(engine.verify_claim(&claim, &evidence), ClaimVerdict::Pass);
    }

    #[test]
    fn fail_when_no_matching_action() {
        let engine = VerificationEngine::new(in_memory_db());
        let claim = AgentClaim {
            action_type: "api_call".into(),
            resource: "https://api.openai.com/v1/chat".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        };
        let evidence = SessionEvidence {
            session_id: "s1".into(),
            actions: vec![SessionAction {
                step_number: 1,
                timestamp: "2024-01-01T00:00:00Z".into(),
                action_type: "file_write".into(),
                agent_pid: 42,
                payload: r#"{"path":"/tmp/out.txt"}"#.into(),
            }],
        };
        assert_eq!(engine.verify_claim(&claim, &evidence), ClaimVerdict::Fail);
    }

    #[test]
    fn verify_session_all_pass_returns_verified() {
        let conn = in_memory_db();
        insert_session(&conn, "sess-pass");
        insert_action(&conn, "sess-pass", 1, "file_write", r#"{"path":"/tmp/result.txt"}"#);
        let engine = VerificationEngine::new(conn);
        let claims = vec![AgentClaim {
            action_type: "file_write".into(),
            resource: "/tmp/result.txt".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        }];
        let report = engine.verify_session("sess-pass", claims).unwrap();
        assert_eq!(report.overall_status, VerificationStatus::Verified);
        assert_eq!(report.claims[0].verdict, ClaimVerdict::Pass);
        assert!(report.truth_score.is_none()); // fallback path
    }

    #[test]
    fn verify_session_any_fail_returns_unverified() {
        let conn = in_memory_db();
        insert_session(&conn, "sess-fail");
        insert_action(&conn, "sess-fail", 1, "file_write", r#"{"path":"/tmp/result.txt"}"#);
        let engine = VerificationEngine::new(conn);
        let claims = vec![
            AgentClaim {
                action_type: "file_write".into(),
                resource: "/tmp/result.txt".into(),
                claimed_at: "2024-01-01T00:00:00Z".into(),
            },
            AgentClaim {
                action_type: "api_call".into(),
                resource: "https://api.openai.com/v1/chat".into(),
                claimed_at: "2024-01-01T00:00:00Z".into(),
            },
        ];
        let report = engine.verify_session("sess-fail", claims).unwrap();
        assert_eq!(report.overall_status, VerificationStatus::Unverified);
    }

    #[test]
    fn verify_session_no_actions_returns_inconclusive() {
        let conn = in_memory_db();
        insert_session(&conn, "sess-inc");
        let engine = VerificationEngine::new(conn);
        let claims = vec![AgentClaim {
            action_type: "file_write".into(),
            resource: "/tmp/out.txt".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        }];
        let report = engine.verify_session("sess-inc", claims).unwrap();
        assert_eq!(report.overall_status, VerificationStatus::Inconclusive);
    }

    #[test]
    fn verify_session_stores_report_in_sqlite() {
        let conn = in_memory_db();
        insert_session(&conn, "sess-store");
        insert_action(&conn, "sess-store", 1, "file_write", r#"{"path":"/tmp/x.txt"}"#);
        let engine = VerificationEngine::new(conn);
        let claims = vec![AgentClaim {
            action_type: "file_write".into(),
            resource: "/tmp/x.txt".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        }];
        engine.verify_session("sess-store", claims).unwrap();
        let count: i64 = engine.db.query_row(
            "SELECT COUNT(*) FROM verification_reports WHERE session_id = 'sess-store'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn verify_session_empty_claims_returns_inconclusive() {
        let conn = in_memory_db();
        insert_session(&conn, "sess-empty");
        let engine = VerificationEngine::new(conn);
        let report = engine.verify_session("sess-empty", vec![]).unwrap();
        assert_eq!(report.overall_status, VerificationStatus::Inconclusive);
    }

    #[test]
    fn discrepancies_populated_on_fail() {
        let conn = in_memory_db();
        insert_session(&conn, "sess-disc");
        insert_action(&conn, "sess-disc", 1, "file_write", r#"{"path":"/tmp/other.txt"}"#);
        let engine = VerificationEngine::new(conn);
        let claims = vec![AgentClaim {
            action_type: "file_write".into(),
            resource: "/tmp/missing.txt".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        }];
        let report = engine.verify_session("sess-disc", claims).unwrap();
        assert_eq!(report.claims[0].verdict, ClaimVerdict::Fail);
        assert!(!report.claims[0].discrepancies.is_empty());
    }

    #[test]
    fn api_call_matched_via_url_field() {
        let engine = VerificationEngine::new(in_memory_db());
        let claim = AgentClaim {
            action_type: "api_call".into(),
            resource: "https://api.openai.com/v1/chat".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        };
        let evidence = SessionEvidence {
            session_id: "s1".into(),
            actions: vec![SessionAction {
                step_number: 1,
                timestamp: "2024-01-01T00:00:00Z".into(),
                action_type: "api_call".into(),
                agent_pid: 42,
                payload: r#"{"url":"https://api.openai.com/v1/chat"}"#.into(),
            }],
        };
        assert_eq!(engine.verify_claim(&claim, &evidence), ClaimVerdict::Pass);
    }

    #[test]
    fn command_exec_matched_via_command_field() {
        let engine = VerificationEngine::new(in_memory_db());
        let claim = AgentClaim {
            action_type: "command_exec".into(),
            resource: "python3".into(),
            claimed_at: "2024-01-01T00:00:00Z".into(),
        };
        let evidence = SessionEvidence {
            session_id: "s1".into(),
            actions: vec![SessionAction {
                step_number: 1,
                timestamp: "2024-01-01T00:00:00Z".into(),
                action_type: "command_exec".into(),
                agent_pid: 42,
                payload: r#"{"command":"python3"}"#.into(),
            }],
        };
        assert_eq!(engine.verify_claim(&claim, &evidence), ClaimVerdict::Pass);
    }
}
