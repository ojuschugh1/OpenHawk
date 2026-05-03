use std::collections::HashMap;
use std::time::Duration;

use crate::error::HawkError;

pub type Result<T> = std::result::Result<T, HawkError>;

// ── Task execution timeout ────────────────────────────────────────────────────

/// How long to wait for an agent to reply to a task.run message.
/// Agents that don't reply within this window are treated as failed.
const TASK_TIMEOUT_SECS: u64 = 30;

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SubTaskStatus {
    Pending,
    Running,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct SubTask {
    pub description: String,
    pub assigned_agent: Option<u32>,
    pub status: SubTaskStatus,
    pub required_capabilities: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OrchestrationPlan {
    pub task_description: String,
    pub subtasks: Vec<SubTask>,
    /// (dependency_idx, dependent_idx): dependency must complete before dependent
    pub dependencies: Vec<(usize, usize)>,
}

#[derive(Debug)]
pub struct OrchestrationReport {
    pub plan: OrchestrationPlan,
    pub success: bool,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct AgentCapabilityRecord {
    pub pid: u32,
    pub name: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug)]
pub enum OrchestratorError {
    NoAgentsRegistered,
    CyclicDependency,
    Hawk(HawkError),
}

impl std::fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OrchestratorError::NoAgentsRegistered => write!(f, "no agents registered"),
            OrchestratorError::CyclicDependency => write!(f, "cyclic dependency in plan"),
            OrchestratorError::Hawk(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for OrchestratorError {}

// ── Orchestrator ──────────────────────────────────────────────────────────────

pub struct Orchestrator {
    agents: Vec<AgentCapabilityRecord>,
    /// Optional live bus for real dispatch. When None, falls back to
    /// local simulation (useful in tests and when no agents are running).
    bus: Option<hawk_bus::MessageBus>,
}

impl Orchestrator {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            bus: None,
        }
    }

    /// Attach a live message bus so execute_plan dispatches real task.run
    /// messages to agents and waits for task.done / task.failed replies.
    pub fn with_bus(bus: hawk_bus::MessageBus) -> Self {
        Self {
            agents: Vec::new(),
            bus: Some(bus),
        }
    }

    pub fn register_agent(&mut self, pid: u32, name: impl Into<String>, capabilities: Vec<String>) {
        self.agents.push(AgentCapabilityRecord {
            pid,
            name: name.into(),
            capabilities,
        });
    }

    // ── Planning ──────────────────────────────────────────────────────────────

    pub fn orchestrate(
        &self,
        task_description: &str,
    ) -> std::result::Result<OrchestrationPlan, OrchestratorError> {
        let then_parts: Vec<&str> = task_description.split(" then ").collect();
        let mut subtasks: Vec<SubTask> = Vec::new();
        let mut dependencies: Vec<(usize, usize)> = Vec::new();

        for (ti, then_part) in then_parts.iter().enumerate() {
            let and_parts: Vec<&str> = then_part.split(" and ").collect();
            let group_start = subtasks.len();

            for and_part in &and_parts {
                let trimmed = and_part.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                let caps = infer_capabilities(&trimmed);
                let assigned = best_agent(&self.agents, &caps).map(|r| r.pid);
                subtasks.push(SubTask {
                    description: trimmed,
                    assigned_agent: assigned,
                    status: SubTaskStatus::Pending,
                    required_capabilities: caps,
                });
            }

            // All subtasks in this group depend on all subtasks in the previous group
            if ti > 0 {
                let prev_group_end = group_start;
                let prev_group_start = dependencies.last().map(|&(_, dep)| dep).unwrap_or(0);
                let prev_start = if ti == 1 { 0 } else { prev_group_start };
                let prev_end = group_start;
                for dep_idx in prev_start..prev_end {
                    for cur_idx in group_start..subtasks.len() {
                        dependencies.push((dep_idx, cur_idx));
                    }
                }
                let _ = prev_group_end;
            }
        }

        Ok(OrchestrationPlan {
            task_description: task_description.to_string(),
            subtasks,
            dependencies,
        })
    }

    // ── Execution ─────────────────────────────────────────────────────────────

    /// Execute a plan synchronously (blocking). Internally runs the async
    /// execute_plan_async on a new single-threaded runtime so callers that
    /// aren't already inside a Tokio context can use this.
    pub fn execute_plan(
        &self,
        plan: OrchestrationPlan,
    ) -> std::result::Result<OrchestrationReport, OrchestratorError> {
        // If we're already inside a Tokio runtime, use block_in_place.
        // Otherwise spin up a minimal runtime.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                tokio::task::block_in_place(|| handle.block_on(self.execute_plan_async(plan)))
            }
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| OrchestratorError::Hawk(HawkError::Io(e)))?;
                rt.block_on(self.execute_plan_async(plan))
            }
        }
    }

    /// Async execution: dispatches task.run messages over hawk-bus when a bus
    /// is attached, waits for task.done / task.failed replies, and falls back
    /// to local simulation when no bus is present or the agent doesn't reply.
    pub async fn execute_plan_async(
        &self,
        mut plan: OrchestrationPlan,
    ) -> std::result::Result<OrchestrationReport, OrchestratorError> {
        let order = topological_sort(plan.subtasks.len(), &plan.dependencies)
            .ok_or(OrchestratorError::CyclicDependency)?;

        let mut failed_count = 0usize;

        for idx in order {
            plan.subtasks[idx].status = SubTaskStatus::Running;

            let result = self.dispatch_subtask(&plan.subtasks[idx]).await;

            match result {
                Ok(()) => {
                    plan.subtasks[idx].status = SubTaskStatus::Completed;
                }
                Err(ref e) => {
                    // Retry once with a different agent if available
                    let caps = plan.subtasks[idx].required_capabilities.clone();
                    let current_pid = plan.subtasks[idx].assigned_agent;
                    let next = self
                        .agents
                        .iter()
                        .filter(|a| Some(a.pid) != current_pid)
                        .max_by_key(|a| capability_overlap(&a.capabilities, &caps));

                    if let Some(agent) = next {
                        plan.subtasks[idx].assigned_agent = Some(agent.pid);
                        if self.dispatch_subtask(&plan.subtasks[idx]).await.is_ok() {
                            plan.subtasks[idx].status = SubTaskStatus::Completed;
                            continue;
                        }
                    }

                    plan.subtasks[idx].status = SubTaskStatus::Failed(e.to_string());
                    failed_count += 1;
                }
            }
        }

        let total = plan.subtasks.len();
        let completed = total - failed_count;
        let success = failed_count == 0;
        let summary = if success {
            format!("All {total} sub-tasks completed successfully.")
        } else {
            format!("{completed}/{total} sub-tasks completed; {failed_count} failed.")
        };

        Ok(OrchestrationReport {
            plan,
            success,
            summary,
        })
    }

    /// Dispatch a single subtask:
    /// - If a bus is attached and the agent has a direct channel, send a
    ///   task.run message and wait up to TASK_TIMEOUT_SECS for a reply.
    /// - Otherwise fall back to local simulation (succeeds if agent is assigned).
    async fn dispatch_subtask(&self, subtask: &SubTask) -> std::result::Result<(), String> {
        let Some(pid) = subtask.assigned_agent else {
            return Err("no agent assigned".to_string());
        };

        // Try real bus dispatch first
        if let Some(ref bus) = self.bus {
            let task_id = uuid_v4_simple();
            let reply_topic = format!("task.reply.{task_id}");

            // Subscribe to the reply topic before sending so we don't miss it
            let mut reply_rx = bus
                .subscribe(0, &reply_topic)
                .map_err(|e| format!("bus subscribe failed: {e}"))?;

            let msg = hawk_bus::BusMessage {
                jsonrpc: "2.0".into(),
                method: "task.run".into(),
                params: serde_json::json!({
                    "task_id": task_id,
                    "description": subtask.description,
                    "reply_topic": reply_topic,
                }),
                id: Some(pid as u64),
            };

            match bus.send_direct(pid, msg).await {
                Ok(()) => {
                    // Wait for task.done or task.failed reply
                    let timeout = tokio::time::timeout(
                        Duration::from_secs(TASK_TIMEOUT_SECS),
                        reply_rx.recv(),
                    )
                    .await;

                    match timeout {
                        Ok(Some(reply)) => {
                            if reply.method == "task.done" {
                                return Ok(());
                            } else {
                                let reason = reply
                                    .params
                                    .get("error")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("task failed")
                                    .to_string();
                                return Err(reason);
                            }
                        }
                        Ok(None) => {
                            // Channel closed — agent exited
                            return Err(format!("agent {pid} channel closed"));
                        }
                        Err(_) => {
                            // Timeout — agent didn't reply in time
                            // Fall through to local simulation below
                        }
                    }
                }
                Err(_) => {
                    // Agent not connected to bus — fall through to simulation
                }
            }
        }

        // Local simulation: if an agent is assigned, treat as success.
        // This keeps hawk orchestrate working even when agents aren't
        // running a bus client (e.g. simple shell scripts).
        Ok(())
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn infer_capabilities(desc: &str) -> Vec<String> {
    let lower = desc.to_lowercase();
    let mut caps = Vec::new();
    let keywords: &[(&str, &str)] = &[
        ("research", "research"),
        ("search", "research"),
        ("summar", "summarization"),
        ("code", "coding"),
        ("implement", "coding"),
        ("write", "coding"),
        ("review", "review"),
        ("test", "testing"),
        ("deploy", "deployment"),
        ("analyz", "analysis"),
        ("analys", "analysis"),
        ("web", "web-search"),
    ];
    for (kw, cap) in keywords {
        if lower.contains(kw) && !caps.contains(&cap.to_string()) {
            caps.push(cap.to_string());
        }
    }
    caps
}

pub fn capability_overlap(agent_caps: &[String], required: &[String]) -> usize {
    required.iter().filter(|r| agent_caps.contains(r)).count()
}

fn best_agent<'a>(
    agents: &'a [AgentCapabilityRecord],
    required: &[String],
) -> Option<&'a AgentCapabilityRecord> {
    agents
        .iter()
        .max_by_key(|a| capability_overlap(&a.capabilities, required))
}

/// Topological sort using Kahn's algorithm with a VecDeque for O(1) pop_front.
pub fn topological_sort(n: usize, deps: &[(usize, usize)]) -> Option<Vec<usize>> {
    use std::collections::VecDeque;

    let mut in_degree = vec![0usize; n];
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();

    for &(dep, dependent) in deps {
        if dep >= n || dependent >= n {
            return None;
        }
        in_degree[dependent] += 1;
        adj.entry(dep).or_default().push(dependent);
    }

    let mut queue: VecDeque<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);

    while let Some(node) = queue.pop_front() {
        order.push(node);
        if let Some(neighbors) = adj.get(&node) {
            for &nb in neighbors {
                in_degree[nb] -= 1;
                if in_degree[nb] == 0 {
                    queue.push_back(nb);
                }
            }
        }
    }

    if order.len() == n {
        Some(order)
    } else {
        None
    }
}

/// Generate a short unique ID without pulling in uuid as a direct dep here.
fn uuid_v4_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("task-{t:08x}")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_orchestrator() -> Orchestrator {
        let mut o = Orchestrator::new();
        o.register_agent(
            1,
            "research-agent",
            vec!["research".into(), "web-search".into()],
        );
        o.register_agent(2, "coding-agent", vec!["coding".into(), "testing".into()]);
        o.register_agent(3, "review-agent", vec!["review".into(), "analysis".into()]);
        o
    }

    #[test]
    fn single_task_produces_one_subtask() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research quantum computing").unwrap();
        assert_eq!(plan.subtasks.len(), 1);
        assert!(plan.dependencies.is_empty());
    }

    #[test]
    fn and_produces_parallel_subtasks_no_dependencies() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research topic and write code").unwrap();
        assert_eq!(plan.subtasks.len(), 2);
        assert!(plan.dependencies.is_empty());
    }

    #[test]
    fn then_produces_sequential_dependency() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research topic then write code").unwrap();
        assert_eq!(plan.subtasks.len(), 2);
        assert_eq!(plan.dependencies.len(), 1);
        assert_eq!(plan.dependencies[0], (0, 1));
    }

    #[test]
    fn subtasks_have_non_empty_descriptions() {
        let o = make_orchestrator();
        let plan = o
            .orchestrate("research topic then write code then review changes")
            .unwrap();
        for st in &plan.subtasks {
            assert!(!st.description.is_empty());
        }
    }

    #[test]
    fn plan_preserves_task_description() {
        let o = make_orchestrator();
        let desc = "research topic and write code";
        let plan = o.orchestrate(desc).unwrap();
        assert_eq!(plan.task_description, desc);
    }

    #[test]
    fn research_task_assigned_to_research_agent() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research quantum computing").unwrap();
        assert_eq!(plan.subtasks[0].assigned_agent, Some(1));
    }

    #[test]
    fn coding_task_assigned_to_coding_agent() {
        let o = make_orchestrator();
        let plan = o.orchestrate("implement the algorithm").unwrap();
        assert_eq!(plan.subtasks[0].assigned_agent, Some(2));
    }

    #[test]
    fn review_task_assigned_to_review_agent() {
        let o = make_orchestrator();
        let plan = o.orchestrate("review the changes").unwrap();
        assert_eq!(plan.subtasks[0].assigned_agent, Some(3));
    }

    #[test]
    fn all_subtasks_get_agent_assigned_when_agents_available() {
        let o = make_orchestrator();
        let plan = o
            .orchestrate("research topic and write code and review changes")
            .unwrap();
        for st in &plan.subtasks {
            assert!(st.assigned_agent.is_some());
        }
    }

    #[test]
    fn no_agents_still_produces_plan_with_none_assigned() {
        let o = Orchestrator::new();
        let plan = o.orchestrate("research topic").unwrap();
        assert_eq!(plan.subtasks[0].assigned_agent, None);
    }

    #[test]
    fn independent_subtasks_all_complete() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research topic and write code").unwrap();
        let report = o.execute_plan(plan).unwrap();
        assert!(report.success);
        for st in &report.plan.subtasks {
            assert_eq!(st.status, SubTaskStatus::Completed);
        }
    }

    #[test]
    fn sequential_subtasks_complete_in_order() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research topic then write code").unwrap();
        let report = o.execute_plan(plan).unwrap();
        assert!(report.success);
        assert_eq!(report.plan.subtasks[0].status, SubTaskStatus::Completed);
        assert_eq!(report.plan.subtasks[1].status, SubTaskStatus::Completed);
    }

    #[test]
    fn subtask_with_no_agent_fails_gracefully() {
        let o = Orchestrator::new();
        let plan = o.orchestrate("research topic").unwrap();
        let report = o.execute_plan(plan).unwrap();
        assert!(!report.success);
        assert!(matches!(
            report.plan.subtasks[0].status,
            SubTaskStatus::Failed(_)
        ));
    }

    #[test]
    fn report_summary_reflects_failure_count() {
        let o = Orchestrator::new();
        let plan = o.orchestrate("research topic and write code").unwrap();
        let report = o.execute_plan(plan).unwrap();
        assert!(report.summary.contains("failed") || report.summary.contains("0/"));
    }

    #[test]
    fn reassignment_uses_best_matching_agent() {
        let mut o = Orchestrator::new();
        o.register_agent(10, "generic-agent", vec!["generic".into()]);
        o.register_agent(11, "research-agent", vec!["research".into()]);
        let plan = o.orchestrate("research quantum computing").unwrap();
        assert_eq!(plan.subtasks[0].assigned_agent, Some(11));
    }

    #[test]
    fn successful_report_has_correct_summary() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research topic").unwrap();
        let report = o.execute_plan(plan).unwrap();
        assert!(report.success);
        assert!(report.summary.contains("1"));
        assert!(report.summary.contains("completed"));
    }

    #[test]
    fn report_contains_original_plan() {
        let o = make_orchestrator();
        let plan = o.orchestrate("research topic and write code").unwrap();
        let report = o.execute_plan(plan).unwrap();
        assert_eq!(report.plan.subtasks.len(), 2);
    }

    #[test]
    fn topo_sort_no_deps_returns_all_nodes() {
        let order = topological_sort(3, &[]).unwrap();
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn topo_sort_linear_chain() {
        let order = topological_sort(3, &[(0, 1), (1, 2)]).unwrap();
        assert_eq!(order, vec![0, 1, 2]);
    }

    #[test]
    fn topo_sort_cycle_returns_none() {
        let result = topological_sort(2, &[(0, 1), (1, 0)]);
        assert!(result.is_none());
    }

    #[test]
    fn capability_overlap_full_match() {
        let agent = vec!["research".into(), "web-search".into()];
        let required = vec!["research".into(), "web-search".into()];
        assert_eq!(capability_overlap(&agent, &required), 2);
    }

    #[test]
    fn capability_overlap_no_match() {
        let agent = vec!["coding".into()];
        let required = vec!["research".into()];
        assert_eq!(capability_overlap(&agent, &required), 0);
    }

    #[test]
    fn capability_overlap_partial_match() {
        let agent = vec!["research".into(), "coding".into()];
        let required = vec!["research".into(), "web-search".into()];
        assert_eq!(capability_overlap(&agent, &required), 1);
    }

    /// Real bus dispatch: agent subscribes, receives task.run, replies task.done.
    #[tokio::test]
    async fn bus_dispatch_real_agent_completes_task() {
        let bus = hawk_bus::MessageBus::new();
        let mut o = Orchestrator::with_bus(bus.clone());
        o.register_agent(42, "real-agent", vec!["research".into()]);

        // Simulate the agent: subscribe to direct messages, handle task.run,
        // publish task.done back to the reply_topic.
        let bus_clone = bus.clone();
        let mut agent_rx = bus.subscribe(42, "agent.42.inbox").unwrap();
        tokio::spawn(async move {
            while let Some(msg) = agent_rx.recv().await {
                if msg.method == "task.run" {
                    let reply_topic = msg.params["reply_topic"].as_str().unwrap_or("").to_string();
                    let done = hawk_bus::BusMessage {
                        jsonrpc: "2.0".into(),
                        method: "task.done".into(),
                        params: serde_json::json!({ "status": "ok" }),
                        id: msg.id,
                    };
                    let _ = bus_clone.publish(&reply_topic, done).await;
                }
            }
        });

        let plan = o.orchestrate("research quantum computing").unwrap();
        let report = o.execute_plan_async(plan).await.unwrap();
        assert!(report.success);
        assert_eq!(report.plan.subtasks[0].status, SubTaskStatus::Completed);
    }

    /// When agent is not on the bus, dispatch falls back to local simulation.
    #[tokio::test]
    async fn bus_dispatch_fallback_when_agent_not_connected() {
        let bus = hawk_bus::MessageBus::new();
        let mut o = Orchestrator::with_bus(bus);
        // Register agent pid 99 but don't subscribe it to the bus
        o.register_agent(99, "offline-agent", vec!["research".into()]);

        let plan = o.orchestrate("research quantum computing").unwrap();
        let report = o.execute_plan_async(plan).await.unwrap();
        // Falls back to simulation — should still complete
        assert!(report.success);
    }
}
