use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecorderError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("session not found: {0}")]
    SessionNotFound(String),
}

pub struct SessionRecorder {
    db: Connection,
}

pub struct SessionAction {
    pub id: i64,
    pub session_id: String,
    pub step_number: u32,
    pub timestamp: String,
    pub action_type: String,
    pub agent_pid: u32,
    pub payload: String,
}

pub struct SessionState {
    pub session_id: String,
    pub actions_up_to_step: Vec<SessionAction>,
    pub step: u32,
}

impl SessionRecorder {
    pub fn new(db: Connection) -> Self {
        Self { db }
    }

    pub fn record_action(
        &self,
        session_id: &str,
        agent_pid: u32,
        action_type: &str,
        payload: serde_json::Value,
    ) -> Result<u32, RecorderError> {
        let next_step: u32 = self.db.query_row(
            "SELECT COALESCE(MAX(step_number), 0) + 1 FROM session_actions WHERE session_id = ?1",
            params![session_id],
            |row| row.get::<_, u32>(0),
        )?;

        let timestamp = chrono::Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO session_actions (session_id, step_number, timestamp, action_type, agent_pid, payload) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![session_id, next_step, timestamp, action_type, agent_pid, payload.to_string()],
        )?;

        Ok(next_step)
    }

    pub fn get_log(&self, session_id: &str) -> Result<Vec<SessionAction>, RecorderError> {
        let mut stmt = self.db.prepare(
            "SELECT id, session_id, step_number, timestamp, action_type, agent_pid, payload \
             FROM session_actions WHERE session_id = ?1 ORDER BY step_number ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(SessionAction {
                id: row.get(0)?,
                session_id: row.get(1)?,
                step_number: row.get(2)?,
                timestamp: row.get(3)?,
                action_type: row.get(4)?,
                agent_pid: row.get(5)?,
                payload: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(RecorderError::Database)
    }

    pub fn get_state_at_step(
        &self,
        session_id: &str,
        step: u32,
    ) -> Result<SessionState, RecorderError> {
        let mut stmt = self.db.prepare(
            "SELECT id, session_id, step_number, timestamp, action_type, agent_pid, payload \
             FROM session_actions WHERE session_id = ?1 AND step_number <= ?2 ORDER BY step_number ASC",
        )?;
        let rows = stmt.query_map(params![session_id, step], |row| {
            Ok(SessionAction {
                id: row.get(0)?,
                session_id: row.get(1)?,
                step_number: row.get(2)?,
                timestamp: row.get(3)?,
                action_type: row.get(4)?,
                agent_pid: row.get(5)?,
                payload: row.get(6)?,
            })
        })?;
        let actions = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(RecorderError::Database)?;
        Ok(SessionState {
            session_id: session_id.to_string(),
            actions_up_to_step: actions,
            step,
        })
    }

    pub fn cleanup_old_sessions(&self, retention_days: u32) -> Result<u64, RecorderError> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = self.db.execute(
            "DELETE FROM session_actions WHERE timestamp < ?1",
            params![cutoff_str],
        )?;
        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_database;
    use tempfile::NamedTempFile;

    fn make_recorder() -> (NamedTempFile, SessionRecorder) {
        let f = NamedTempFile::new().unwrap();
        let conn = init_database(f.path()).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, started_at, status) VALUES ('sess-1', datetime('now'), 'Active')",
            [],
        ).unwrap();
        (f, SessionRecorder::new(conn))
    }

    #[test]
    fn record_action_stores_correct_data() {
        let (_f, rec) = make_recorder();
        let payload = serde_json::json!({"path": "/tmp/foo.txt"});
        let step = rec
            .record_action("sess-1", 42, "file_write", payload)
            .unwrap();
        assert_eq!(step, 1);
        let log = rec.get_log("sess-1").unwrap();
        assert_eq!(log.len(), 1);
        let action = &log[0];
        assert_eq!(action.session_id, "sess-1");
        assert_eq!(action.step_number, 1);
        assert_eq!(action.action_type, "file_write");
        assert_eq!(action.agent_pid, 42);
        assert!(!action.timestamp.is_empty());
        assert!(action.payload.contains("/tmp/foo.txt"));
    }

    #[test]
    fn step_numbers_are_sequential() {
        let (_f, rec) = make_recorder();
        let s1 = rec
            .record_action("sess-1", 1, "file_read", serde_json::json!({}))
            .unwrap();
        let s2 = rec
            .record_action("sess-1", 1, "api_call", serde_json::json!({}))
            .unwrap();
        let s3 = rec
            .record_action("sess-1", 1, "msg_sent", serde_json::json!({}))
            .unwrap();
        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(s3, 3);
    }

    #[test]
    fn get_log_returns_chronological_order() {
        let (_f, rec) = make_recorder();
        for action_type in &["file_read", "api_call", "llm_prompt", "llm_response"] {
            rec.record_action("sess-1", 1, action_type, serde_json::json!({}))
                .unwrap();
        }
        let log = rec.get_log("sess-1").unwrap();
        assert_eq!(log.len(), 4);
        for (i, action) in log.iter().enumerate() {
            assert_eq!(action.step_number, (i + 1) as u32);
        }
    }

    #[test]
    fn get_state_at_step_returns_actions_up_to_step() {
        let (_f, rec) = make_recorder();
        for action_type in &["file_read", "file_write", "api_call", "msg_sent"] {
            rec.record_action("sess-1", 1, action_type, serde_json::json!({}))
                .unwrap();
        }
        let state = rec.get_state_at_step("sess-1", 2).unwrap();
        assert_eq!(state.step, 2);
        assert_eq!(state.actions_up_to_step.len(), 2);
        assert_eq!(state.actions_up_to_step[0].action_type, "file_read");
        assert_eq!(state.actions_up_to_step[1].action_type, "file_write");
    }

    #[test]
    fn get_state_at_step_includes_step_itself() {
        let (_f, rec) = make_recorder();
        rec.record_action("sess-1", 1, "file_read", serde_json::json!({}))
            .unwrap();
        rec.record_action("sess-1", 1, "file_write", serde_json::json!({}))
            .unwrap();
        let state = rec.get_state_at_step("sess-1", 1).unwrap();
        assert_eq!(state.actions_up_to_step.len(), 1);
    }

    #[test]
    fn cleanup_removes_old_actions() {
        let f = NamedTempFile::new().unwrap();
        let conn = init_database(f.path()).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, started_at, status) VALUES ('sess-old', datetime('now'), 'Active')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO session_actions (session_id, step_number, timestamp, action_type, agent_pid, payload) \
             VALUES ('sess-old', 1, '2000-01-01T00:00:00+00:00', 'file_read', 1, '{}')",
            [],
        ).unwrap();
        let rec = SessionRecorder::new(conn);
        let deleted = rec.cleanup_old_sessions(30).unwrap();
        assert_eq!(deleted, 1);
        assert!(rec.get_log("sess-old").unwrap().is_empty());
    }

    #[test]
    fn cleanup_preserves_recent_actions() {
        let (_f, rec) = make_recorder();
        rec.record_action("sess-1", 1, "file_read", serde_json::json!({}))
            .unwrap();
        let deleted = rec.cleanup_old_sessions(30).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(rec.get_log("sess-1").unwrap().len(), 1);
    }
}
