use serde::{Deserialize, Serialize};
use crate::error::HawkError;

pub type Result<T> = std::result::Result<T, HawkError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    #[serde(rename = "agent")]
    pub info: AgentInfo,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub resources: Resources,
    #[serde(rename = "llm", default)]
    pub llm: LlmConfig,
    #[serde(rename = "talons", default)]
    pub talon_requirements: TalonRequirements,
    #[serde(default)]
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub framework: String,
    #[serde(default)]
    pub entry_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub filesystem: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    pub cpu_percent: u8,
    pub memory_mb: u64,
    pub max_open_fds: u64,
}

impl Default for Resources {
    fn default() -> Self {
        Self { cpu_percent: 25, memory_mb: 512, max_open_fds: 64 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub privacy: String,
    #[serde(default)]
    pub budget_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TalonRequirements {
    #[serde(default)]
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Capabilities {
    #[serde(default)]
    pub tags: Vec<String>,
}

pub fn parse(toml_str: &str) -> Result<AgentManifest> {
    let manifest: AgentManifest = toml::from_str(toml_str)
        .map_err(|e| HawkError::InvalidManifest(format!("parse error: {e}")))?;
    validate(&manifest)?;
    Ok(manifest)
}

pub fn to_toml(manifest: &AgentManifest) -> Result<String> {
    toml::to_string_pretty(manifest)
        .map_err(|e| HawkError::InvalidManifest(format!("serialization error: {e}")))
}

const VALID_PRIVACY: &[&str] = &["cloud", "local-only"];

fn validate(m: &AgentManifest) -> Result<()> {
    if m.info.name.trim().is_empty() {
        return Err(HawkError::InvalidManifest("[agent] name must not be empty".to_string()));
    }
    if m.info.version.trim().is_empty() {
        return Err(HawkError::InvalidManifest("[agent] version must not be empty".to_string()));
    }
    let cpu = m.resources.cpu_percent;
    if !(1..=100).contains(&cpu) {
        return Err(HawkError::InvalidManifest(format!(
            "[resources] cpu_percent {cpu} is out of range; must be 1..=100"
        )));
    }
    if m.resources.memory_mb < 1 {
        return Err(HawkError::InvalidManifest(
            "[resources] memory_mb must be >= 1".to_string(),
        ));
    }
    if m.resources.max_open_fds < 1 {
        return Err(HawkError::InvalidManifest(
            "[resources] max_open_fds must be >= 1".to_string(),
        ));
    }
    for endpoint in &m.permissions.network {
        validate_network_endpoint(endpoint)?;
    }
    if !m.llm.privacy.is_empty() && !VALID_PRIVACY.contains(&m.llm.privacy.as_str()) {
        return Err(HawkError::InvalidManifest(format!(
            "[llm] privacy \"{}\" is invalid; expected one of: {}",
            m.llm.privacy,
            VALID_PRIVACY.join(", ")
        )));
    }
    Ok(())
}

fn validate_network_endpoint(endpoint: &str) -> Result<()> {
    // strip trailing glob wildcards to get the base URL prefix
    let base = endpoint.trim_end_matches('*').trim_end_matches('/');
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(HawkError::InvalidManifest(format!(
            "[permissions.network] endpoint \"{endpoint}\" must start with http:// or https://"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_MANIFEST: &str = r#"
[agent]
name = "research-agent"
version = "1.0.0"
description = "Researches topics and produces summaries"
framework = "langraph"
entry_command = "python research_agent.py"

[permissions]
filesystem = ["~/projects/research/**", "/tmp/hawk-scratch/**"]
network = ["https://api.openai.com/*", "https://scholar.google.com/*"]
commands = ["curl", "python3"]
secrets = ["OPENAI_API_KEY"]

[resources]
cpu_percent = 25
memory_mb = 512
max_open_fds = 64

[llm]
provider = "openai"
privacy = "cloud"
budget_tokens = 1000000

[talons]
required = ["browser-talon", "github-talon"]

[capabilities]
tags = ["research", "summarization", "web-search"]
"#;

    #[test]
    fn test_parse_full_manifest() {
        let m = parse(FULL_MANIFEST).expect("should parse valid manifest");
        assert_eq!(m.info.name, "research-agent");
        assert_eq!(m.info.version, "1.0.0");
        assert_eq!(m.permissions.filesystem.len(), 2);
        assert_eq!(m.permissions.network.len(), 2);
        assert_eq!(m.permissions.commands, vec!["curl", "python3"]);
        assert_eq!(m.permissions.secrets, vec!["OPENAI_API_KEY"]);
        assert_eq!(m.resources.cpu_percent, 25);
        assert_eq!(m.resources.memory_mb, 512);
        assert_eq!(m.resources.max_open_fds, 64);
        assert_eq!(m.llm.provider, "openai");
        assert_eq!(m.llm.privacy, "cloud");
        assert_eq!(m.llm.budget_tokens, 1_000_000);
        assert_eq!(m.talon_requirements.required, vec!["browser-talon", "github-talon"]);
        assert_eq!(m.capabilities.tags, vec!["research", "summarization", "web-search"]);
    }

    #[test]
    fn test_round_trip() {
        let m = parse(FULL_MANIFEST).unwrap();
        let serialized = to_toml(&m).unwrap();
        let reparsed = parse(&serialized).unwrap();
        assert_eq!(m.info.name, reparsed.info.name);
        assert_eq!(m.info.version, reparsed.info.version);
        assert_eq!(m.permissions.filesystem, reparsed.permissions.filesystem);
        assert_eq!(m.resources.cpu_percent, reparsed.resources.cpu_percent);
        assert_eq!(m.llm.privacy, reparsed.llm.privacy);
        assert_eq!(m.capabilities.tags, reparsed.capabilities.tags);
    }

    #[test]
    fn test_minimal_manifest() {
        let toml = "[agent]\nname = \"minimal\"\nversion = \"0.1.0\"\n";
        let m = parse(toml).unwrap();
        assert_eq!(m.info.name, "minimal");
        assert!(m.permissions.filesystem.is_empty());
        assert_eq!(m.resources.cpu_percent, 25);
    }

    #[test]
    fn test_empty_name_rejected() {
        let toml = "[agent]\nname = \"\"\nversion = \"1.0.0\"\n";
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn test_empty_version_rejected() {
        let toml = "[agent]\nname = \"a\"\nversion = \"\"\n";
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn test_cpu_percent_zero_rejected() {
        let toml = "[agent]\nname = \"a\"\nversion = \"1.0.0\"\n[resources]\ncpu_percent = 0\nmemory_mb = 512\nmax_open_fds = 64\n";
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("cpu_percent"));
    }

    #[test]
    fn test_invalid_network_endpoint() {
        let toml = "[agent]\nname = \"a\"\nversion = \"1.0.0\"\n[permissions]\nnetwork = [\"ftp://example.com/*\"]\n";
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("http://") || err.to_string().contains("https://"));
    }

    #[test]
    fn test_invalid_llm_privacy() {
        let toml = "[agent]\nname = \"a\"\nversion = \"1.0.0\"\n[llm]\nprivacy = \"hybrid\"\n";
        let err = parse(toml).unwrap_err();
        assert!(err.to_string().contains("privacy"));
    }

    #[test]
    fn test_invalid_toml_syntax() {
        let err = parse("[agent\nname = \"a\"").unwrap_err();
        assert!(err.to_string().contains("parse error"));
    }
}
