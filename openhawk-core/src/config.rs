use crate::error::HawkError;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, HawkError>;

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HawkConfig {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub privacy: PrivacyConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub savepoint: SavepointConfig,
    #[serde(default)]
    pub bus: BusConfig,
    #[serde(default)]
    pub sync: SyncConfig,
    #[serde(default)]
    pub compress: CompressConfig,
    #[serde(default)]
    pub healing: HealingConfig,
}

// ── Section structs ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub log_level: String,
    pub session_retention_days: u32,
    pub pattern_retention_days: u32,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            session_retention_days: 30,
            pattern_retention_days: 90,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyConfig {
    pub mode: String,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            mode: "standard".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub providers: Vec<LlmProvider>,
    #[serde(default)]
    pub pricing: LlmPricing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProvider {
    pub name: String,
    pub endpoint: String,
    pub priority: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmPricing {
    #[serde(default)]
    pub openai_gpt4_prompt: f64,
    #[serde(default)]
    pub openai_gpt4_completion: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavepointConfig {
    pub auto_snapshot: bool,
    pub max_snapshots_per_agent: u32,
}

impl Default for SavepointConfig {
    fn default() -> Self {
        Self {
            auto_snapshot: true,
            max_snapshots_per_agent: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusConfig {
    pub message_retention_seconds: u64,
    pub max_queue_size: u64,
}

impl Default for BusConfig {
    fn default() -> Self {
        Self {
            message_retention_seconds: 3600,
            max_queue_size: 10000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub enabled: bool,
    pub conflict_strategy: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            conflict_strategy: "last-writer-wins".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressConfig {
    pub token_threshold: u32,
    pub cache_max_entries: u32,
}

impl Default for CompressConfig {
    fn default() -> Self {
        Self {
            token_threshold: 4000,
            cache_max_entries: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealingConfig {
    pub max_retries: u32,
    pub enabled: bool,
}

impl Default for HealingConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            enabled: true,
        }
    }
}

// ── Parse / serialize ─────────────────────────────────────────────────────────

pub fn parse(toml_str: &str) -> Result<HawkConfig> {
    let config: HawkConfig =
        toml::from_str(toml_str).map_err(|e| HawkError::Config(format!("parse error: {e}")))?;
    validate(&config)?;
    Ok(config)
}

pub fn to_toml(config: &HawkConfig) -> Result<String> {
    toml::to_string_pretty(config)
        .map_err(|e| HawkError::Config(format!("serialization error: {e}")))
}

// ── Validation ────────────────────────────────────────────────────────────────

const VALID_LOG_LEVELS: &[&str] = &["error", "warn", "info", "debug", "trace"];
const VALID_PRIVACY_MODES: &[&str] = &["standard", "local-only", "air-gapped"];

fn validate(config: &HawkConfig) -> Result<()> {
    if !VALID_LOG_LEVELS.contains(&config.core.log_level.as_str()) {
        return Err(HawkError::Config(format!(
            "[core] log_level \"{}\" is invalid; expected one of: {}",
            config.core.log_level,
            VALID_LOG_LEVELS.join(", ")
        )));
    }

    if !VALID_PRIVACY_MODES.contains(&config.privacy.mode.as_str()) {
        return Err(HawkError::Config(format!(
            "[privacy] mode \"{}\" is invalid; expected one of: {}",
            config.privacy.mode,
            VALID_PRIVACY_MODES.join(", ")
        )));
    }

    if config.core.session_retention_days == 0 {
        return Err(HawkError::Config(
            "[core] session_retention_days must be greater than 0".to_string(),
        ));
    }

    if config.healing.max_retries < 1 {
        return Err(HawkError::Config(
            "[healing] max_retries must be at least 1".to_string(),
        ));
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_CONFIG: &str = r#"
[core]
log_level = "info"
session_retention_days = 30
pattern_retention_days = 90

[privacy]
mode = "standard"

[llm]
providers = [
    { name = "openai", endpoint = "https://api.openai.com/v1", priority = 1 },
    { name = "ollama", endpoint = "http://localhost:11434", priority = 2 },
]

[llm.pricing]
openai_gpt4_prompt = 0.00003
openai_gpt4_completion = 0.00006

[savepoint]
auto_snapshot = true
max_snapshots_per_agent = 50

[bus]
message_retention_seconds = 3600
max_queue_size = 10000

[sync]
enabled = false
conflict_strategy = "last-writer-wins"

[compress]
token_threshold = 4000
cache_max_entries = 1000

[healing]
max_retries = 3
enabled = true
"#;

    #[test]
    fn test_parse_full_config() {
        let config = parse(FULL_CONFIG).expect("should parse valid config");
        assert_eq!(config.core.log_level, "info");
        assert_eq!(config.core.session_retention_days, 30);
        assert_eq!(config.core.pattern_retention_days, 90);
        assert_eq!(config.privacy.mode, "standard");
        assert_eq!(config.llm.providers.len(), 2);
        assert_eq!(config.llm.providers[0].name, "openai");
        assert_eq!(config.llm.providers[1].priority, 2);
        assert!((config.llm.pricing.openai_gpt4_prompt - 0.00003).abs() < f64::EPSILON);
        assert!(config.savepoint.auto_snapshot);
        assert_eq!(config.savepoint.max_snapshots_per_agent, 50);
        assert_eq!(config.bus.message_retention_seconds, 3600);
        assert_eq!(config.bus.max_queue_size, 10000);
        assert!(!config.sync.enabled);
        assert_eq!(config.sync.conflict_strategy, "last-writer-wins");
        assert_eq!(config.compress.token_threshold, 4000);
        assert_eq!(config.compress.cache_max_entries, 1000);
        assert_eq!(config.healing.max_retries, 3);
        assert!(config.healing.enabled);
    }

    #[test]
    fn test_round_trip() {
        let config = parse(FULL_CONFIG).unwrap();
        let serialized = to_toml(&config).unwrap();
        let reparsed = parse(&serialized).unwrap();
        assert_eq!(config.core.log_level, reparsed.core.log_level);
        assert_eq!(config.privacy.mode, reparsed.privacy.mode);
        assert_eq!(config.llm.providers.len(), reparsed.llm.providers.len());
        assert_eq!(config.healing.max_retries, reparsed.healing.max_retries);
    }

    #[test]
    fn test_defaults() {
        let config = parse("").unwrap();
        assert_eq!(config.core.log_level, "info");
        assert_eq!(config.core.session_retention_days, 30);
        assert_eq!(config.privacy.mode, "standard");
        assert_eq!(config.healing.max_retries, 3);
        assert!(config.healing.enabled);
    }

    #[test]
    fn test_invalid_log_level() {
        let toml = r#"
[core]
log_level = "verbose"
session_retention_days = 30
pattern_retention_days = 90
"#;
        let err = parse(toml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("log_level"),
            "error should mention log_level: {msg}"
        );
        assert!(
            msg.contains("verbose"),
            "error should include the bad value: {msg}"
        );
    }

    #[test]
    fn test_invalid_privacy_mode() {
        let toml = r#"
[privacy]
mode = "cloud-only"
"#;
        let err = parse(toml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("mode"), "error should mention mode: {msg}");
        assert!(
            msg.contains("cloud-only"),
            "error should include the bad value: {msg}"
        );
    }

    #[test]
    fn test_session_retention_zero() {
        let toml = r#"
[core]
log_level = "info"
session_retention_days = 0
pattern_retention_days = 90
"#;
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("session_retention_days"));
    }

    #[test]
    fn test_max_retries_zero() {
        let toml = r#"
[healing]
max_retries = 0
enabled = true
"#;
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("max_retries"));
    }

    #[test]
    fn test_invalid_toml_syntax() {
        let toml = "[core\nlog_level = \"info\"";
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("parse error"));
    }

    #[test]
    fn test_privacy_modes_all_valid() {
        for mode in &["standard", "local-only", "air-gapped"] {
            let toml = format!("[privacy]\nmode = \"{mode}\"");
            assert!(parse(&toml).is_ok(), "mode {mode} should be valid");
        }
    }

    #[test]
    fn test_log_levels_all_valid() {
        for level in &["error", "warn", "info", "debug", "trace"] {
            let toml = format!(
                "[core]\nlog_level = \"{level}\"\nsession_retention_days = 1\npattern_retention_days = 1"
            );
            assert!(parse(&toml).is_ok(), "log_level {level} should be valid");
        }
    }
}
