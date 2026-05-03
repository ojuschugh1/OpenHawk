// Talon plugin system — interface, registry, signature verification, isolation

use std::collections::HashMap;
use std::panic;
use std::sync::{Arc, Mutex};

use crate::error::HawkError;

pub type Result<T> = std::result::Result<T, TalonError>;

#[derive(Debug, thiserror::Error)]
pub enum TalonError {
    #[error("signature verification failed for '{0}'")]
    InvalidSignature(String),
    #[error("talon not found: '{0}'")]
    NotFound(String),
    #[error("talon already installed: '{0}'")]
    AlreadyInstalled(String),
    #[error("talon lifecycle error: {0}")]
    Lifecycle(String),
    #[error("talon panicked: {0}")]
    Panicked(String),
}

impl From<TalonError> for HawkError {
    fn from(e: TalonError) -> Self {
        HawkError::Config(e.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Capability {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Default)]
pub struct TalonConfig {
    pub settings: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TalonStatus {
    Loaded,
    Unloaded,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct TalonRecord {
    pub name: String,
    pub version: String,
    pub status: TalonStatus,
    pub capabilities: Vec<Capability>,
    pub signature: String,
}

pub trait Talon: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn load(&mut self) -> Result<()>;
    fn unload(&mut self) -> Result<()>;
    fn configure(&mut self, config: TalonConfig) -> Result<()>;
    fn capabilities(&self) -> Vec<Capability>;
}

// ── Signature verification ────────────────────────────────────────────────────
//
// Production would use ed25519; here we use SHA-256 of "{name}:{version}" as a
// hex string so tests can generate valid signatures without a key pair.

fn expected_signature(name: &str, version: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(format!("{name}:{version}").as_bytes());
    hex::encode(h.finalize())
}

fn verify_signature(name: &str, version: &str, signature: &str) -> bool {
    expected_signature(name, version) == signature
}

pub fn make_signature(name: &str, version: &str) -> String {
    expected_signature(name, version)
}

// ── Registry ──────────────────────────────────────────────────────────────────

pub struct TalonRegistry {
    records: Arc<Mutex<HashMap<String, TalonRecord>>>,
    instances: Arc<Mutex<HashMap<String, Box<dyn Talon>>>>,
}

impl TalonRegistry {
    pub fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(HashMap::new())),
            instances: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn install(
        &self,
        name: &str,
        version: &str,
        signature: &str,
        capabilities: Vec<Capability>,
    ) -> Result<()> {
        if !verify_signature(name, version, signature) {
            eprintln!("SECURITY WARNING: signature verification failed for talon '{name}'. Installation rejected.");
            return Err(TalonError::InvalidSignature(name.to_string()));
        }
        let mut records = self.records.lock().unwrap();
        if records.contains_key(name) {
            return Err(TalonError::AlreadyInstalled(name.to_string()));
        }
        records.insert(
            name.to_string(),
            TalonRecord {
                name: name.to_string(),
                version: version.to_string(),
                status: TalonStatus::Unloaded,
                capabilities,
                signature: signature.to_string(),
            },
        );
        Ok(())
    }

    pub fn load(&self, name: &str) -> Result<()> {
        let mut instances = self.instances.lock().unwrap();
        let mut records = self.records.lock().unwrap();

        let record = records
            .get_mut(name)
            .ok_or_else(|| TalonError::NotFound(name.to_string()))?;

        if let Some(t) = instances.get_mut(name) {
            let t_ptr = t.as_mut() as *mut dyn Talon;
            let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
                // SAFETY: we hold the mutex for the duration of this call
                unsafe { &mut *t_ptr }.load()
            }));
            match result {
                Ok(Ok(())) => {
                    record.status = TalonStatus::Loaded;
                    Ok(())
                }
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    record.status = TalonStatus::Failed(msg.clone());
                    Err(TalonError::Lifecycle(msg))
                }
                Err(_) => {
                    let msg = "talon panicked during load".to_string();
                    record.status = TalonStatus::Failed(msg.clone());
                    Err(TalonError::Panicked(msg))
                }
            }
        } else {
            // no concrete instance — mark as loaded (CLI path)
            record.status = TalonStatus::Loaded;
            Ok(())
        }
    }

    pub fn unload(&self, name: &str) -> Result<()> {
        let mut instances = self.instances.lock().unwrap();
        let mut records = self.records.lock().unwrap();

        let record = records
            .get_mut(name)
            .ok_or_else(|| TalonError::NotFound(name.to_string()))?;

        if let Some(t) = instances.get_mut(name) {
            let t_ptr = t.as_mut() as *mut dyn Talon;
            let result =
                panic::catch_unwind(panic::AssertUnwindSafe(|| unsafe { &mut *t_ptr }.unload()));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    let msg = e.to_string();
                    record.status = TalonStatus::Failed(msg.clone());
                    return Err(TalonError::Lifecycle(msg));
                }
                Err(_) => {
                    let msg = "talon panicked during unload".to_string();
                    record.status = TalonStatus::Failed(msg.clone());
                    return Err(TalonError::Panicked(msg));
                }
            }
        }

        record.status = TalonStatus::Unloaded;
        Ok(())
    }

    pub fn list(&self) -> Vec<TalonRecord> {
        self.records.lock().unwrap().values().cloned().collect()
    }

    pub fn get_capabilities(&self, name: &str) -> Option<Vec<Capability>> {
        self.records
            .lock()
            .unwrap()
            .get(name)
            .map(|r| r.capabilities.clone())
    }

    pub fn is_authorized(agent_manifest_talons: &[String], talon_name: &str) -> bool {
        agent_manifest_talons.iter().any(|t| t == talon_name)
    }

    pub fn register_instance(&self, talon: Box<dyn Talon>) -> Result<()> {
        let name = talon.name().to_string();
        self.instances.lock().unwrap().insert(name, talon);
        Ok(())
    }
}

impl Default for TalonRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct GoodTalon {
        loaded: bool,
    }
    impl GoodTalon {
        fn new() -> Self {
            Self { loaded: false }
        }
    }
    impl Talon for GoodTalon {
        fn name(&self) -> &str {
            "good-talon"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn load(&mut self) -> Result<()> {
            self.loaded = true;
            Ok(())
        }
        fn unload(&mut self) -> Result<()> {
            self.loaded = false;
            Ok(())
        }
        fn configure(&mut self, _: TalonConfig) -> Result<()> {
            Ok(())
        }
        fn capabilities(&self) -> Vec<Capability> {
            vec![Capability {
                name: "browse".to_string(),
                description: "web browsing".to_string(),
            }]
        }
    }

    struct PanickingTalon;
    impl Talon for PanickingTalon {
        fn name(&self) -> &str {
            "panic-talon"
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        fn load(&mut self) -> Result<()> {
            panic!("intentional panic for isolation test");
        }
        fn unload(&mut self) -> Result<()> {
            Ok(())
        }
        fn configure(&mut self, _: TalonConfig) -> Result<()> {
            Ok(())
        }
        fn capabilities(&self) -> Vec<Capability> {
            vec![]
        }
    }

    struct FailingTalon;
    impl Talon for FailingTalon {
        fn name(&self) -> &str {
            "fail-talon"
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        fn load(&mut self) -> Result<()> {
            Err(TalonError::Lifecycle("load failed".to_string()))
        }
        fn unload(&mut self) -> Result<()> {
            Ok(())
        }
        fn configure(&mut self, _: TalonConfig) -> Result<()> {
            Ok(())
        }
        fn capabilities(&self) -> Vec<Capability> {
            vec![]
        }
    }

    fn install_good(registry: &TalonRegistry) {
        let sig = make_signature("good-talon", "1.0.0");
        registry
            .install(
                "good-talon",
                "1.0.0",
                &sig,
                vec![Capability {
                    name: "browse".to_string(),
                    description: "web browsing".to_string(),
                }],
            )
            .unwrap();
    }

    #[test]
    fn load_and_unload_lifecycle() {
        let registry = TalonRegistry::new();
        install_good(&registry);
        registry
            .register_instance(Box::new(GoodTalon::new()))
            .unwrap();
        registry.load("good-talon").unwrap();
        assert_eq!(
            registry.records.lock().unwrap()["good-talon"].status,
            TalonStatus::Loaded
        );
        registry.unload("good-talon").unwrap();
        assert_eq!(
            registry.records.lock().unwrap()["good-talon"].status,
            TalonStatus::Unloaded
        );
    }

    #[test]
    fn valid_signature_accepted() {
        let registry = TalonRegistry::new();
        let sig = make_signature("my-talon", "2.0.0");
        assert!(registry.install("my-talon", "2.0.0", &sig, vec![]).is_ok());
    }

    #[test]
    fn invalid_signature_rejected() {
        let registry = TalonRegistry::new();
        assert!(matches!(
            registry.install("my-talon", "2.0.0", "bad-sig", vec![]),
            Err(TalonError::InvalidSignature(_))
        ));
    }

    #[test]
    fn wrong_version_signature_rejected() {
        let registry = TalonRegistry::new();
        let sig = make_signature("my-talon", "1.0.0");
        assert!(matches!(
            registry.install("my-talon", "2.0.0", &sig, vec![]),
            Err(TalonError::InvalidSignature(_))
        ));
    }

    #[test]
    fn panicking_talon_does_not_crash_process() {
        let registry = TalonRegistry::new();
        let sig = make_signature("panic-talon", "0.1.0");
        registry
            .install("panic-talon", "0.1.0", &sig, vec![])
            .unwrap();
        registry
            .register_instance(Box::new(PanickingTalon))
            .unwrap();
        let result = registry.load("panic-talon");
        assert!(matches!(result, Err(TalonError::Panicked(_))));
        assert!(matches!(
            registry.records.lock().unwrap()["panic-talon"].status,
            TalonStatus::Failed(_)
        ));
    }

    #[test]
    fn failing_talon_is_marked_failed() {
        let registry = TalonRegistry::new();
        let sig = make_signature("fail-talon", "0.1.0");
        registry
            .install("fail-talon", "0.1.0", &sig, vec![])
            .unwrap();
        registry.register_instance(Box::new(FailingTalon)).unwrap();
        let result = registry.load("fail-talon");
        assert!(matches!(result, Err(TalonError::Lifecycle(_))));
        assert!(matches!(
            registry.records.lock().unwrap()["fail-talon"].status,
            TalonStatus::Failed(_)
        ));
    }

    #[test]
    fn authorized_agent_can_use_talon() {
        let declared = vec!["browser-talon".to_string(), "github-talon".to_string()];
        assert!(TalonRegistry::is_authorized(&declared, "browser-talon"));
    }

    #[test]
    fn unauthorized_agent_cannot_use_talon() {
        let declared = vec!["github-talon".to_string()];
        assert!(!TalonRegistry::is_authorized(&declared, "browser-talon"));
    }

    #[test]
    fn empty_manifest_talons_denies_all() {
        assert!(!TalonRegistry::is_authorized(&[], "any-talon"));
    }

    #[test]
    fn get_capabilities_returns_installed_caps() {
        let registry = TalonRegistry::new();
        install_good(&registry);
        let caps = registry.get_capabilities("good-talon").unwrap();
        assert_eq!(caps.len(), 1);
        assert_eq!(caps[0].name, "browse");
    }

    #[test]
    fn get_capabilities_returns_none_for_unknown() {
        let registry = TalonRegistry::new();
        assert!(registry.get_capabilities("unknown").is_none());
    }

    #[test]
    fn list_returns_all_installed() {
        let registry = TalonRegistry::new();
        install_good(&registry);
        let sig2 = make_signature("other-talon", "0.5.0");
        registry
            .install("other-talon", "0.5.0", &sig2, vec![])
            .unwrap();
        assert_eq!(registry.list().len(), 2);
    }

    #[test]
    fn duplicate_install_rejected() {
        let registry = TalonRegistry::new();
        install_good(&registry);
        let sig = make_signature("good-talon", "1.0.0");
        assert!(matches!(
            registry.install("good-talon", "1.0.0", &sig, vec![]),
            Err(TalonError::AlreadyInstalled(_))
        ));
    }

    #[test]
    fn load_unknown_talon_returns_not_found() {
        let registry = TalonRegistry::new();
        assert!(matches!(
            registry.load("ghost"),
            Err(TalonError::NotFound(_))
        ));
    }
}
