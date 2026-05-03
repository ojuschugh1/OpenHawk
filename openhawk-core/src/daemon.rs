use std::path::PathBuf;

use rusqlite::Connection;
use thiserror::Error;

use crate::config::HawkConfig;
use crate::config_engine::LayeredConfig;
use crate::db::init_database;
use crate::platform::PlatformConfig;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("database error: {0}")]
    Database(String),
    #[error("config error: {0}")]
    Config(String),
}

impl From<crate::error::HawkError> for DaemonError {
    fn from(e: crate::error::HawkError) -> Self {
        match e {
            crate::error::HawkError::Database(msg) => DaemonError::Database(msg),
            crate::error::HawkError::Config(msg) => DaemonError::Config(msg),
            other => DaemonError::Config(other.to_string()),
        }
    }
}

pub struct DaemonContext {
    pub db_path: PathBuf,
    pub config: HawkConfig,
    pub platform: PlatformConfig,
}

impl DaemonContext {
    pub fn initialize(db_path: PathBuf) -> Result<Self, DaemonError> {
        let platform = PlatformConfig::detect();

        let layered = LayeredConfig::load(None).map_err(|e| DaemonError::Config(e.to_string()))?;
        let config = layered.merged();

        init_database(&db_path).map_err(|e| DaemonError::Database(e.to_string()))?;

        Ok(Self {
            db_path,
            config,
            platform,
        })
    }

    pub fn db(&self) -> Result<Connection, DaemonError> {
        init_database(&self.db_path).map_err(|e| DaemonError::Database(e.to_string()))
    }

    pub fn is_air_gapped(&self) -> bool {
        self.config.privacy.mode == "air-gapped"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn initialize_creates_db_and_returns_context() {
        let f = NamedTempFile::new().unwrap();
        let ctx = DaemonContext::initialize(f.path().to_path_buf()).unwrap();
        assert_eq!(ctx.db_path, f.path());
    }

    #[test]
    fn db_returns_usable_connection() {
        let f = NamedTempFile::new().unwrap();
        let ctx = DaemonContext::initialize(f.path().to_path_buf()).unwrap();
        let conn = ctx.db().unwrap();
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
    fn is_air_gapped_false_by_default() {
        let f = NamedTempFile::new().unwrap();
        let mut ctx = DaemonContext::initialize(f.path().to_path_buf()).unwrap();
        // force standard mode regardless of what ~/.hawk/config.toml says
        ctx.config.privacy.mode = "standard".to_string();
        assert!(!ctx.is_air_gapped());
    }

    #[test]
    fn is_air_gapped_true_when_mode_set() {
        let f = NamedTempFile::new().unwrap();
        let mut ctx = DaemonContext::initialize(f.path().to_path_buf()).unwrap();
        ctx.config.privacy.mode = "air-gapped".to_string();
        assert!(ctx.is_air_gapped());
    }

    #[test]
    fn platform_detected_on_initialize() {
        let f = NamedTempFile::new().unwrap();
        let ctx = DaemonContext::initialize(f.path().to_path_buf()).unwrap();
        let _ = &ctx.platform.os;
        let _ = &ctx.platform.arch;
    }
}
