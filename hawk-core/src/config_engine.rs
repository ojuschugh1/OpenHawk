use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{self, HawkConfig};
use crate::error::HawkError;

pub type Result<T> = std::result::Result<T, HawkError>;

pub enum ConfigScope {
    Global,
    Project,
    Agent(String),
}

pub struct ConfigValue {
    pub value: String,
    pub source: ConfigScope,
}

pub struct LayeredConfig {
    global: Option<HawkConfig>,
    project: Option<HawkConfig>,
    global_path: PathBuf,
    project_path: Option<PathBuf>,
}

impl LayeredConfig {
    pub fn load(project_dir: Option<&Path>) -> Result<Self> {
        let global_path = global_config_path()?;
        let global = load_optional(&global_path)?;

        let (project, project_path) = if let Some(dir) = project_dir {
            let p = dir.join("hawk.toml");
            let cfg = load_optional(&p)?;
            (cfg, Some(p))
        } else {
            (None, None)
        };

        Ok(Self {
            global,
            project,
            global_path,
            project_path,
        })
    }

    /// Returns the effective value for a dot-notation key, with source annotation.
    /// Priority: project > global (agent-level is handled by callers via manifest).
    pub fn get_effective(&self, key: &str) -> Option<ConfigValue> {
        if let Some(proj) = &self.project {
            if let Some(v) = extract(proj, key) {
                return Some(ConfigValue {
                    value: v,
                    source: ConfigScope::Project,
                });
            }
        }
        if let Some(glob) = &self.global {
            if let Some(v) = extract(glob, key) {
                return Some(ConfigValue {
                    value: v,
                    source: ConfigScope::Global,
                });
            }
        }
        None
    }

    /// Persists `value` for `key` to the file indicated by `scope`.
    /// Parses the existing file (if any), updates the key, and writes back.
    pub fn set(&self, key: &str, value: &str, scope: ConfigScope) -> Result<()> {
        let path = match scope {
            ConfigScope::Global => self.global_path.clone(),
            ConfigScope::Project => self
                .project_path
                .clone()
                .ok_or_else(|| HawkError::Config("no project directory set".to_string()))?,
            ConfigScope::Agent(_) => {
                return Err(HawkError::Config(
                    "agent-level config is managed via the agent manifest".to_string(),
                ))
            }
        };

        let mut doc = load_toml_document(&path)?;
        apply_key(&mut doc, key, value)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| HawkError::Config(format!("cannot create config dir: {e}")))?;
        }
        fs::write(&path, doc.to_string())
            .map_err(|e| HawkError::Config(format!("cannot write config: {e}")))?;
        Ok(())
    }

    /// Returns the merged `HawkConfig` (project layer overrides global defaults).
    pub fn merged(&self) -> HawkConfig {
        let base = self.global.clone().unwrap_or_default();
        if let Some(proj) = &self.project {
            // Merge: project values win where they differ from defaults.
            let mut merged = base;
            if !proj.llm.providers.is_empty() {
                merged.llm.providers = proj.llm.providers.clone();
            }
            merged
        } else {
            base
        }
    }

    /// Validates all loaded layers against the schema.
    /// Returns a list of error messages (empty = valid).
    pub fn validate(&self) -> Result<Vec<String>> {
        let mut errors = Vec::new();
        for (label, cfg) in [("global", &self.global), ("project", &self.project)] {
            if let Some(c) = cfg {
                let toml_str = config::to_toml(c)
                    .map_err(|e| HawkError::Config(format!("serialization error: {e}")))?;
                if let Err(e) = config::parse(&toml_str) {
                    errors.push(format!("[{label}] {e}"));
                }
            }
        }
        Ok(errors)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn global_config_path() -> Result<PathBuf> {
    let home = dirs_next::home_dir()
        .ok_or_else(|| HawkError::Config("cannot determine home directory".to_string()))?;
    Ok(home.join(".hawk").join("config.toml"))
}

fn load_optional(path: &Path) -> Result<Option<HawkConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .map_err(|e| HawkError::Config(format!("cannot read {}: {e}", path.display())))?;
    let cfg = config::parse(&text)?;
    Ok(Some(cfg))
}

/// Extract a dot-notation key from a `HawkConfig` as a string.
fn extract(cfg: &HawkConfig, key: &str) -> Option<String> {
    match key {
        "core.log_level" => Some(cfg.core.log_level.clone()),
        "core.session_retention_days" => Some(cfg.core.session_retention_days.to_string()),
        "core.pattern_retention_days" => Some(cfg.core.pattern_retention_days.to_string()),
        "privacy.mode" => Some(cfg.privacy.mode.clone()),
        "llm.providers" => {
            let s = serde_json::to_string(&cfg.llm.providers).ok()?;
            Some(s)
        }
        "llm.pricing.openai_gpt4_prompt" => Some(cfg.llm.pricing.openai_gpt4_prompt.to_string()),
        "llm.pricing.openai_gpt4_completion" => {
            Some(cfg.llm.pricing.openai_gpt4_completion.to_string())
        }
        "savepoint.auto_snapshot" => Some(cfg.savepoint.auto_snapshot.to_string()),
        "savepoint.max_snapshots_per_agent" => {
            Some(cfg.savepoint.max_snapshots_per_agent.to_string())
        }
        "bus.message_retention_seconds" => Some(cfg.bus.message_retention_seconds.to_string()),
        "bus.max_queue_size" => Some(cfg.bus.max_queue_size.to_string()),
        "sync.enabled" => Some(cfg.sync.enabled.to_string()),
        "sync.conflict_strategy" => Some(cfg.sync.conflict_strategy.clone()),
        "compress.token_threshold" => Some(cfg.compress.token_threshold.to_string()),
        "compress.cache_max_entries" => Some(cfg.compress.cache_max_entries.to_string()),
        "healing.max_retries" => Some(cfg.healing.max_retries.to_string()),
        "healing.enabled" => Some(cfg.healing.enabled.to_string()),
        _ => None,
    }
}

/// Load the file as a raw `toml_edit::DocumentMut` (preserves formatting).
/// Returns an empty document if the file does not exist.
fn load_toml_document(path: &Path) -> Result<toml_edit::DocumentMut> {
    if !path.exists() {
        return Ok(toml_edit::DocumentMut::new());
    }
    let text = fs::read_to_string(path)
        .map_err(|e| HawkError::Config(format!("cannot read {}: {e}", path.display())))?;
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|e| HawkError::Config(format!("TOML parse error in {}: {e}", path.display())))
}

/// Write a scalar string value at a dot-notation path into a `toml_edit::DocumentMut`.
fn apply_key(doc: &mut toml_edit::DocumentMut, key: &str, value: &str) -> Result<()> {
    let parts: Vec<&str> = key.splitn(3, '.').collect();
    match parts.as_slice() {
        [section, field] => {
            let table = doc[section].or_insert(toml_edit::table());
            table[field] = toml_edit::value(value);
        }
        [section, subsection, field] => {
            let outer = doc[section].or_insert(toml_edit::table());
            let inner = outer[subsection].or_insert(toml_edit::table());
            inner[field] = toml_edit::value(value);
        }
        [field] => {
            doc[field] = toml_edit::value(value);
        }
        _ => {
            return Err(HawkError::Config(format!(
                "key \"{key}\" has too many segments (max 3)"
            )))
        }
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_toml(dir: &Path, name: &str, content: &str) {
        fs::write(dir.join(name), content).unwrap();
    }

    const PROJECT_TOML: &str = r#"
[core]
log_level = "debug"
session_retention_days = 7
pattern_retention_days = 14

[privacy]
mode = "local-only"

[healing]
max_retries = 5
enabled = true
"#;

    const GLOBAL_TOML: &str = r#"
[core]
log_level = "warn"
session_retention_days = 30
pattern_retention_days = 90

[privacy]
mode = "standard"

[healing]
max_retries = 3
enabled = true
"#;

    fn make_layered(tmp: &TempDir, global: &str, project: &str) -> LayeredConfig {
        let global_dir = tmp.path().join("global");
        fs::create_dir_all(&global_dir).unwrap();
        fs::write(global_dir.join("config.toml"), global).unwrap();

        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        write_toml(&project_dir, "hawk.toml", project);

        // Manually construct so we don't depend on $HOME
        let global_cfg = config::parse(global).unwrap();
        let project_cfg = config::parse(project).unwrap();
        LayeredConfig {
            global: Some(global_cfg),
            project: Some(project_cfg),
            global_path: global_dir.join("config.toml"),
            project_path: Some(project_dir.join("hawk.toml")),
        }
    }

    #[test]
    fn project_overrides_global() {
        let tmp = TempDir::new().unwrap();
        let lc = make_layered(&tmp, GLOBAL_TOML, PROJECT_TOML);

        let v = lc.get_effective("core.log_level").unwrap();
        assert_eq!(v.value, "debug");
        assert!(matches!(v.source, ConfigScope::Project));
    }

    #[test]
    fn global_used_when_no_project_layer() {
        // Only global layer present — all keys should come from global
        let global_cfg = config::parse(GLOBAL_TOML).unwrap();
        let lc = LayeredConfig {
            global: Some(global_cfg),
            project: None,
            global_path: PathBuf::from("/tmp/g.toml"),
            project_path: None,
        };
        let v = lc.get_effective("core.log_level").unwrap();
        assert_eq!(v.value, "warn");
        assert!(matches!(v.source, ConfigScope::Global));
    }

    #[test]
    fn unknown_key_returns_none() {
        let tmp = TempDir::new().unwrap();
        let lc = make_layered(&tmp, GLOBAL_TOML, PROJECT_TOML);
        assert!(lc.get_effective("nonexistent.key").is_none());
    }

    #[test]
    fn set_project_scope_writes_file() {
        let tmp = TempDir::new().unwrap();
        let project_dir = tmp.path().join("proj");
        fs::create_dir_all(&project_dir).unwrap();
        write_toml(&project_dir, "hawk.toml", PROJECT_TOML);

        let lc = LayeredConfig {
            global: None,
            project: config::parse(PROJECT_TOML).ok(),
            global_path: tmp.path().join("g.toml"),
            project_path: Some(project_dir.join("hawk.toml")),
        };

        lc.set("core.log_level", "trace", ConfigScope::Project)
            .unwrap();

        let written = fs::read_to_string(project_dir.join("hawk.toml")).unwrap();
        assert!(written.contains("trace"));
    }

    #[test]
    fn set_global_scope_writes_file() {
        let tmp = TempDir::new().unwrap();
        let global_path = tmp.path().join("config.toml");
        fs::write(&global_path, GLOBAL_TOML).unwrap();

        let lc = LayeredConfig {
            global: config::parse(GLOBAL_TOML).ok(),
            project: None,
            global_path: global_path.clone(),
            project_path: None,
        };

        lc.set("privacy.mode", "air-gapped", ConfigScope::Global)
            .unwrap();

        let written = fs::read_to_string(&global_path).unwrap();
        assert!(written.contains("air-gapped"));
    }

    #[test]
    fn set_agent_scope_returns_error() {
        let tmp = TempDir::new().unwrap();
        let lc = LayeredConfig {
            global: None,
            project: None,
            global_path: tmp.path().join("g.toml"),
            project_path: None,
        };
        let err = lc
            .set(
                "core.log_level",
                "info",
                ConfigScope::Agent("my-agent".to_string()),
            )
            .unwrap_err();
        assert!(err.to_string().contains("manifest"));
    }

    #[test]
    fn validate_returns_empty_for_valid_configs() {
        let tmp = TempDir::new().unwrap();
        let lc = make_layered(&tmp, GLOBAL_TOML, PROJECT_TOML);
        let errors = lc.validate().unwrap();
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn validate_reports_invalid_layer() {
        let bad = "[core]\nlog_level = \"verbose\"\nsession_retention_days = 30\npattern_retention_days = 90\n";
        let tmp = TempDir::new().unwrap();
        let _lc = LayeredConfig {
            global: config::parse(bad).ok(), // parse succeeds (validation is separate)
            project: None,
            global_path: tmp.path().join("g.toml"),
            project_path: None,
        };
        // parse() calls validate() internally, so global will be None if invalid.
        // Test that validate() on a manually-constructed bad config surfaces errors.
        let bad_cfg = toml::from_str::<HawkConfig>(bad).unwrap();
        let lc2 = LayeredConfig {
            global: Some(bad_cfg),
            project: None,
            global_path: tmp.path().join("g.toml"),
            project_path: None,
        };
        let errors = lc2.validate().unwrap();
        assert!(
            !errors.is_empty(),
            "expected validation errors for bad config"
        );
    }

    #[test]
    fn no_project_dir_loads_only_global() {
        let global_cfg = config::parse(GLOBAL_TOML).unwrap();
        let lc = LayeredConfig {
            global: Some(global_cfg),
            project: None,
            global_path: PathBuf::from("/tmp/g.toml"),
            project_path: None,
        };
        let v = lc.get_effective("core.log_level").unwrap();
        assert_eq!(v.value, "warn");
        assert!(matches!(v.source, ConfigScope::Global));
    }

    #[test]
    fn set_creates_file_if_not_exists() {
        let tmp = TempDir::new().unwrap();
        let global_path = tmp.path().join("new_config.toml");
        assert!(!global_path.exists());

        let lc = LayeredConfig {
            global: None,
            project: None,
            global_path: global_path.clone(),
            project_path: None,
        };

        lc.set("core.log_level", "error", ConfigScope::Global)
            .unwrap();
        assert!(global_path.exists());
        let content = fs::read_to_string(&global_path).unwrap();
        assert!(content.contains("error"));
    }
}
