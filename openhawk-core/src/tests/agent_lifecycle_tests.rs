// Unit tests for agent lifecycle (6.4)
// Requirements: 1.1, 1.2, 1.3, 1.5, 1.6, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::agent_manager::AgentManager;
use crate::db::SCHEMA;
use crate::manifest::{
    AgentInfo, AgentManifest, Capabilities, LlmConfig, Permissions, Resources, TalonRequirements,
};
use crate::permission_guard::PermissionGuard;
use crate::resource_monitor::{ResourceEvent, ResourceLimits, ResourceMonitor};
use crate::types::{LifecycleState, PermissionResult};

fn mem_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn
}

fn make_manager() -> AgentManager {
    AgentManager::new(mem_conn())
}

fn make_guard() -> PermissionGuard {
    PermissionGuard::new(mem_conn())
}

fn minimal_manifest(entry: &str) -> AgentManifest {
    AgentManifest {
        info: AgentInfo {
            name: "test-agent".to_string(),
            version: "1.0.0".to_string(),
            description: String::new(),
            framework: String::new(),
            entry_command: entry.to_string(),
        },
        permissions: Permissions {
            filesystem: vec!["/tmp/**".to_string()],
            network: vec!["https://api.openai.com/*".to_string()],
            commands: vec!["echo".to_string()],
            secrets: vec!["MY_KEY".to_string()],
        },
        resources: Resources {
            cpu_percent: 25,
            memory_mb: 512,
            max_open_fds: 64,
        },
        llm: LlmConfig::default(),
        talon_requirements: TalonRequirements::default(),
        capabilities: Capabilities::default(),
    }
}

// ── spawn ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_spawn_creates_child_and_records_in_db() {
    let manager = make_manager();
    let pid = manager
        .spawn(minimal_manifest("sleep 60"))
        .await
        .expect("spawn should succeed");
    assert!(pid > 0);
    assert_eq!(manager.get_state(pid), Some(LifecycleState::Running));
    let _ = manager.stop(pid).await;
}

#[tokio::test]
async fn test_spawn_invalid_manifest_returns_error() {
    let manager = make_manager();
    assert!(manager.spawn(minimal_manifest("")).await.is_err());
}

#[tokio::test]
async fn test_spawn_records_agent_in_list() {
    let manager = make_manager();
    let pid = manager
        .spawn(minimal_manifest("sleep 60"))
        .await
        .expect("spawn should succeed");
    assert!(manager.list().iter().any(|a| a.pid == pid));
    let _ = manager.stop(pid).await;
}

// ── stop ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_stop_terminates_process() {
    let manager = make_manager();
    let pid = manager
        .spawn(minimal_manifest("sleep 60"))
        .await
        .expect("spawn should succeed");
    let result = manager.stop(pid).await.expect("stop should succeed");
    assert_eq!(result.pid, pid);
    assert!(!manager.list().iter().any(|a| a.pid == pid));
}

#[tokio::test]
async fn test_stop_unknown_pid_returns_error() {
    let manager = make_manager();
    assert!(manager.stop(99999).await.is_err());
}

#[tokio::test]
async fn test_stop_fast_process_completes() {
    let manager = make_manager();
    let pid = manager
        .spawn(minimal_manifest("echo done"))
        .await
        .expect("spawn should succeed");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let result = manager.stop(pid).await.expect("stop should succeed");
    assert_eq!(result.pid, pid);
}

// ── pause / resume ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_pause_updates_state_to_paused() {
    let manager = make_manager();
    let pid = manager
        .spawn(minimal_manifest("sleep 60"))
        .await
        .expect("spawn should succeed");
    manager.pause(pid).expect("pause should succeed");
    assert_eq!(manager.get_state(pid), Some(LifecycleState::Paused));
    manager.resume(pid).expect("resume should succeed");
    let _ = manager.stop(pid).await;
}

#[tokio::test]
async fn test_resume_updates_state_to_running() {
    let manager = make_manager();
    let pid = manager
        .spawn(minimal_manifest("sleep 60"))
        .await
        .expect("spawn should succeed");
    manager.pause(pid).expect("pause should succeed");
    manager.resume(pid).expect("resume should succeed");
    assert_eq!(manager.get_state(pid), Some(LifecycleState::Running));
    let _ = manager.stop(pid).await;
}

#[tokio::test]
async fn test_pause_unknown_pid_returns_error() {
    let manager = make_manager();
    assert!(manager.pause(99999).is_err());
}

#[tokio::test]
async fn test_resume_unknown_pid_returns_error() {
    let manager = make_manager();
    assert!(manager.resume(99999).is_err());
}

// ── permission checks ─────────────────────────────────────────────────────────

#[test]
fn test_permission_fs_allow() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_fs_access(1, Path::new("/tmp/file.txt")),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_permission_fs_deny() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_fs_access(1, Path::new("/etc/shadow")),
        PermissionResult::Denied { .. }
    ));
}

#[test]
fn test_permission_network_allow() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_network(1, "https://api.openai.com/v1/completions"),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_permission_network_deny() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_network(1, "https://malicious.example.com"),
        PermissionResult::Denied { .. }
    ));
}

#[test]
fn test_permission_command_allow() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_command(1, "echo hello world"),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_permission_command_deny() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_command(1, "rm -rf /"),
        PermissionResult::Denied { .. }
    ));
}

#[test]
fn test_permission_secret_allow() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_secret(1, "MY_KEY"),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_permission_secret_deny() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    assert!(matches!(
        guard.check_secret(1, "OTHER_KEY"),
        PermissionResult::Denied { .. }
    ));
}

// ── trust mode ────────────────────────────────────────────────────────────────

#[test]
fn test_trust_bypasses_fs_check() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    guard.trust("test-agent", "session-1".to_string());
    assert!(guard.is_trusted(1));
    assert!(matches!(
        guard.check_fs_access(1, Path::new("/etc/shadow")),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_trust_bypasses_network_check() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    guard.trust("test-agent", "session-1".to_string());
    assert!(matches!(
        guard.check_network(1, "https://any.endpoint.com"),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_trust_bypasses_command_check() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    guard.trust("test-agent", "session-1".to_string());
    assert!(matches!(
        guard.check_command(1, "rm -rf /"),
        PermissionResult::Allowed
    ));
}

#[test]
fn test_trust_bypasses_secret_check() {
    let mut guard = make_guard();
    guard.register(1, minimal_manifest("echo"));
    guard.trust("test-agent", "session-1".to_string());
    assert!(matches!(
        guard.check_secret(1, "UNKNOWN_SECRET"),
        PermissionResult::Allowed
    ));
}

// ── resource monitor ──────────────────────────────────────────────────────────

#[test]
fn test_resource_monitor_register_deregister() {
    let monitor = ResourceMonitor::new();
    monitor.register(
        1234,
        ResourceLimits {
            cpu_percent: 25,
            memory_mb: 512,
            max_open_fds: 64,
        },
    );
    assert!(monitor.limits.lock().unwrap().contains_key(&1234));
    monitor.deregister(1234);
    assert!(!monitor.limits.lock().unwrap().contains_key(&1234));
}

#[tokio::test]
async fn test_resource_monitor_memory_exceeded_event() {
    let monitor = ResourceMonitor::new();
    monitor.register(
        42,
        ResourceLimits {
            cpu_percent: 10,
            memory_mb: 512,
            max_open_fds: 64,
        },
    );

    monitor
        .event_tx
        .send(ResourceEvent::MemoryExceeded {
            pid: 42,
            memory_bytes: 600 * 1024 * 1024,
            limit_bytes: 512 * 1024 * 1024,
        })
        .unwrap();

    let event = monitor.event_rx.lock().unwrap().try_recv().unwrap();
    match event {
        ResourceEvent::MemoryExceeded {
            pid,
            memory_bytes,
            limit_bytes,
        } => {
            assert_eq!(pid, 42);
            assert!(memory_bytes > limit_bytes);
        }
    }
}
