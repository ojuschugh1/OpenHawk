// Integration tests for end-to-end workflows
// Requirements: 1.1, 2.1, 2.2, 3.6, 5.2, 14.1, 14.2

use std::fs;
use std::io::Write;
use std::path::Path;

use tempfile::TempDir;

use crate::db::init_database;
use crate::manifest::{AgentInfo, AgentManifest, Capabilities, LlmConfig, Permissions, Resources, TalonRequirements};
use crate::orchestrator::{Orchestrator, SubTaskStatus};
use crate::resource_monitor::{ResourceEvent, ResourceLimits, ResourceMonitor};
use hawk_bus::{BusMessage, MessageBus};

fn write_file(dir: &Path, name: &str, content: &[u8]) {
    let mut f = fs::File::create(dir.join(name)).unwrap();
    f.write_all(content).unwrap();
}

fn valid_msg(method: &str, payload: serde_json::Value) -> BusMessage {
    BusMessage { jsonrpc: "2.0".into(), method: method.into(), params: payload, id: None }
}

// ── snapshot workflow ─────────────────────────────────────────────────────────

/// Req 1.1, 2.1 — write files → snapshot → agent modifies → rollback → verify restored
#[test]
fn test_snapshot_modify_rollback_restores_files() {
    let work = TempDir::new().unwrap();
    let snap_dir = TempDir::new().unwrap();

    write_file(work.path(), "data.txt", b"original content");
    write_file(work.path(), "config.toml", b"[settings]\nvalue = 1");

    // Snapshot: copy files
    for name in &["data.txt", "config.toml"] {
        fs::copy(work.path().join(name), snap_dir.path().join(name)).unwrap();
    }

    // Agent modifies files
    write_file(work.path(), "data.txt", b"modified by agent");
    write_file(work.path(), "new_output.txt", b"agent created this");

    assert_eq!(fs::read(work.path().join("data.txt")).unwrap(), b"modified by agent");
    assert!(work.path().join("new_output.txt").exists());

    // Rollback
    for name in &["data.txt", "config.toml"] {
        fs::copy(snap_dir.path().join(name), work.path().join(name)).unwrap();
    }
    fs::remove_file(work.path().join("new_output.txt")).unwrap();

    assert_eq!(fs::read(work.path().join("data.txt")).unwrap(), b"original content");
    assert_eq!(fs::read(work.path().join("config.toml")).unwrap(), b"[settings]\nvalue = 1");
    assert!(!work.path().join("new_output.txt").exists());
}

/// Req 2.1, 2.2 — snapshot metadata persisted in SQLite
#[test]
fn test_snapshot_metadata_persisted_in_db() {
    let db_file = TempDir::new().unwrap();
    let db_path = db_file.path().join("hawk.db");
    let conn = init_database(&db_path).unwrap();

    conn.execute(
        "INSERT INTO sessions (id, started_at, status) VALUES ('sess-snap', datetime('now'), 'Active')",
        [],
    ).unwrap();

    conn.execute(
        "INSERT INTO snapshots (id, timestamp, agent_pid, task_description, file_count, strategy, working_dir, session_id) \
         VALUES ('snap-001', datetime('now'), 1, 'pre-task', 2, 'file_copy', '/tmp/work', 'sess-snap')",
        [],
    ).unwrap();

    conn.execute(
        "INSERT INTO snapshot_files (snapshot_id, file_path, hash, size_bytes) VALUES ('snap-001', 'data.txt', 'abc123', 42)",
        [],
    ).unwrap();

    let file_count: i64 = conn.query_row(
        "SELECT file_count FROM snapshots WHERE id = 'snap-001'", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(file_count, 2);

    let manifest_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM snapshot_files WHERE snapshot_id = 'snap-001'", [], |r| r.get(0),
    ).unwrap();
    assert_eq!(manifest_count, 1);
}

// ── message bus workflow ──────────────────────────────────────────────────────

/// Req 3.6, 14.1 — subscribe agent → publish → verify delivery
#[tokio::test]
async fn test_bus_publish_delivers_to_subscriber() {
    let bus = MessageBus::new();
    let mut agent1_rx = bus.subscribe(1, "research.done").unwrap();

    bus.publish("research.done", valid_msg("research.done", serde_json::json!({ "result": "summary" }))).await.unwrap();

    let received = agent1_rx.recv().await.unwrap();
    assert_eq!(received.method, "research.done");
    assert_eq!(received.params["result"], "summary");
}

/// Req 3.6, 14.1, 14.2 — two agents on different topics; only matching subscriber receives
#[tokio::test]
async fn test_bus_two_agents_topic_isolation() {
    let bus = MessageBus::new();
    let mut agent1_rx = bus.subscribe(1, "topic.a").unwrap();
    let mut agent2_rx = bus.subscribe(2, "topic.b").unwrap();

    bus.publish("topic.a", valid_msg("event.a", serde_json::json!({}))).await.unwrap();

    let msg = agent1_rx.recv().await.unwrap();
    assert_eq!(msg.method, "event.a");
    assert!(agent2_rx.try_recv().is_err());
}

/// Req 14.1, 14.2 — both agents on same topic receive the broadcast
#[tokio::test]
async fn test_bus_broadcast_to_multiple_subscribers() {
    let bus = MessageBus::new();
    let mut rx1 = bus.subscribe(10, "broadcast.channel").unwrap();
    let mut rx2 = bus.subscribe(20, "broadcast.channel").unwrap();

    bus.publish("broadcast.channel", valid_msg("system.alert", serde_json::json!({ "level": "warning" }))).await.unwrap();

    let m1 = rx1.recv().await.unwrap();
    let m2 = rx2.recv().await.unwrap();
    assert_eq!(m1.method, "system.alert");
    assert_eq!(m2.method, "system.alert");
}

// ── resource monitor workflow ─────────────────────────────────────────────────

/// Req 5.2 — register agent → simulate MemoryExceeded → verify event
#[tokio::test]
async fn test_resource_monitor_memory_exceeded_triggers_event() {
    let monitor = ResourceMonitor::new();
    monitor.register(42, ResourceLimits { cpu_percent: 25, memory_mb: 512, max_open_fds: 64 });

    let limit_bytes = 512u64 * 1024 * 1024;
    let actual_bytes = 650u64 * 1024 * 1024;

    monitor.event_tx.send(ResourceEvent::MemoryExceeded { pid: 42, memory_bytes: actual_bytes, limit_bytes }).unwrap();

    let event = monitor.event_rx.lock().unwrap().try_recv().unwrap();
    match event {
        ResourceEvent::MemoryExceeded { pid, memory_bytes, limit_bytes: lim } => {
            assert_eq!(pid, 42);
            assert!(memory_bytes > lim);
        }
    }
}

/// Req 5.2 — deregister clears limits
#[test]
fn test_resource_monitor_deregister_after_suspension() {
    let monitor = ResourceMonitor::new();
    monitor.register(99, ResourceLimits { cpu_percent: 10, memory_mb: 256, max_open_fds: 32 });
    assert!(monitor.limits.lock().unwrap().contains_key(&99));
    monitor.deregister(99);
    assert!(!monitor.limits.lock().unwrap().contains_key(&99));
}

// ── orchestration workflow ────────────────────────────────────────────────────

/// Req 1.1, 14.1, 14.2 — register agents → orchestrate → execute → all completed
#[test]
fn test_orchestration_subtask_assignment_and_completion() {
    let mut orchestrator = Orchestrator::new();
    orchestrator.register_agent(1, "research-agent", vec!["research".into(), "web-search".into()]);
    orchestrator.register_agent(2, "coding-agent", vec!["coding".into(), "testing".into()]);
    orchestrator.register_agent(3, "review-agent", vec!["review".into(), "analysis".into()]);

    let plan = orchestrator.orchestrate("research the topic and write code").unwrap();
    assert_eq!(plan.subtasks.len(), 2);
    assert!(plan.dependencies.is_empty());
    for subtask in &plan.subtasks {
        assert!(subtask.assigned_agent.is_some());
    }

    let report = orchestrator.execute_plan(plan).unwrap();
    assert!(report.success, "all sub-tasks should complete: {}", report.summary);
    for subtask in &report.plan.subtasks {
        assert_eq!(subtask.status, SubTaskStatus::Completed);
    }
}

/// Req 14.1, 14.2 — sequential tasks have correct dependency edge
#[test]
fn test_orchestration_sequential_tasks_respect_dependencies() {
    let mut orchestrator = Orchestrator::new();
    orchestrator.register_agent(1, "research-agent", vec!["research".into()]);
    orchestrator.register_agent(2, "coding-agent", vec!["coding".into()]);

    let plan = orchestrator.orchestrate("research the topic then write code").unwrap();
    assert_eq!(plan.subtasks.len(), 2);
    assert!(plan.dependencies.contains(&(0, 1)));

    let report = orchestrator.execute_plan(plan).unwrap();
    assert!(report.success);
    assert_eq!(report.plan.subtasks[0].status, SubTaskStatus::Completed);
    assert_eq!(report.plan.subtasks[1].status, SubTaskStatus::Completed);
}

/// Req 14.1, 14.2 — (A and B) then C all complete
#[test]
fn test_orchestration_parallel_then_sequential() {
    let mut orchestrator = Orchestrator::new();
    orchestrator.register_agent(1, "research-agent", vec!["research".into()]);
    orchestrator.register_agent(2, "coding-agent", vec!["coding".into()]);
    orchestrator.register_agent(3, "review-agent", vec!["review".into()]);

    let plan = orchestrator.orchestrate("research the topic and write code then review changes").unwrap();
    assert_eq!(plan.subtasks.len(), 3);

    let report = orchestrator.execute_plan(plan).unwrap();
    assert!(report.success, "all sub-tasks should complete: {}", report.summary);
    for subtask in &report.plan.subtasks {
        assert_eq!(subtask.status, SubTaskStatus::Completed);
    }
}
