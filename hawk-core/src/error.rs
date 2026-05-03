use thiserror::Error;

#[derive(Debug, Error)]
pub enum HawkError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Permission denied: {0}")]
    Permission(String),

    #[error("Snapshot error: {0}")]
    Snapshot(String),

    #[error("Vault error: {0}")]
    Vault(String),

    #[error("Bus error: {0}")]
    Bus(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),
}
