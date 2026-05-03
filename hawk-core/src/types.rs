use std::time::Duration;

pub type ProcessId = u32;
pub type SessionId = String;
pub type SnapshotId = String;

#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleState {
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct AgentStatus {
    pub pid: ProcessId,
    pub name: String,
    pub state: LifecycleState,
    pub uptime: Duration,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub open_fds: u32,
}

#[derive(Debug)]
pub enum PermissionResult {
    Allowed,
    Denied { reason: String },
}
