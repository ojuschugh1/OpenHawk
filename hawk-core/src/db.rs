use std::path::Path;

use rusqlite::{Connection, params};

use crate::error::HawkError;

pub fn init_database(path: &Path) -> Result<Connection, HawkError> {
    let conn = Connection::open(path)
        .map_err(|e| HawkError::Database(e.to_string()))?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(|e| HawkError::Database(e.to_string()))?;

    conn.execute_batch(SCHEMA)
        .map_err(|e| HawkError::Database(e.to_string()))?;

    migrate(&conn)?;

    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), HawkError> {
    let version: i64 = conn
        .query_row(
            "SELECT version FROM schema_version ORDER BY version DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if version < 1 {
        conn.execute(
            "INSERT INTO schema_version (version, applied_at) VALUES (?1, datetime('now'))",
            params![1],
        )
        .map_err(|e| HawkError::Database(e.to_string()))?;
    }

    Ok(())
}

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
    pid             INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    state           TEXT NOT NULL CHECK(state IN ('Starting','Running','Paused','Stopping','Stopped','Failed')),
    manifest_path   TEXT NOT NULL,
    started_at      TEXT NOT NULL,
    stopped_at      TEXT,
    session_id      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    started_at  TEXT NOT NULL,
    ended_at    TEXT,
    status      TEXT NOT NULL DEFAULT 'Active'
);

CREATE TABLE IF NOT EXISTS session_actions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT NOT NULL REFERENCES sessions(id),
    step_number INTEGER NOT NULL,
    timestamp   TEXT NOT NULL,
    action_type TEXT NOT NULL,
    agent_pid   INTEGER NOT NULL,
    payload     TEXT NOT NULL,
    UNIQUE(session_id, step_number)
);

CREATE TABLE IF NOT EXISTS snapshots (
    id               TEXT PRIMARY KEY,
    timestamp        TEXT NOT NULL,
    agent_pid        INTEGER NOT NULL,
    task_description TEXT,
    file_count       INTEGER NOT NULL,
    strategy         TEXT NOT NULL,
    working_dir      TEXT NOT NULL,
    session_id       TEXT NOT NULL REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS snapshot_files (
    snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
    file_path   TEXT NOT NULL,
    hash        TEXT NOT NULL,
    size_bytes  INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, file_path)
);

CREATE TABLE IF NOT EXISTS token_usage (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_pid         INTEGER NOT NULL,
    timestamp         TEXT NOT NULL,
    provider          TEXT NOT NULL,
    prompt_tokens     INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    estimated_cost    REAL
);

CREATE TABLE IF NOT EXISTS verification_reports (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id     TEXT NOT NULL REFERENCES sessions(id),
    timestamp      TEXT NOT NULL,
    overall_status TEXT NOT NULL,
    report_json    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS healing_events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_pid      INTEGER NOT NULL,
    timestamp      TEXT NOT NULL,
    original_error TEXT NOT NULL,
    adjustment     TEXT NOT NULL,
    outcome        TEXT NOT NULL,
    attempt_number INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS patterns (
    id               TEXT PRIMARY KEY,
    action_sequence  TEXT NOT NULL,
    occurrence_count INTEGER NOT NULL DEFAULT 0,
    last_occurrence  TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'Detected',
    created_at       TEXT NOT NULL,
    expires_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS installed_packages (
    name         TEXT PRIMARY KEY,
    version      TEXT NOT NULL,
    package_type TEXT NOT NULL,
    signature    TEXT NOT NULL,
    installed_at TEXT NOT NULL,
    capabilities TEXT
);

CREATE TABLE IF NOT EXISTS sync_peers (
    device_id       TEXT PRIMARY KEY,
    last_sync       TEXT,
    status          TEXT NOT NULL DEFAULT 'Disconnected',
    pending_changes INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS sync_queue (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    target_device TEXT NOT NULL,
    data_type     TEXT NOT NULL,
    data_key      TEXT NOT NULL,
    payload       BLOB NOT NULL,
    created_at    TEXT NOT NULL,
    synced_at     TEXT
);

CREATE TABLE IF NOT EXISTS bus_queue (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    target_pid   INTEGER NOT NULL,
    topic        TEXT,
    message_json TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    delivered    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS watch_alerts (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    alert_type   TEXT NOT NULL,
    timestamp    TEXT NOT NULL,
    agent_name   TEXT,
    details_json TEXT NOT NULL,
    acknowledged INTEGER NOT NULL DEFAULT 0
);
";

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_db() -> (NamedTempFile, Connection) {
        let f = NamedTempFile::new().unwrap();
        let conn = init_database(f.path()).unwrap();
        (f, conn)
    }

    #[test]
    fn test_all_tables_created() {
        let (_f, conn) = temp_db();

        let expected = [
            "schema_version",
            "agents",
            "sessions",
            "session_actions",
            "snapshots",
            "snapshot_files",
            "token_usage",
            "verification_reports",
            "healing_events",
            "patterns",
            "installed_packages",
            "sync_peers",
            "sync_queue",
            "bus_queue",
            "watch_alerts",
        ];

        for table in &expected {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "missing table: {table}");
        }
    }

    #[test]
    fn test_idempotent_reinit() {
        let f = NamedTempFile::new().unwrap();
        init_database(f.path()).unwrap();
        let conn = init_database(f.path()).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0);
    }

    #[test]
    fn test_wal_mode_enabled() {
        let (_f, conn) = temp_db();

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }
}
