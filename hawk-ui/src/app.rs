// Application state and logic for HawkEye TUI

use crossterm::event::KeyCode;
use hawk_core::types::{AgentStatus, LifecycleState, ProcessId};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum AlertSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub timestamp: String,
    pub message: String,
    pub severity: AlertSeverity,
}

/// A single node in the orchestration dependency graph shown in HawkEye.
#[derive(Debug, Clone)]
pub struct OrchestrationNode {
    pub index: usize,
    pub description: String,
    pub assigned_agent: Option<u32>,
    pub status: OrchestrationNodeStatus,
    pub depends_on: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrchestrationNodeStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
}

impl OrchestrationNodeStatus {
    pub fn label(&self) -> &str {
        match self {
            OrchestrationNodeStatus::Pending => "Pending",
            OrchestrationNodeStatus::Running => "Running",
            OrchestrationNodeStatus::Completed => "Completed",
            OrchestrationNodeStatus::Failed(_) => "Failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HawkEyeView {
    Dashboard,
    AgentDetail(ProcessId),
    AlertsPanel,
    OrchestrationGraph,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppAction {
    SelectNext,
    SelectPrev,
    OpenDetail,
    CloseDetail,
    Quit,
    StartSearch,
    SwitchPanel,
    UndoSelected,
    SetStatusMessage(String),
}

pub struct HawkEyeApp {
    pub agents: Vec<AgentStatus>,
    pub alerts: Vec<Alert>,
    pub selected_agent: Option<usize>,
    pub view: HawkEyeView,
    pub search_query: String,
    pub status_message: Option<String>,
    pub orchestration_nodes: Vec<OrchestrationNode>,
}

impl HawkEyeApp {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            alerts: Vec::new(),
            selected_agent: None,
            view: HawkEyeView::Dashboard,
            search_query: String::new(),
            status_message: None,
            orchestration_nodes: Vec::new(),
        }
    }

    pub fn handle_key(&self, key: KeyCode) -> Option<AppAction> {
        match key {
            KeyCode::Char('q') => Some(AppAction::Quit),
            KeyCode::Char('j') | KeyCode::Down => Some(AppAction::SelectNext),
            KeyCode::Char('k') | KeyCode::Up => Some(AppAction::SelectPrev),
            KeyCode::Enter => Some(AppAction::OpenDetail),
            KeyCode::Esc => Some(AppAction::CloseDetail),
            KeyCode::Char('/') => Some(AppAction::StartSearch),
            KeyCode::Tab => Some(AppAction::SwitchPanel),
            KeyCode::Char('u') => Some(AppAction::UndoSelected),
            _ => None,
        }
    }

    pub fn apply_action(&mut self, action: AppAction) {
        match action {
            AppAction::SelectNext => {
                let len = self.filtered_agents().len();
                if len == 0 { return; }
                self.selected_agent = Some(match self.selected_agent {
                    None => 0,
                    Some(i) => (i + 1).min(len - 1),
                });
            }
            AppAction::SelectPrev => {
                let len = self.filtered_agents().len();
                if len == 0 { return; }
                self.selected_agent = Some(match self.selected_agent {
                    None => 0,
                    Some(0) => 0,
                    Some(i) => i - 1,
                });
            }
            AppAction::OpenDetail => {
                if let Some(idx) = self.selected_agent {
                    let filtered = self.filtered_agents();
                    if let Some(agent) = filtered.get(idx) {
                        self.view = HawkEyeView::AgentDetail(agent.pid);
                    }
                }
            }
            AppAction::CloseDetail => {
                self.view = HawkEyeView::Dashboard;
            }
            AppAction::SwitchPanel => {
                self.view = match &self.view {
                    HawkEyeView::Dashboard => HawkEyeView::AlertsPanel,
                    HawkEyeView::AlertsPanel => HawkEyeView::OrchestrationGraph,
                    HawkEyeView::OrchestrationGraph => HawkEyeView::Dashboard,
                    HawkEyeView::AgentDetail(_) => HawkEyeView::Dashboard,
                };
            }
            AppAction::StartSearch => {
                self.status_message = Some("Search: ".to_string());
            }
            AppAction::UndoSelected => {
                if let Some(idx) = self.selected_agent {
                    let filtered = self.filtered_agents();
                    if let Some(agent) = filtered.get(idx) {
                        let pid = agent.pid;
                        let result = try_rollback(pid);
                        self.status_message = Some(result);
                    } else {
                        self.status_message = Some("No agent selected.".to_string());
                    }
                } else {
                    self.status_message = Some("No agent selected.".to_string());
                }
            }
            AppAction::SetStatusMessage(msg) => {
                self.status_message = Some(msg);
            }
            AppAction::Quit => {}
        }
    }

    pub fn filtered_agents(&self) -> Vec<&AgentStatus> {
        if self.search_query.is_empty() {
            self.agents.iter().collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.agents.iter().filter(|a| a.name.to_lowercase().contains(&q)).collect()
        }
    }

    pub fn sorted_alerts(&self) -> Vec<&Alert> {
        let mut refs: Vec<&Alert> = self.alerts.iter().collect();
        refs.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        refs
    }
}

impl Default for HawkEyeApp {
    fn default() -> Self {
        Self::new()
    }
}

fn try_rollback(pid: ProcessId) -> String {
    let db_path = dirs_next::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("hawk")
        .join("hawk.db");
    let snap_dir = hawk_core::agent_manager::snapshot_dir();
    let db = match hawk_core::db::init_database(&db_path) {
        Ok(d) => d,
        Err(e) => return format!("DB error: {e}"),
    };
    let engine = hawk_savepoint::SnapshotEngine::new(db, snap_dir);
    match engine.rollback_latest(pid) {
        Ok(r) => format!(
            "Rolled back agent {} to snapshot {} ({} files restored).",
            pid, r.snapshot_id, r.files_restored
        ),
        Err(e) => format!("Rollback failed: {e}"),
    }
}

pub fn format_uptime(d: Duration) -> String {
    let total = d.as_secs();
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn state_label(state: &LifecycleState) -> &'static str {
    match state {
        LifecycleState::Starting => "Starting",
        LifecycleState::Running => "Running",
        LifecycleState::Paused => "Paused",
        LifecycleState::Stopping => "Stopping",
        LifecycleState::Stopped => "Stopped",
        LifecycleState::Failed => "Failed",
    }
}
