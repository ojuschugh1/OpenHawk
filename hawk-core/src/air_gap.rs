use crate::llm_router::LlmProvider;

#[derive(Debug, Clone, PartialEq)]
pub enum AirGapError {
    Blocked { agent_pid: u32, endpoint: String },
    CloudProviderBlocked { provider_name: String },
    NetworkOperationBlocked { operation: String },
}

impl std::fmt::Display for AirGapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AirGapError::Blocked {
                agent_pid,
                endpoint,
            } => {
                write!(f, "air-gap: agent {agent_pid} blocked from {endpoint}")
            }
            AirGapError::CloudProviderBlocked { provider_name } => {
                write!(f, "air-gap: cloud provider '{provider_name}' blocked")
            }
            AirGapError::NetworkOperationBlocked { operation } => {
                write!(f, "air-gap: network operation '{operation}' blocked")
            }
        }
    }
}

impl std::error::Error for AirGapError {}

pub struct AirGapEnforcer {
    enabled: bool,
}

impl AirGapEnforcer {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn check_network_request(&self, agent_pid: u32, endpoint: &str) -> Result<(), AirGapError> {
        if !self.enabled {
            return Ok(());
        }
        if is_local_endpoint(endpoint) {
            return Ok(());
        }
        eprintln!("air-gap: denied agent {agent_pid} → {endpoint}");
        Err(AirGapError::Blocked {
            agent_pid,
            endpoint: endpoint.to_string(),
        })
    }

    pub fn check_llm_provider(&self, provider: &LlmProvider) -> Result<(), AirGapError> {
        if !self.enabled {
            return Ok(());
        }
        if provider.is_local {
            return Ok(());
        }
        Err(AirGapError::CloudProviderBlocked {
            provider_name: provider.name.clone(),
        })
    }

    pub fn filter_llm_providers<'a>(&self, providers: &'a [LlmProvider]) -> Vec<&'a LlmProvider> {
        if !self.enabled {
            return providers.iter().collect();
        }
        providers.iter().filter(|p| p.is_local).collect()
    }

    pub fn check_nest_operation(&self, operation: &str) -> Result<(), AirGapError> {
        if !self.enabled {
            return Ok(());
        }
        match operation {
            "publish" | "search_remote" => Err(AirGapError::NetworkOperationBlocked {
                operation: operation.to_string(),
            }),
            _ => Ok(()),
        }
    }
}

fn is_local_endpoint(endpoint: &str) -> bool {
    endpoint.contains("localhost") || endpoint.contains("127.0.0.1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_provider(name: &str) -> LlmProvider {
        LlmProvider {
            name: name.to_string(),
            endpoint: "http://localhost:11434".to_string(),
            priority: 1,
            is_local: true,
        }
    }

    fn cloud_provider(name: &str) -> LlmProvider {
        LlmProvider {
            name: name.to_string(),
            endpoint: "https://api.openai.com/v1".to_string(),
            priority: 1,
            is_local: false,
        }
    }

    #[test]
    fn network_request_blocked_when_enabled_and_remote() {
        let enforcer = AirGapEnforcer::new(true);
        let err = enforcer
            .check_network_request(42, "https://api.example.com")
            .unwrap_err();
        assert_eq!(
            err,
            AirGapError::Blocked {
                agent_pid: 42,
                endpoint: "https://api.example.com".to_string()
            }
        );
    }

    #[test]
    fn network_request_allowed_when_disabled() {
        let enforcer = AirGapEnforcer::new(false);
        assert!(enforcer
            .check_network_request(1, "https://api.example.com")
            .is_ok());
    }

    #[test]
    fn network_request_allowed_for_localhost() {
        let enforcer = AirGapEnforcer::new(true);
        assert!(enforcer
            .check_network_request(1, "http://localhost:11434")
            .is_ok());
    }

    #[test]
    fn network_request_allowed_for_127_0_0_1() {
        let enforcer = AirGapEnforcer::new(true);
        assert!(enforcer
            .check_network_request(1, "http://127.0.0.1:8080")
            .is_ok());
    }

    #[test]
    fn denied_request_includes_agent_pid_and_endpoint() {
        let enforcer = AirGapEnforcer::new(true);
        let err = enforcer
            .check_network_request(99, "https://remote.host/api")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("99"));
        assert!(msg.contains("https://remote.host/api"));
    }

    #[test]
    fn llm_cloud_provider_blocked_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        let provider = cloud_provider("openai");
        let err = enforcer.check_llm_provider(&provider).unwrap_err();
        assert_eq!(
            err,
            AirGapError::CloudProviderBlocked {
                provider_name: "openai".to_string()
            }
        );
    }

    #[test]
    fn llm_local_provider_allowed_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        assert!(enforcer
            .check_llm_provider(&local_provider("ollama"))
            .is_ok());
    }

    #[test]
    fn llm_cloud_provider_allowed_when_disabled() {
        let enforcer = AirGapEnforcer::new(false);
        assert!(enforcer
            .check_llm_provider(&cloud_provider("openai"))
            .is_ok());
    }

    #[test]
    fn filter_returns_only_local_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        let providers = vec![cloud_provider("openai"), local_provider("ollama")];
        let filtered = enforcer.filter_llm_providers(&providers);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "ollama");
    }

    #[test]
    fn filter_returns_all_when_disabled() {
        let enforcer = AirGapEnforcer::new(false);
        let providers = vec![cloud_provider("openai"), local_provider("ollama")];
        assert_eq!(enforcer.filter_llm_providers(&providers).len(), 2);
    }

    #[test]
    fn filter_returns_empty_when_no_local_providers() {
        let enforcer = AirGapEnforcer::new(true);
        let providers = vec![cloud_provider("openai"), cloud_provider("anthropic")];
        assert!(enforcer.filter_llm_providers(&providers).is_empty());
    }

    #[test]
    fn nest_publish_blocked_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        let err = enforcer.check_nest_operation("publish").unwrap_err();
        assert_eq!(
            err,
            AirGapError::NetworkOperationBlocked {
                operation: "publish".to_string()
            }
        );
    }

    #[test]
    fn nest_search_remote_blocked_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        assert!(matches!(
            enforcer.check_nest_operation("search_remote"),
            Err(AirGapError::NetworkOperationBlocked { .. })
        ));
    }

    #[test]
    fn nest_search_local_allowed_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        assert!(enforcer.check_nest_operation("search_local").is_ok());
    }

    #[test]
    fn nest_install_from_cache_allowed_when_enabled() {
        let enforcer = AirGapEnforcer::new(true);
        assert!(enforcer.check_nest_operation("install_from_cache").is_ok());
    }

    #[test]
    fn nest_publish_allowed_when_disabled() {
        let enforcer = AirGapEnforcer::new(false);
        assert!(enforcer.check_nest_operation("publish").is_ok());
    }

    #[test]
    fn is_enabled_reflects_constructor_arg() {
        assert!(AirGapEnforcer::new(true).is_enabled());
        assert!(!AirGapEnforcer::new(false).is_enabled());
    }
}
