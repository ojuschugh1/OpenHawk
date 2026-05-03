use rusqlite::{Connection, params};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PatternError {
    #[error("database error: {0}")]
    Database(String),
    #[error("pattern not found: {0}")]
    NotFound(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<rusqlite::Error> for PatternError {
    fn from(e: rusqlite::Error) -> Self {
        PatternError::Database(e.to_string())
    }
}

impl From<serde_json::Error> for PatternError {
    fn from(e: serde_json::Error) -> Self {
        PatternError::Serialization(e.to_string())
    }
}

pub struct DetectedPattern {
    pub id: String,
    pub action_sequence: Vec<String>,
    pub occurrence_count: u32,
}

pub struct PatternRecord {
    pub id: String,
    pub action_sequence: Vec<String>,
    pub occurrence_count: u32,
    pub last_occurrence: String,
    pub status: String,
}

pub struct PatternDetector {
    db: Connection,
    retention_days: u32,
    pub action_log: Vec<String>,
}

impl PatternDetector {
    pub fn new(db: Connection, retention_days: u32) -> Self {
        Self { db, retention_days, action_log: Vec::new() }
    }

    pub fn record_action(&mut self, action: &str) {
        self.action_log.push(action.to_string());
    }

    pub fn detect_patterns(&mut self) -> Vec<DetectedPattern> {
        let log = &self.action_log;
        let n = log.len();
        if n < 3 { return Vec::new(); }

        let mut results: Vec<DetectedPattern> = Vec::new();

        for window_size in 3..=n {
            let mut counts: std::collections::HashMap<Vec<String>, u32> = std::collections::HashMap::new();
            for start in 0..=(n - window_size) {
                let seq: Vec<String> = log[start..start + window_size].to_vec();
                *counts.entry(seq).or_insert(0) += 1;
            }

            for (seq, count) in counts {
                if count >= 5 {
                    let already = results.iter().any(|p| {
                        p.occurrence_count >= count && is_subsequence(&seq, &p.action_sequence)
                    });
                    if already { continue; }

                    let id = self.upsert_pattern(&seq, count).unwrap_or_else(|_| Uuid::new_v4().to_string());
                    results.push(DetectedPattern { id, action_sequence: seq, occurrence_count: count });
                }
            }
        }

        results
    }

    fn upsert_pattern(&self, seq: &[String], count: u32) -> Result<String, PatternError> {
        let seq_json = serde_json::to_string(seq)?;
        let now = chrono::Utc::now().to_rfc3339();
        let expires = (chrono::Utc::now() + chrono::Duration::days(self.retention_days as i64)).to_rfc3339();

        let existing: Option<String> = self.db.query_row(
            "SELECT id FROM patterns WHERE action_sequence = ?1",
            params![seq_json],
            |row| row.get(0),
        ).ok();

        if let Some(id) = existing {
            self.db.execute(
                "UPDATE patterns SET occurrence_count = ?1, last_occurrence = ?2 WHERE id = ?3",
                params![count, now, id],
            )?;
            Ok(id)
        } else {
            let id = Uuid::new_v4().to_string();
            self.db.execute(
                "INSERT INTO patterns (id, action_sequence, occurrence_count, last_occurrence, status, created_at, expires_at) \
                 VALUES (?1, ?2, ?3, ?4, 'Detected', ?5, ?6)",
                params![id, seq_json, count, now, now, expires],
            )?;
            Ok(id)
        }
    }

    pub fn accept_pattern(&self, pattern_id: &str) -> Result<String, PatternError> {
        let record = self.get_pattern(pattern_id)?;
        self.db.execute("UPDATE patterns SET status = 'Accepted' WHERE id = ?1", params![pattern_id])?;
        Ok(generate_manifest(&record))
    }

    pub fn decline_pattern(&self, pattern_id: &str) -> Result<(), PatternError> {
        let rows = self.db.execute("UPDATE patterns SET status = 'Declined' WHERE id = ?1", params![pattern_id])?;
        if rows == 0 {
            return Err(PatternError::NotFound(pattern_id.to_string()));
        }
        Ok(())
    }

    pub fn reset_declined(&self) -> Result<u64, PatternError> {
        let rows = self.db.execute("UPDATE patterns SET status = 'Detected' WHERE status = 'Declined'", [])?;
        Ok(rows as u64)
    }

    pub fn list_patterns(&self) -> Result<Vec<PatternRecord>, PatternError> {
        let mut stmt = self.db.prepare(
            "SELECT id, action_sequence, occurrence_count, last_occurrence, status \
             FROM patterns ORDER BY last_occurrence DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut records = Vec::new();
        for row in rows {
            let (id, seq_json, count, last, status) = row?;
            let action_sequence: Vec<String> = serde_json::from_str(&seq_json)
                .map_err(|e| PatternError::Serialization(e.to_string()))?;
            records.push(PatternRecord { id, action_sequence, occurrence_count: count, last_occurrence: last, status });
        }
        Ok(records)
    }

    pub fn cleanup_expired(&self) -> Result<u64, PatternError> {
        let now = chrono::Utc::now().to_rfc3339();
        let rows = self.db.execute("DELETE FROM patterns WHERE expires_at < ?1", params![now])?;
        Ok(rows as u64)
    }

    fn get_pattern(&self, pattern_id: &str) -> Result<PatternRecord, PatternError> {
        let (seq_json, count, last, status): (String, u32, String, String) = self.db.query_row(
            "SELECT action_sequence, occurrence_count, last_occurrence, status FROM patterns WHERE id = ?1",
            params![pattern_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).map_err(|_| PatternError::NotFound(pattern_id.to_string()))?;

        let action_sequence: Vec<String> = serde_json::from_str(&seq_json)
            .map_err(|e| PatternError::Serialization(e.to_string()))?;

        Ok(PatternRecord { id: pattern_id.to_string(), action_sequence, occurrence_count: count, last_occurrence: last, status })
    }
}

fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn generate_manifest(record: &PatternRecord) -> String {
    let name = format!("pattern-{}", &record.id[..8]);
    let steps: Vec<String> = record.action_sequence.iter().enumerate()
        .map(|(i, a)| format!("  # step {}: {}", i + 1, a))
        .collect();
    let steps_str = steps.join("\n");

    format!(
        "[agent]\nname = \"{name}\"\nversion = \"1.0.0\"\ndescription = \"Auto-generated from detected pattern (occurrences: {})\"\nframework = \"hawk-pattern\"\nentry_command = \"hawk pattern-run {}\"\n\n[permissions]\nfilesystem = []\nnetwork = []\ncommands = []\nsecrets = []\n\n[resources]\ncpu_percent = 10\nmemory_mb = 128\nmax_open_fds = 16\n\n[pattern]\nsequence = {sequence}\noccurrence_count = {count}\n\n# Recorded action sequence:\n{steps_str}\n",
        record.occurrence_count,
        record.id,
        sequence = serde_json::to_string(&record.action_sequence).unwrap_or_default(),
        count = record.occurrence_count,
        steps_str = steps_str,
    )
}
