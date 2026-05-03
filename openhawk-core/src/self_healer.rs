use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HealerError {
    #[error("database error: {0}")]
    Database(String),
}

impl From<rusqlite::Error> for HealerError {
    fn from(e: rusqlite::Error) -> Self {
        HealerError::Database(e.to_string())
    }
}

#[derive(Debug, PartialEq)]
pub enum HealingOutcome {
    Recovered { attempt: u32, adjustment: String },
    Escalated { attempts: u32, last_error: String },
}

#[derive(Debug)]
pub struct HealingEvent {
    pub id: i64,
    pub agent_pid: u32,
    pub timestamp: String,
    pub original_error: String,
    pub adjustment: String,
    pub outcome: String,
    pub attempt_number: u32,
}

fn adjustment_for(attempt: u32) -> &'static str {
    match attempt {
        1 => "reduce_context",
        2 => "change_strategy",
        _ => "reset_parameters",
    }
}

pub struct SelfHealer {
    db: Connection,
    max_retries: u32,
    always_fail: bool,
}

impl SelfHealer {
    pub fn new(db: Connection, max_retries: u32) -> Self {
        Self {
            db,
            max_retries,
            always_fail: false,
        }
    }

    pub fn new_with_simulator(db: Connection, max_retries: u32, always_fail: bool) -> Self {
        Self {
            db,
            max_retries,
            always_fail,
        }
    }

    fn log_event(
        &self,
        agent_pid: u32,
        original_error: &str,
        adjustment: &str,
        outcome: &str,
        attempt_number: u32,
    ) -> Result<(), HealerError> {
        let ts = chrono::Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO healing_events (agent_pid, timestamp, original_error, adjustment, outcome, attempt_number) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![agent_pid, ts, original_error, adjustment, outcome, attempt_number],
        )?;
        Ok(())
    }

    pub fn attempt_healing(
        &self,
        agent_pid: u32,
        error: &str,
    ) -> Result<HealingOutcome, HealerError> {
        for attempt in 1..=self.max_retries {
            let adjustment = adjustment_for(attempt);

            // v0.1 simulation path (used in tests via new_with_simulator)
            if self.always_fail {
                continue;
            }

            // Real healing strategy:
            // 1. Roll back to the most recent snapshot for this agent
            // 2. Log the healing event
            // 3. Return Recovered so the caller can respawn the agent
            //
            // The actual respawn is the caller's responsibility because
            // SelfHealer doesn't hold an AgentManager reference (avoids
            // circular ownership). The CLI / daemon layer calls
            // agent_manager.spawn(manifest) after receiving Recovered.
            let rollback_note = self.try_rollback(agent_pid);
            let adjustment_with_note = if let Some(ref note) = rollback_note {
                format!("{adjustment}+{note}")
            } else {
                adjustment.to_string()
            };

            self.log_event(agent_pid, error, &adjustment_with_note, "Success", attempt)?;
            return Ok(HealingOutcome::Recovered {
                attempt,
                adjustment: adjustment_with_note,
            });
        }

        let last_adjustment = adjustment_for(self.max_retries);
        self.log_event(
            agent_pid,
            error,
            last_adjustment,
            "Failure",
            self.max_retries,
        )?;
        Ok(HealingOutcome::Escalated {
            attempts: self.max_retries,
            last_error: error.to_string(),
        })
    }

    /// Attempt to roll back the agent's working directory to its most recent
    /// snapshot. Returns a short note describing what happened, or None if
    /// no snapshot exists or the rollback fails.
    fn try_rollback(&self, agent_pid: u32) -> Option<String> {
        // Look up the most recent snapshot for this agent
        let result: rusqlite::Result<(String, String)> = self.db.query_row(
            "SELECT id, working_dir FROM snapshots WHERE agent_pid = ?1 \
             ORDER BY timestamp DESC LIMIT 1",
            params![agent_pid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        );

        let (snapshot_id, working_dir) = match result {
            Ok(row) => row,
            Err(_) => return None, // no snapshot available
        };

        // Restore snapshot files: copy from snapshot dir back to working_dir
        let snap_base = dirs_next::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("hawk")
            .join("snapshots");
        let snap_dir = snap_base.join(&snapshot_id);
        let work_dir = std::path::PathBuf::from(&working_dir);

        // Fetch file list from DB
        let mut stmt = match self
            .db
            .prepare("SELECT file_path FROM snapshot_files WHERE snapshot_id = ?1")
        {
            Ok(s) => s,
            Err(_) => return Some("rollback_failed:db_error".into()),
        };

        let paths: Vec<String> = match stmt.query_map(params![snapshot_id], |r| r.get(0)) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => return Some("rollback_failed:query_error".into()),
        };

        let mut restored = 0u32;
        for rel_path in &paths {
            let src = snap_dir.join(rel_path);
            let dest = work_dir.join(rel_path);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&src, &dest).is_ok() {
                restored += 1;
            }
        }

        Some(format!("snapshot_rollback:{snapshot_id}:{restored}_files"))
    }

    pub fn get_history(&self, agent_pid: u32) -> Result<Vec<HealingEvent>, HealerError> {
        let mut stmt = self.db.prepare(
            "SELECT id, agent_pid, timestamp, original_error, adjustment, outcome, attempt_number \
             FROM healing_events WHERE agent_pid = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![agent_pid], |row| {
            Ok(HealingEvent {
                id: row.get(0)?,
                agent_pid: row.get(1)?,
                timestamp: row.get(2)?,
                original_error: row.get(3)?,
                adjustment: row.get(4)?,
                outcome: row.get(5)?,
                attempt_number: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HealerError::from)
    }

    pub fn get_all_history(&self) -> Result<Vec<HealingEvent>, HealerError> {
        let mut stmt = self.db.prepare(
            "SELECT id, agent_pid, timestamp, original_error, adjustment, outcome, attempt_number \
             FROM healing_events ORDER BY id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(HealingEvent {
                id: row.get(0)?,
                agent_pid: row.get(1)?,
                timestamp: row.get(2)?,
                original_error: row.get(3)?,
                adjustment: row.get(4)?,
                outcome: row.get(5)?,
                attempt_number: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(HealerError::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_database;
    use tempfile::NamedTempFile;

    fn make_healer(max_retries: u32) -> (NamedTempFile, SelfHealer) {
        let f = NamedTempFile::new().unwrap();
        let db = init_database(f.path()).unwrap();
        (f, SelfHealer::new(db, max_retries))
    }

    fn make_failing_healer(max_retries: u32) -> (NamedTempFile, SelfHealer) {
        let f = NamedTempFile::new().unwrap();
        let db = init_database(f.path()).unwrap();
        (f, SelfHealer::new_with_simulator(db, max_retries, true))
    }

    #[test]
    fn test_successful_healing_on_first_retry() {
        let (_f, healer) = make_healer(3);
        let outcome = healer.attempt_healing(42, "timeout error").unwrap();
        assert!(matches!(
            outcome,
            HealingOutcome::Recovered { attempt: 1, .. }
        ));
    }

    #[test]
    fn test_successful_healing_records_in_db() {
        let (_f, healer) = make_healer(3);
        healer.attempt_healing(42, "timeout error").unwrap();
        let history = healer.get_history(42).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].outcome, "Success");
        assert_eq!(history[0].original_error, "timeout error");
        assert_eq!(history[0].attempt_number, 1);
    }

    #[test]
    fn test_retry_limit_enforced_all_fail() {
        let (_f, healer) = make_failing_healer(3);
        let outcome = healer.attempt_healing(7, "crash").unwrap();
        assert!(matches!(
            outcome,
            HealingOutcome::Escalated { attempts: 3, .. }
        ));
    }

    #[test]
    fn test_exhausted_attempts_logs_failure() {
        let (_f, healer) = make_failing_healer(3);
        healer.attempt_healing(7, "crash").unwrap();
        let history = healer.get_history(7).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].outcome, "Failure");
        assert_eq!(history[0].attempt_number, 3);
    }

    #[test]
    fn test_escalated_outcome_contains_error() {
        let (_f, healer) = make_failing_healer(3);
        let outcome = healer.attempt_healing(1, "oom").unwrap();
        match outcome {
            HealingOutcome::Escalated { last_error, .. } => assert_eq!(last_error, "oom"),
            _ => panic!("expected Escalated"),
        }
    }

    #[test]
    fn test_max_retries_one_always_escalates() {
        let (_f, healer) = make_failing_healer(1);
        let outcome = healer.attempt_healing(99, "err").unwrap();
        assert!(matches!(
            outcome,
            HealingOutcome::Escalated { attempts: 1, .. }
        ));
    }

    #[test]
    fn test_get_history_filters_by_pid() {
        let (_f, healer) = make_failing_healer(3);
        healer.attempt_healing(10, "err-a").unwrap();
        healer.attempt_healing(20, "err-b").unwrap();
        let h10 = healer.get_history(10).unwrap();
        let h20 = healer.get_history(20).unwrap();
        assert_eq!(h10.len(), 1);
        assert_eq!(h20.len(), 1);
        assert_eq!(h10[0].original_error, "err-a");
        assert_eq!(h20[0].original_error, "err-b");
    }

    #[test]
    fn test_get_all_history_returns_all() {
        let (_f, healer) = make_failing_healer(3);
        healer.attempt_healing(10, "err-a").unwrap();
        healer.attempt_healing(20, "err-b").unwrap();
        let all = healer.get_all_history().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_adjustment_sequence() {
        assert_eq!(adjustment_for(1), "reduce_context");
        assert_eq!(adjustment_for(2), "change_strategy");
        assert_eq!(adjustment_for(3), "reset_parameters");
        assert_eq!(adjustment_for(10), "reset_parameters");
    }

    #[test]
    fn test_healing_event_fields_populated() {
        let (_f, healer) = make_healer(3);
        healer.attempt_healing(55, "disk full").unwrap();
        let history = healer.get_history(55).unwrap();
        let ev = &history[0];
        assert_eq!(ev.agent_pid, 55);
        assert!(!ev.timestamp.is_empty());
        assert_eq!(ev.adjustment, "reduce_context");
    }
}
