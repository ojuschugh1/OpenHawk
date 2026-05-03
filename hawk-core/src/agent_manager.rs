use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection};
use tokio::process::{Child, Command};
use uuid::Uuid;

use crate::error::HawkError;
use crate::manifest::AgentManifest;
use crate::types::{AgentStatus, LifecycleState, ProcessId};

pub type Result<T> = std::result::Result<T, HawkError>;

#[derive(Debug)]
pub struct AgentRecord {
    pub pid: ProcessId,
    pub name: String,
    pub state: LifecycleState,
    pub started_at: Instant,
    pub session_id: String,
    pub manifest: AgentManifest,
}

pub struct AgentManager {
    pub db: Arc<Mutex<Connection>>,
    agents: Arc<Mutex<HashMap<ProcessId, (Child, AgentRecord)>>>,
}

impl AgentManager {
    pub fn new(db: Connection) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            agents: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn spawn(&self, manifest: AgentManifest) -> Result<ProcessId> {
        if manifest.info.entry_command.trim().is_empty() {
            return Err(HawkError::InvalidManifest("entry_command must not be empty".to_string()));
        }

        let session_id = Uuid::new_v4().to_string();
        let parts: Vec<&str> = manifest.info.entry_command.split_whitespace().collect();
        let (program, args) = parts.split_first().ok_or_else(|| {
            HawkError::InvalidManifest("entry_command is empty".to_string())
        })?;

        let child = Command::new(program)
            .args(args)
            .spawn()
            .map_err(HawkError::Io)?;

        let pid = child.id().ok_or_else(|| {
            HawkError::Io(std::io::Error::new(std::io::ErrorKind::Other, "could not get child PID"))
        })?;

        let record = AgentRecord {
            pid,
            name: manifest.info.name.clone(),
            state: LifecycleState::Running,
            started_at: Instant::now(),
            session_id: session_id.clone(),
            manifest: manifest.clone(),
        };

        {
            let db = self.db.lock().unwrap();
            db.execute(
                "INSERT OR IGNORE INTO sessions (id, started_at, status) VALUES (?1, datetime('now'), 'Active')",
                params![session_id],
            )
            .map_err(|e| HawkError::Database(e.to_string()))?;

            db.execute(
                "INSERT INTO agents (pid, name, state, manifest_path, started_at, session_id) \
                 VALUES (?1, ?2, ?3, ?4, datetime('now'), ?5)",
                params![pid, manifest.info.name, "Running", manifest.info.entry_command, session_id],
            )
            .map_err(|e| HawkError::Database(e.to_string()))?;
        }

        self.agents.lock().unwrap().insert(pid, (child, record));
        Ok(pid)
    }

    pub async fn stop(&self, pid: ProcessId) -> Result<StopResult> {
        if !self.agents.lock().unwrap().contains_key(&pid) {
            return Err(HawkError::NotFound(format!("agent {pid}")));
        }

        self.update_state(pid, LifecycleState::Stopping)?;
        send_term(pid)?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut graceful = false;
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let exited = {
                let mut agents = self.agents.lock().unwrap();
                if let Some((child, _)) = agents.get_mut(&pid) {
                    child.try_wait().map_err(HawkError::Io)?.is_some()
                } else {
                    true
                }
            };
            if exited {
                graceful = true;
                break;
            }
        }

        if !graceful {
            let mut agents = self.agents.lock().unwrap();
            if let Some((child, _)) = agents.get_mut(&pid) {
                child.kill().await.map_err(HawkError::Io)?;
            }
            drop(agents);
            self.log_forced_termination(pid)?;
        }

        self.agents.lock().unwrap().remove(&pid);
        self.update_state_db(pid, LifecycleState::Stopped)?;

        Ok(StopResult { pid, forced: !graceful })
    }

    pub fn pause(&self, pid: ProcessId) -> Result<()> {
        self.require_agent(pid)?;
        send_stop(pid)?;
        self.update_state(pid, LifecycleState::Paused)
    }

    pub fn resume(&self, pid: ProcessId) -> Result<()> {
        self.require_agent(pid)?;
        send_cont(pid)?;
        self.update_state(pid, LifecycleState::Running)
    }

    pub fn list(&self) -> Vec<AgentStatus> {
        use sysinfo::{Pid, System};
        let mut sys = System::new_all();
        sys.refresh_all();

        self.agents
            .lock()
            .unwrap()
            .values()
            .map(|(_, rec)| {
                let sysinfo_pid = Pid::from_u32(rec.pid);
                let (cpu_percent, memory_bytes) = sys
                    .process(sysinfo_pid)
                    .map(|p| (p.cpu_usage(), p.memory()))
                    .unwrap_or((0.0, 0));

                AgentStatus {
                    pid: rec.pid,
                    name: rec.name.clone(),
                    state: rec.state.clone(),
                    uptime: rec.started_at.elapsed(),
                    cpu_percent,
                    memory_bytes,
                    open_fds: 0, // platform-specific; /proc/<pid>/fd on Linux
                }
            })
            .collect()
    }

    pub fn get_state(&self, pid: ProcessId) -> Option<LifecycleState> {
        self.agents.lock().unwrap().get(&pid).map(|(_, rec)| rec.state.clone())
    }

    /// Check if agent has exceeded its token budget; if so, pause it and return true.
    pub fn enforce_budget(&self, pid: ProcessId) -> Result<bool> {
        let budget = {
            let agents = self.agents.lock().unwrap();
            agents.get(&pid).map(|(_, rec)| rec.manifest.llm.budget_tokens)
        };
        let Some(budget) = budget else { return Ok(false) };
        if budget == 0 {
            return Ok(false);
        }
        let db = self.db.lock().unwrap();
        let total: i64 = db
            .query_row(
                "SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0) \
                 FROM token_usage WHERE agent_pid = ?1",
                params![pid],
                |row| row.get(0),
            )
            .map_err(|e| HawkError::Database(e.to_string()))?;
        drop(db);
        if total as u64 > budget {
            send_stop(pid)?;
            self.update_state(pid, LifecycleState::Paused)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn require_agent(&self, pid: ProcessId) -> Result<()> {
        if self.agents.lock().unwrap().contains_key(&pid) {
            Ok(())
        } else {
            Err(HawkError::NotFound(format!("agent {pid}")))
        }
    }

    fn update_state(&self, pid: ProcessId, state: LifecycleState) -> Result<()> {
        if let Some((_, rec)) = self.agents.lock().unwrap().get_mut(&pid) {
            rec.state = state.clone();
        }
        self.update_state_db(pid, state)
    }

    fn update_state_db(&self, pid: ProcessId, state: LifecycleState) -> Result<()> {
        let state_str = lifecycle_str(&state);
        self.db
            .lock()
            .unwrap()
            .execute("UPDATE agents SET state = ?1 WHERE pid = ?2", params![state_str, pid])
            .map_err(|e| HawkError::Database(e.to_string()))?;
        Ok(())
    }

    fn log_forced_termination(&self, pid: ProcessId) -> Result<()> {
        self.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO healing_events \
                 (agent_pid, timestamp, original_error, adjustment, outcome, attempt_number) \
                 VALUES (?1, datetime('now'), 'graceful stop timeout', 'force kill', 'Success', 1)",
                params![pid],
            )
            .map_err(|e| HawkError::Database(e.to_string()))?;
        Ok(())
    }
}

fn lifecycle_str(s: &LifecycleState) -> &'static str {
    match s {
        LifecycleState::Starting => "Starting",
        LifecycleState::Running => "Running",
        LifecycleState::Paused => "Paused",
        LifecycleState::Stopping => "Stopping",
        LifecycleState::Stopped => "Stopped",
        LifecycleState::Failed => "Failed",
    }
}

#[derive(Debug)]
pub struct StopResult {
    pub pid: ProcessId,
    pub forced: bool,
}

// ── platform signal helpers ───────────────────────────────────────────────────

#[cfg(target_family = "unix")]
fn send_term(pid: ProcessId) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
        .map_err(|e| HawkError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
}

#[cfg(target_family = "unix")]
fn send_stop(pid: ProcessId) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGSTOP)
        .map_err(|e| HawkError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
}

#[cfg(target_family = "unix")]
fn send_cont(pid: ProcessId) -> Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid as i32), Signal::SIGCONT)
        .map_err(|e| HawkError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
}

#[cfg(not(target_family = "unix"))]
fn send_term(_pid: ProcessId) -> Result<()> { Ok(()) }
#[cfg(not(target_family = "unix"))]
fn send_stop(_pid: ProcessId) -> Result<()> { Ok(()) }
#[cfg(not(target_family = "unix"))]
fn send_cont(_pid: ProcessId) -> Result<()> { Ok(()) }

pub fn snapshot_dir() -> PathBuf {
    dirs_next::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hawk")
        .join("snapshots")
}
