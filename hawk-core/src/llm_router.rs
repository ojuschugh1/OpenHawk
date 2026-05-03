use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub prompt: String,
    pub max_tokens: Option<u32>,
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub provider: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct LlmProvider {
    pub name: String,
    pub endpoint: String,
    pub priority: u32,
    pub is_local: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProviderStatus {
    Available,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct LlmProviderStatus {
    pub provider: LlmProvider,
    pub status: ProviderStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouterError {
    NoProviderAvailable,
    ProviderFailed { provider: String, reason: String },
}

impl std::fmt::Display for RouterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouterError::NoProviderAvailable => write!(f, "no LLM provider available"),
            RouterError::ProviderFailed { provider, reason } => {
                write!(f, "provider '{provider}' failed: {reason}")
            }
        }
    }
}

impl std::error::Error for RouterError {}

pub struct LlmRouter {
    providers: Vec<LlmProvider>,
    queued: Mutex<VecDeque<(u32, LlmRequest)>>,
}

impl LlmRouter {
    pub fn new(mut providers: Vec<LlmProvider>) -> Self {
        providers.sort_by_key(|p| p.priority);
        Self { providers, queued: Mutex::new(VecDeque::new()) }
    }

    pub fn route_request(&self, agent_pid: u32, request: LlmRequest, local_only: bool) -> Result<LlmResponse, RouterError> {
        let candidates: Vec<&LlmProvider> = self.providers.iter()
            .filter(|p| !local_only || p.is_local)
            .collect();

        for provider in &candidates {
            match self.check_availability(provider) {
                ProviderStatus::Available => {
                    match self.call_provider(provider, &request) {
                        Ok(resp) => return Ok(resp),
                        Err(reason) => {
                            eprintln!("LLM fallback: provider '{}' failed for agent {}: {}; trying next", provider.name, agent_pid, reason);
                        }
                    }
                }
                status => {
                    eprintln!("LLM fallback: provider '{}' status {:?} for agent {}; trying next", provider.name, status, agent_pid);
                }
            }
        }

        eprintln!("No LLM provider available for agent {}; request queued", agent_pid);
        self.queued.lock().unwrap().push_back((agent_pid, request));
        Err(RouterError::NoProviderAvailable)
    }

    pub fn check_availability(&self, _provider: &LlmProvider) -> ProviderStatus {
        // Real impl would HTTP-ping the endpoint; always available in this stub
        ProviderStatus::Available
    }

    pub fn get_providers(&self) -> Vec<LlmProviderStatus> {
        self.providers.iter().map(|p| LlmProviderStatus {
            status: self.check_availability(p),
            provider: p.clone(),
        }).collect()
    }

    pub fn queued_count(&self) -> usize {
        self.queued.lock().unwrap().len()
    }

    pub fn filter_for_air_gap(providers: &[LlmProvider], air_gapped: bool) -> Vec<&LlmProvider> {
        if !air_gapped {
            return providers.iter().collect();
        }
        providers.iter().filter(|p| p.is_local).collect()
    }

    fn call_provider(&self, provider: &LlmProvider, request: &LlmRequest) -> Result<LlmResponse, String> {
        let prompt_tokens = (request.prompt.split_whitespace().count() as u32).max(1);
        let completion_tokens = request.max_tokens.unwrap_or(64);
        Ok(LlmResponse {
            text: format!("[{}] response to: {}", provider.name, request.prompt),
            provider: provider.name.clone(),
            prompt_tokens,
            completion_tokens,
        })
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::collections::HashSet;

    pub struct MockRouter {
        providers: Vec<LlmProvider>,
        unavailable: HashSet<String>,
        queued: Mutex<VecDeque<(u32, LlmRequest)>>,
    }

    impl MockRouter {
        pub fn new(mut providers: Vec<LlmProvider>) -> Self {
            providers.sort_by_key(|p| p.priority);
            Self { providers, unavailable: HashSet::new(), queued: Mutex::new(VecDeque::new()) }
        }

        pub fn mark_unavailable(&mut self, name: &str) {
            self.unavailable.insert(name.to_string());
        }

        pub fn route_request(&self, agent_pid: u32, request: LlmRequest, local_only: bool) -> Result<LlmResponse, RouterError> {
            let candidates: Vec<&LlmProvider> = self.providers.iter()
                .filter(|p| !local_only || p.is_local)
                .collect();

            for provider in &candidates {
                if self.unavailable.contains(&provider.name) {
                    eprintln!("LLM fallback: provider '{}' unavailable for agent {}; trying next", provider.name, agent_pid);
                    continue;
                }
                let prompt_tokens = (request.prompt.split_whitespace().count() as u32).max(1);
                let completion_tokens = request.max_tokens.unwrap_or(64);
                return Ok(LlmResponse {
                    text: format!("[{}] response to: {}", provider.name, request.prompt),
                    provider: provider.name.clone(),
                    prompt_tokens,
                    completion_tokens,
                });
            }

            eprintln!("No LLM provider available for agent {}; request queued", agent_pid);
            self.queued.lock().unwrap().push_back((agent_pid, request));
            Err(RouterError::NoProviderAvailable)
        }

        pub fn queued_count(&self) -> usize {
            self.queued.lock().unwrap().len()
        }

        pub fn get_providers(&self) -> Vec<LlmProviderStatus> {
            self.providers.iter().map(|p| LlmProviderStatus {
                status: if self.unavailable.contains(&p.name) {
                    ProviderStatus::Unavailable
                } else {
                    ProviderStatus::Available
                },
                provider: p.clone(),
            }).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_support::MockRouter;

    fn make_providers() -> Vec<LlmProvider> {
        vec![
            LlmProvider { name: "openai".into(), endpoint: "https://api.openai.com/v1".into(), priority: 1, is_local: false },
            LlmProvider { name: "ollama".into(), endpoint: "http://localhost:11434".into(), priority: 2, is_local: true },
        ]
    }

    fn req(prompt: &str) -> LlmRequest {
        LlmRequest { prompt: prompt.to_string(), max_tokens: None, model: None }
    }

    #[test]
    fn routes_to_highest_priority_provider() {
        let router = MockRouter::new(make_providers());
        let resp = router.route_request(1, req("hello"), false).unwrap();
        assert_eq!(resp.provider, "openai");
    }

    #[test]
    fn provider_list_sorted_by_priority() {
        let providers = vec![
            LlmProvider { name: "ollama".into(), endpoint: "http://localhost:11434".into(), priority: 2, is_local: true },
            LlmProvider { name: "openai".into(), endpoint: "https://api.openai.com/v1".into(), priority: 1, is_local: false },
        ];
        let router = MockRouter::new(providers);
        let resp = router.route_request(1, req("test"), false).unwrap();
        assert_eq!(resp.provider, "openai");
    }

    #[test]
    fn falls_back_to_next_provider_on_failure() {
        let mut router = MockRouter::new(make_providers());
        router.mark_unavailable("openai");
        let resp = router.route_request(1, req("hello"), false).unwrap();
        assert_eq!(resp.provider, "ollama");
    }

    #[test]
    fn response_contains_prompt_text() {
        let router = MockRouter::new(make_providers());
        let resp = router.route_request(42, req("what is rust"), false).unwrap();
        assert!(resp.text.contains("what is rust"));
    }

    #[test]
    fn local_only_skips_cloud_providers() {
        let router = MockRouter::new(make_providers());
        let resp = router.route_request(1, req("private"), true).unwrap();
        assert_eq!(resp.provider, "ollama");
    }

    #[test]
    fn local_only_fails_when_no_local_provider() {
        let providers = vec![
            LlmProvider { name: "openai".into(), endpoint: "https://api.openai.com/v1".into(), priority: 1, is_local: false },
        ];
        let router = MockRouter::new(providers);
        let err = router.route_request(1, req("private"), true).unwrap_err();
        assert_eq!(err, RouterError::NoProviderAvailable);
    }

    #[test]
    fn queues_request_when_no_provider_available() {
        let mut router = MockRouter::new(make_providers());
        router.mark_unavailable("openai");
        router.mark_unavailable("ollama");
        let err = router.route_request(7, req("queue me"), false).unwrap_err();
        assert_eq!(err, RouterError::NoProviderAvailable);
        assert_eq!(router.queued_count(), 1);
    }

    #[test]
    fn multiple_failed_requests_all_queued() {
        let mut router = MockRouter::new(make_providers());
        router.mark_unavailable("openai");
        router.mark_unavailable("ollama");
        for _ in 0..3 {
            let _ = router.route_request(1, req("q"), false);
        }
        assert_eq!(router.queued_count(), 3);
    }

    #[test]
    fn get_providers_reflects_availability() {
        let mut router = MockRouter::new(make_providers());
        router.mark_unavailable("openai");
        let statuses = router.get_providers();
        let openai = statuses.iter().find(|s| s.provider.name == "openai").unwrap();
        let ollama = statuses.iter().find(|s| s.provider.name == "ollama").unwrap();
        assert_eq!(openai.status, ProviderStatus::Unavailable);
        assert_eq!(ollama.status, ProviderStatus::Available);
    }

    #[test]
    fn llm_router_routes_successfully() {
        let router = LlmRouter::new(make_providers());
        let resp = router.route_request(1, req("hello"), false).unwrap();
        assert!(!resp.provider.is_empty());
        assert!(!resp.text.is_empty());
    }

    #[test]
    fn llm_router_local_only_returns_local_provider() {
        let router = LlmRouter::new(make_providers());
        let resp = router.route_request(1, req("private"), true).unwrap();
        assert_eq!(resp.provider, "ollama");
    }

    #[test]
    fn filter_for_air_gap_returns_only_local_when_enabled() {
        let providers = make_providers();
        let filtered = LlmRouter::filter_for_air_gap(&providers, true);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "ollama");
    }

    #[test]
    fn filter_for_air_gap_returns_all_when_disabled() {
        let providers = make_providers();
        let filtered = LlmRouter::filter_for_air_gap(&providers, false);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_for_air_gap_empty_when_no_local_providers() {
        let providers = vec![
            LlmProvider { name: "openai".into(), endpoint: "https://api.openai.com/v1".into(), priority: 1, is_local: false },
            LlmProvider { name: "anthropic".into(), endpoint: "https://api.anthropic.com".into(), priority: 2, is_local: false },
        ];
        let filtered = LlmRouter::filter_for_air_gap(&providers, true);
        assert!(filtered.is_empty());
    }
}
