use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("database error: {0}")]
    Database(String),
}

impl From<rusqlite::Error> for TokenError {
    fn from(e: rusqlite::Error) -> Self {
        TokenError::Database(e.to_string())
    }
}

pub struct AgentTokenStats {
    pub agent_pid: u32,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost: f64,
}

pub struct DailyTokenUsage {
    pub date: String, // YYYY-MM-DD
    pub tokens: u64,
    pub cost: f64,
}

pub struct TokenTracker {
    db: Connection,
}

impl TokenTracker {
    pub fn new(db: Connection) -> Self {
        Self { db }
    }

    pub fn record(
        &self,
        agent_pid: u32,
        provider: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
        pricing: Option<f64>,
    ) -> Result<(), TokenError> {
        let estimated_cost =
            pricing.map(|price| (prompt_tokens + completion_tokens) as f64 * price);
        self.db.execute(
            "INSERT INTO token_usage (agent_pid, timestamp, provider, prompt_tokens, completion_tokens, estimated_cost) \
             VALUES (?1, datetime('now'), ?2, ?3, ?4, ?5)",
            params![agent_pid, provider, prompt_tokens, completion_tokens, estimated_cost],
        )?;
        Ok(())
    }

    pub fn get_agent_stats(&self, agent_pid: u32) -> Result<AgentTokenStats, TokenError> {
        let (prompt, completion, cost) = self.db.query_row(
            "SELECT COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), COALESCE(SUM(estimated_cost), 0.0) \
             FROM token_usage WHERE agent_pid = ?1",
            params![agent_pid],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, f64>(2)?)),
        )?;
        let total_prompt = prompt as u64;
        let total_completion = completion as u64;
        Ok(AgentTokenStats {
            agent_pid,
            total_prompt_tokens: total_prompt,
            total_completion_tokens: total_completion,
            total_tokens: total_prompt + total_completion,
            estimated_cost: cost,
        })
    }

    pub fn get_all_stats(&self) -> Result<Vec<(u32, AgentTokenStats)>, TokenError> {
        let mut stmt = self.db.prepare(
            "SELECT agent_pid, COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0), COALESCE(SUM(estimated_cost), 0.0) \
             FROM token_usage GROUP BY agent_pid ORDER BY agent_pid",
        )?;
        let rows = stmt.query_map([], |row| {
            let pid = row.get::<_, i64>(0)? as u32;
            let prompt = row.get::<_, i64>(1)? as u64;
            let completion = row.get::<_, i64>(2)? as u64;
            let cost = row.get::<_, f64>(3)?;
            Ok((pid, prompt, completion, cost))
        })?;
        let mut result = Vec::new();
        for row in rows {
            let (pid, prompt, completion, cost) = row?;
            result.push((
                pid,
                AgentTokenStats {
                    agent_pid: pid,
                    total_prompt_tokens: prompt,
                    total_completion_tokens: completion,
                    total_tokens: prompt + completion,
                    estimated_cost: cost,
                },
            ));
        }
        Ok(result)
    }

    pub fn get_7day_trend(&self, agent_pid: u32) -> Result<Vec<DailyTokenUsage>, TokenError> {
        let mut stmt = self.db.prepare(
            "SELECT date(timestamp) AS day, COALESCE(SUM(prompt_tokens + completion_tokens), 0), COALESCE(SUM(estimated_cost), 0.0) \
             FROM token_usage WHERE agent_pid = ?1 AND timestamp >= datetime('now', '-7 days') \
             GROUP BY day ORDER BY day ASC",
        )?;
        let rows = stmt.query_map(params![agent_pid], |row| {
            Ok(DailyTokenUsage {
                date: row.get(0)?,
                tokens: row.get::<_, i64>(1)? as u64,
                cost: row.get(2)?,
            })
        })?;
        let mut trend = Vec::new();
        for row in rows {
            trend.push(row?);
        }
        Ok(trend)
    }

    pub fn check_budget(&self, agent_pid: u32, budget_tokens: u64) -> Result<bool, TokenError> {
        let total: i64 = self.db.query_row(
            "SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0) FROM token_usage WHERE agent_pid = ?1",
            params![agent_pid],
            |row| row.get(0),
        )?;
        Ok(total as u64 > budget_tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init_database;
    use tempfile::NamedTempFile;

    fn make_tracker() -> (NamedTempFile, TokenTracker) {
        let f = NamedTempFile::new().unwrap();
        let conn = init_database(f.path()).unwrap();
        (f, TokenTracker::new(conn))
    }

    #[test]
    fn record_stores_correct_values() {
        let (_f, tracker) = make_tracker();
        tracker.record(1, "openai", 100, 50, Some(0.00003)).unwrap();
        let stats = tracker.get_agent_stats(1).unwrap();
        assert_eq!(stats.total_prompt_tokens, 100);
        assert_eq!(stats.total_completion_tokens, 50);
        assert_eq!(stats.total_tokens, 150);
    }

    #[test]
    fn record_without_pricing_stores_zero_cost() {
        let (_f, tracker) = make_tracker();
        tracker.record(2, "ollama", 200, 100, None).unwrap();
        let stats = tracker.get_agent_stats(2).unwrap();
        assert_eq!(stats.total_tokens, 300);
        assert!((stats.estimated_cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn multiple_records_accumulate() {
        let (_f, tracker) = make_tracker();
        tracker.record(3, "openai", 100, 50, Some(0.00003)).unwrap();
        tracker
            .record(3, "openai", 200, 100, Some(0.00003))
            .unwrap();
        let stats = tracker.get_agent_stats(3).unwrap();
        assert_eq!(stats.total_tokens, 450);
    }

    #[test]
    fn cost_calculation_uses_pricing() {
        let (_f, tracker) = make_tracker();
        let price = 0.00003_f64;
        tracker.record(4, "openai", 1000, 500, Some(price)).unwrap();
        let stats = tracker.get_agent_stats(4).unwrap();
        let expected = 1500.0 * price;
        assert!((stats.estimated_cost - expected).abs() < 1e-9);
    }

    #[test]
    fn get_all_stats_returns_per_agent() {
        let (_f, tracker) = make_tracker();
        tracker
            .record(10, "openai", 100, 50, Some(0.00003))
            .unwrap();
        tracker.record(11, "ollama", 200, 100, None).unwrap();
        let all = tracker.get_all_stats().unwrap();
        assert_eq!(all.len(), 2);
        let pids: Vec<u32> = all.iter().map(|(p, _)| *p).collect();
        assert!(pids.contains(&10));
        assert!(pids.contains(&11));
    }

    #[test]
    fn check_budget_returns_false_when_under() {
        let (_f, tracker) = make_tracker();
        tracker.record(20, "openai", 100, 50, None).unwrap();
        assert!(!tracker.check_budget(20, 1000).unwrap());
    }

    #[test]
    fn check_budget_returns_true_when_over() {
        let (_f, tracker) = make_tracker();
        tracker.record(21, "openai", 600, 500, None).unwrap();
        assert!(tracker.check_budget(21, 1000).unwrap());
    }

    #[test]
    fn check_budget_exact_limit_not_exceeded() {
        let (_f, tracker) = make_tracker();
        tracker.record(22, "openai", 500, 500, None).unwrap();
        assert!(!tracker.check_budget(22, 1000).unwrap());
    }

    #[test]
    fn check_budget_no_records_not_exceeded() {
        let (_f, tracker) = make_tracker();
        assert!(!tracker.check_budget(99, 100).unwrap());
    }

    #[test]
    fn get_7day_trend_returns_empty_for_no_records() {
        let (_f, tracker) = make_tracker();
        assert!(tracker.get_7day_trend(50).unwrap().is_empty());
    }

    #[test]
    fn get_7day_trend_includes_recent_records() {
        let (_f, tracker) = make_tracker();
        tracker
            .record(30, "openai", 100, 50, Some(0.00003))
            .unwrap();
        tracker
            .record(30, "openai", 200, 100, Some(0.00003))
            .unwrap();
        let trend = tracker.get_7day_trend(30).unwrap();
        assert!(!trend.is_empty());
        let total_tokens: u64 = trend.iter().map(|d| d.tokens).sum();
        assert_eq!(total_tokens, 450);
    }

    #[test]
    fn get_7day_trend_date_format_is_yyyy_mm_dd() {
        let (_f, tracker) = make_tracker();
        tracker.record(31, "openai", 10, 5, None).unwrap();
        let trend = tracker.get_7day_trend(31).unwrap();
        assert!(!trend.is_empty());
        let date = &trend[0].date;
        assert_eq!(date.len(), 10, "date should be YYYY-MM-DD: {date}");
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }

    #[test]
    fn get_7day_trend_cost_matches_records() {
        let (_f, tracker) = make_tracker();
        let price = 0.00003_f64;
        tracker.record(32, "openai", 100, 50, Some(price)).unwrap();
        let trend = tracker.get_7day_trend(32).unwrap();
        assert!(!trend.is_empty());
        let total_cost: f64 = trend.iter().map(|d| d.cost).sum();
        let expected = 150.0 * price;
        assert!((total_cost - expected).abs() < 1e-9);
    }
}
