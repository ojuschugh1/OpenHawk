use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};

use crate::error::HawkError;
use crate::manifest::AgentManifest;
use crate::types::{PermissionResult, ProcessId, SessionId};

pub type Result<T> = std::result::Result<T, HawkError>;

pub struct PermissionGuard {
    pub db: Arc<Mutex<Connection>>,
    manifests: HashMap<ProcessId, AgentManifest>,
    trusted: HashSet<ProcessId>,
}

impl PermissionGuard {
    pub fn new(db: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            manifests: HashMap::new(),
            trusted: HashSet::new(),
        }
    }

    pub fn register(&mut self, pid: ProcessId, manifest: AgentManifest) {
        self.manifests.insert(pid, manifest);
    }

    pub fn check_fs_access(&self, pid: ProcessId, path: &Path) -> PermissionResult {
        if self.trusted.contains(&pid) {
            return PermissionResult::Allowed;
        }
        let path_str = path.to_string_lossy();
        if let Some(manifest) = self.manifests.get(&pid) {
            for pattern in &manifest.permissions.filesystem {
                if glob_match(pattern, &path_str) {
                    return PermissionResult::Allowed;
                }
            }
            self.log_denial(pid, "fs_access", &path_str);
            PermissionResult::Denied {
                reason: format!("filesystem path '{}' not in manifest allowlist", path_str),
            }
        } else {
            PermissionResult::Denied {
                reason: format!("no manifest registered for pid {pid}"),
            }
        }
    }

    pub fn check_network(&self, pid: ProcessId, endpoint: &str) -> PermissionResult {
        if self.trusted.contains(&pid) {
            return PermissionResult::Allowed;
        }
        if let Some(manifest) = self.manifests.get(&pid) {
            for pattern in &manifest.permissions.network {
                if glob_match(pattern, endpoint) {
                    return PermissionResult::Allowed;
                }
            }
            self.log_denial(pid, "network", endpoint);
            PermissionResult::Denied {
                reason: format!("network endpoint '{}' not in manifest allowlist", endpoint),
            }
        } else {
            PermissionResult::Denied {
                reason: format!("no manifest registered for pid {pid}"),
            }
        }
    }

    pub fn check_command(&self, pid: ProcessId, command: &str) -> PermissionResult {
        if self.trusted.contains(&pid) {
            return PermissionResult::Allowed;
        }
        if let Some(manifest) = self.manifests.get(&pid) {
            let base = command.split_whitespace().next().unwrap_or(command);
            if manifest.permissions.commands.iter().any(|c| c == base) {
                return PermissionResult::Allowed;
            }
            self.log_denial(pid, "command", command);
            PermissionResult::Denied {
                reason: format!("command '{}' not in manifest allowlist", command),
            }
        } else {
            PermissionResult::Denied {
                reason: format!("no manifest registered for pid {pid}"),
            }
        }
    }

    pub fn check_secret(&self, pid: ProcessId, key: &str) -> PermissionResult {
        if self.trusted.contains(&pid) {
            return PermissionResult::Allowed;
        }
        if let Some(manifest) = self.manifests.get(&pid) {
            if manifest.permissions.secrets.iter().any(|s| s == key) {
                return PermissionResult::Allowed;
            }
            self.log_denial(pid, "secret", key);
            PermissionResult::Denied {
                reason: format!("secret key '{}' not in manifest secrets list", key),
            }
        } else {
            PermissionResult::Denied {
                reason: format!("no manifest registered for pid {pid}"),
            }
        }
    }

    pub fn trust(&mut self, agent_name: &str, _session: SessionId) {
        for (pid, manifest) in &self.manifests {
            if manifest.info.name == agent_name {
                self.trusted.insert(*pid);
            }
        }
    }

    pub fn is_trusted(&self, pid: ProcessId) -> bool {
        self.trusted.contains(&pid)
    }

    fn log_denial(&self, pid: ProcessId, kind: &str, resource: &str) {
        let db = self.db.lock().unwrap();
        let _ = db.execute(
            "INSERT INTO watch_alerts \
             (alert_type, timestamp, agent_name, details_json, acknowledged) \
             VALUES (?1, datetime('now'), ?2, ?3, 0)",
            params![
                "permission_denied",
                pid.to_string(),
                format!(r#"{{"kind":"{kind}","resource":"{resource}","pid":{pid}}}"#),
            ],
        );
    }
}

fn glob_match(pattern: &str, value: &str) -> bool {
    match glob::Pattern::new(pattern) {
        Ok(p) => p.matches(value),
        Err(_) => pattern == value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SCHEMA;
    use crate::manifest::{
        AgentInfo, Capabilities, LlmConfig, Permissions, Resources, TalonRequirements,
    };

    fn make_guard() -> PermissionGuard {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        PermissionGuard::new(conn)
    }

    fn test_manifest() -> AgentManifest {
        AgentManifest {
            info: AgentInfo {
                name: "test-agent".to_string(),
                version: "1.0.0".to_string(),
                description: String::new(),
                framework: String::new(),
                entry_command: "echo hello".to_string(),
            },
            permissions: Permissions {
                filesystem: vec!["~/projects/**".to_string(), "/tmp/**".to_string()],
                network: vec!["https://api.openai.com/*".to_string()],
                commands: vec!["curl".to_string(), "python3".to_string()],
                secrets: vec!["OPENAI_API_KEY".to_string()],
            },
            resources: Resources::default(),
            llm: LlmConfig::default(),
            talon_requirements: TalonRequirements::default(),
            capabilities: Capabilities::default(),
        }
    }

    #[test]
    fn test_fs_access_allowed() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_fs_access(1, Path::new("/tmp/foo.txt")),
            PermissionResult::Allowed
        ));
    }

    #[test]
    fn test_fs_access_denied() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_fs_access(1, Path::new("/etc/passwd")),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_network_allowed() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_network(1, "https://api.openai.com/v1/chat"),
            PermissionResult::Allowed
        ));
    }

    #[test]
    fn test_network_denied() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_network(1, "https://evil.example.com"),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_command_allowed() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_command(1, "curl https://example.com"),
            PermissionResult::Allowed
        ));
    }

    #[test]
    fn test_command_denied() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_command(1, "rm -rf /"),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_secret_allowed() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_secret(1, "OPENAI_API_KEY"),
            PermissionResult::Allowed
        ));
    }

    #[test]
    fn test_secret_denied() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        assert!(matches!(
            guard.check_secret(1, "AWS_SECRET_KEY"),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_trust_bypasses_all_checks() {
        let mut guard = make_guard();
        guard.register(1, test_manifest());
        guard.trust("test-agent", "session-1".to_string());

        assert!(guard.is_trusted(1));
        assert!(matches!(
            guard.check_fs_access(1, Path::new("/etc/passwd")),
            PermissionResult::Allowed
        ));
        assert!(matches!(
            guard.check_network(1, "https://evil.example.com"),
            PermissionResult::Allowed
        ));
        assert!(matches!(
            guard.check_command(1, "rm -rf /"),
            PermissionResult::Allowed
        ));
        assert!(matches!(
            guard.check_secret(1, "UNKNOWN_KEY"),
            PermissionResult::Allowed
        ));
    }

    #[test]
    fn test_no_manifest_returns_denied() {
        let guard = make_guard();
        assert!(matches!(
            guard.check_fs_access(99, Path::new("/tmp/x")),
            PermissionResult::Denied { .. }
        ));
    }

    #[test]
    fn test_denial_logged_to_db() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let mut guard = PermissionGuard::new(conn);
        guard.register(1, test_manifest());
        guard.check_fs_access(1, Path::new("/etc/passwd"));

        let count: i64 = guard
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM watch_alerts WHERE alert_type = 'permission_denied'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
