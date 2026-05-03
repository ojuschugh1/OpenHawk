// hawk-bus: JSON-RPC 2.0 in-memory message bus with pub/sub and direct messaging

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum BusError {
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("no subscriber for pid {0}")]
    NoSubscriber(u32),
    #[error("queue full (max {0} messages)")]
    QueueFull(usize),
    #[error("topic not found: {0}")]
    TopicNotFound(String),
    #[error("not subscribed: pid {0} on topic {1}")]
    NotSubscribed(u32, String),
}

// ── Message ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusMessage {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    pub id: Option<u64>,
}

impl BusMessage {
    pub fn validate(&self) -> Result<(), BusError> {
        if self.jsonrpc != "2.0" {
            return Err(BusError::InvalidMessage(format!(
                "jsonrpc must be \"2.0\", got {:?}",
                self.jsonrpc
            )));
        }
        if self.method.is_empty() {
            return Err(BusError::InvalidMessage("method must not be empty".into()));
        }
        Ok(())
    }
}

// ── Inspection types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub name: String,
    pub subscriber_count: usize,
    pub pending_count: usize,
}

#[derive(Debug, Clone)]
pub struct BusInspection {
    pub topics: Vec<TopicInfo>,
    pub pending_messages: usize,
}

// ── Queued message ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct QueuedMessage {
    topic: Option<String>,
    message: BusMessage,
    created_at: u64,
    expires_at: u64,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Inner state ───────────────────────────────────────────────────────────────

struct BusState {
    subscriptions: HashMap<String, Vec<(u32, mpsc::Sender<BusMessage>)>>,
    direct: HashMap<u32, mpsc::Sender<BusMessage>>,
    queue: HashMap<u32, Vec<QueuedMessage>>,
    max_queue: usize,
    retention_secs: u64,
}

impl BusState {
    fn new(max_queue: usize, retention_secs: u64) -> Self {
        Self {
            subscriptions: HashMap::new(),
            direct: HashMap::new(),
            queue: HashMap::new(),
            max_queue,
            retention_secs,
        }
    }
}

// ── MessageBus ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MessageBus {
    state: Arc<Mutex<BusState>>,
}

impl MessageBus {
    pub fn new() -> Self {
        Self::with_config(10_000, 3600)
    }

    pub fn with_config(max_queue: usize, retention_secs: u64) -> Self {
        Self { state: Arc::new(Mutex::new(BusState::new(max_queue, retention_secs))) }
    }

    pub async fn publish(&self, topic: &str, message: BusMessage) -> Result<(), BusError> {
        message.validate()?;
        let senders: Vec<mpsc::Sender<BusMessage>> = {
            let state = self.state.lock().unwrap();
            state
                .subscriptions
                .get(topic)
                .map(|subs| subs.iter().map(|(_, tx)| tx.clone()).collect())
                .unwrap_or_default()
        };
        for tx in senders {
            let _ = tx.send(message.clone()).await;
        }
        Ok(())
    }

    pub async fn send_direct(&self, target_pid: u32, message: BusMessage) -> Result<(), BusError> {
        message.validate()?;
        let tx = {
            let state = self.state.lock().unwrap();
            state.direct.get(&target_pid).cloned()
        };
        match tx {
            Some(sender) => {
                let _ = sender.send(message).await;
                Ok(())
            }
            None => Err(BusError::NoSubscriber(target_pid)),
        }
    }

    pub fn subscribe(&self, pid: u32, topic: &str) -> Result<mpsc::Receiver<BusMessage>, BusError> {
        let (tx, rx) = mpsc::channel(256);
        let mut state = self.state.lock().unwrap();
        state
            .subscriptions
            .entry(topic.to_owned())
            .or_default()
            .push((pid, tx.clone()));
        state.direct.insert(pid, tx);
        Ok(rx)
    }

    pub fn unsubscribe(&self, pid: u32, topic: &str) -> Result<(), BusError> {
        let mut state = self.state.lock().unwrap();
        let subs = state
            .subscriptions
            .get_mut(topic)
            .ok_or_else(|| BusError::TopicNotFound(topic.to_owned()))?;
        let before = subs.len();
        subs.retain(|(p, _)| *p != pid);
        if subs.len() == before {
            return Err(BusError::NotSubscribed(pid, topic.to_owned()));
        }
        Ok(())
    }

    pub fn inspect(&self) -> BusInspection {
        let state = self.state.lock().unwrap();
        let mut total_pending = 0usize;
        let topics: Vec<TopicInfo> = state
            .subscriptions
            .iter()
            .map(|(name, subs)| {
                let pending: usize = state
                    .queue
                    .values()
                    .flat_map(|msgs| msgs.iter())
                    .filter(|m| m.topic.as_deref() == Some(name.as_str()))
                    .count();
                total_pending += pending;
                TopicInfo {
                    name: name.clone(),
                    subscriber_count: subs.len(),
                    pending_count: pending,
                }
            })
            .collect();

        let direct_pending: usize = state
            .queue
            .values()
            .flat_map(|msgs| msgs.iter())
            .filter(|m| m.topic.is_none())
            .count();
        total_pending += direct_pending;

        BusInspection { topics, pending_messages: total_pending }
    }

    pub fn queue_for_offline(
        &self,
        target_pid: u32,
        topic: Option<&str>,
        message: BusMessage,
    ) -> Result<(), BusError> {
        message.validate()?;
        let mut state = self.state.lock().unwrap();
        let max_queue = state.max_queue;
        let retention_secs = state.retention_secs;
        let queue = state.queue.entry(target_pid).or_default();
        if queue.len() >= max_queue {
            return Err(BusError::QueueFull(max_queue));
        }
        let now = unix_now();
        queue.push(QueuedMessage {
            topic: topic.map(str::to_owned),
            message,
            created_at: now,
            expires_at: now + retention_secs,
        });
        Ok(())
    }

    pub fn deliver_queued(&self, target_pid: u32) -> Vec<BusMessage> {
        let mut state = self.state.lock().unwrap();
        let now = unix_now();
        let queue = state.queue.remove(&target_pid).unwrap_or_default();
        queue.into_iter().filter(|m| m.expires_at > now).map(|m| m.message).collect()
    }

    pub fn expire_old_messages(&self, retention_secs: u64) {
        let mut state = self.state.lock().unwrap();
        let now = unix_now();
        for queue in state.queue.values_mut() {
            queue.retain(|m| m.created_at + retention_secs > now);
        }
        state.queue.retain(|_, v| !v.is_empty());
    }
}

impl Default for MessageBus {
    fn default() -> Self {
        Self::new()
    }
}

// ── CLI inspection formatting ─────────────────────────────────────────────────

pub fn format_inspection(inspection: &BusInspection) -> String {
    if inspection.topics.is_empty() {
        return format!(
            "No active topics. Pending messages (direct): {}\n",
            inspection.pending_messages
        );
    }
    let mut out = String::new();
    out.push_str(&format!("{:<30} {:>11} {:>13}\n", "TOPIC", "SUBSCRIBERS", "PENDING MSGS"));
    out.push_str(&"-".repeat(56));
    out.push('\n');
    for t in &inspection.topics {
        out.push_str(&format!(
            "{:<30} {:>11} {:>13}\n",
            t.name, t.subscriber_count, t.pending_count
        ));
    }
    out.push_str(&"-".repeat(56));
    out.push('\n');
    out.push_str(&format!("Total pending messages: {}\n", inspection.pending_messages));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_msg(method: &str) -> BusMessage {
        BusMessage {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params: serde_json::json!({}),
            id: None,
        }
    }

    #[test]
    fn valid_message_passes_validation() {
        assert!(valid_msg("do.thing").validate().is_ok());
    }

    #[test]
    fn wrong_jsonrpc_version_rejected() {
        let msg = BusMessage { jsonrpc: "1.0".into(), method: "test".into(), params: serde_json::json!({}), id: None };
        assert!(matches!(msg.validate(), Err(BusError::InvalidMessage(_))));
    }

    #[test]
    fn empty_method_rejected() {
        let msg = BusMessage { jsonrpc: "2.0".into(), method: "".into(), params: serde_json::json!({}), id: None };
        assert!(matches!(msg.validate(), Err(BusError::InvalidMessage(_))));
    }

    #[tokio::test]
    async fn publish_delivers_to_single_subscriber() {
        let bus = MessageBus::new();
        let mut rx = bus.subscribe(1, "events").unwrap();
        bus.publish("events", valid_msg("ping")).await.unwrap();
        assert_eq!(rx.recv().await.unwrap().method, "ping");
    }

    #[tokio::test]
    async fn publish_delivers_to_multiple_subscribers() {
        let bus = MessageBus::new();
        let mut rx1 = bus.subscribe(1, "news").unwrap();
        let mut rx2 = bus.subscribe(2, "news").unwrap();
        let mut rx3 = bus.subscribe(3, "news").unwrap();
        bus.publish("news", valid_msg("headline")).await.unwrap();
        assert_eq!(rx1.recv().await.unwrap().method, "headline");
        assert_eq!(rx2.recv().await.unwrap().method, "headline");
        assert_eq!(rx3.recv().await.unwrap().method, "headline");
    }

    #[tokio::test]
    async fn publish_to_empty_topic_is_ok() {
        let bus = MessageBus::new();
        assert!(bus.publish("empty-topic", valid_msg("noop")).await.is_ok());
    }

    #[tokio::test]
    async fn publish_invalid_message_returns_error() {
        let bus = MessageBus::new();
        let _ = bus.subscribe(1, "t").unwrap();
        let bad = BusMessage { jsonrpc: "1.0".into(), method: "x".into(), params: serde_json::json!({}), id: None };
        assert!(bus.publish("t", bad).await.is_err());
    }

    #[tokio::test]
    async fn send_direct_delivers_to_target_only() {
        let bus = MessageBus::new();
        let mut rx1 = bus.subscribe(10, "ch").unwrap();
        let mut rx2 = bus.subscribe(20, "ch").unwrap();
        bus.send_direct(10, valid_msg("private")).await.unwrap();
        assert_eq!(rx1.recv().await.unwrap().method, "private");
        assert!(rx2.try_recv().is_err());
    }

    #[tokio::test]
    async fn send_direct_to_unknown_pid_returns_error() {
        let bus = MessageBus::new();
        assert!(matches!(bus.send_direct(999, valid_msg("hi")).await, Err(BusError::NoSubscriber(999))));
    }

    #[tokio::test]
    async fn unsubscribe_stops_delivery() {
        let bus = MessageBus::new();
        let mut rx = bus.subscribe(1, "feed").unwrap();
        bus.publish("feed", valid_msg("first")).await.unwrap();
        assert_eq!(rx.recv().await.unwrap().method, "first");
        bus.unsubscribe(1, "feed").unwrap();
        bus.publish("feed", valid_msg("second")).await.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn unsubscribe_unknown_topic_returns_error() {
        let bus = MessageBus::new();
        assert!(matches!(bus.unsubscribe(1, "ghost"), Err(BusError::TopicNotFound(_))));
    }

    #[test]
    fn queue_for_offline_stores_message() {
        let bus = MessageBus::new();
        bus.queue_for_offline(42, Some("alerts"), valid_msg("queued")).unwrap();
        let msgs = bus.deliver_queued(42);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].method, "queued");
    }

    #[test]
    fn deliver_queued_drains_queue() {
        let bus = MessageBus::new();
        bus.queue_for_offline(7, None, valid_msg("m1")).unwrap();
        bus.queue_for_offline(7, None, valid_msg("m2")).unwrap();
        let first = bus.deliver_queued(7);
        assert_eq!(first.len(), 2);
        assert!(bus.deliver_queued(7).is_empty());
    }

    #[test]
    fn queue_full_returns_error() {
        let bus = MessageBus::with_config(2, 3600);
        bus.queue_for_offline(1, None, valid_msg("a")).unwrap();
        bus.queue_for_offline(1, None, valid_msg("b")).unwrap();
        assert!(matches!(bus.queue_for_offline(1, None, valid_msg("c")), Err(BusError::QueueFull(2))));
    }

    #[test]
    fn expire_old_messages_removes_expired() {
        let bus = MessageBus::with_config(100, 3600);
        bus.queue_for_offline(5, None, valid_msg("old")).unwrap();
        bus.expire_old_messages(0);
        assert!(bus.deliver_queued(5).is_empty());
    }

    #[test]
    fn expire_old_messages_keeps_fresh() {
        let bus = MessageBus::with_config(100, 3600);
        bus.queue_for_offline(5, None, valid_msg("fresh")).unwrap();
        bus.expire_old_messages(9999);
        assert_eq!(bus.deliver_queued(5).len(), 1);
    }

    #[test]
    fn inspect_shows_active_topics() {
        let bus = MessageBus::new();
        let _ = bus.subscribe(1, "alpha").unwrap();
        let _ = bus.subscribe(2, "alpha").unwrap();
        let _ = bus.subscribe(3, "beta").unwrap();
        let info = bus.inspect();
        let alpha = info.topics.iter().find(|t| t.name == "alpha").unwrap();
        assert_eq!(alpha.subscriber_count, 2);
        let beta = info.topics.iter().find(|t| t.name == "beta").unwrap();
        assert_eq!(beta.subscriber_count, 1);
    }

    #[test]
    fn inspect_empty_bus() {
        let bus = MessageBus::new();
        let info = bus.inspect();
        assert!(info.topics.is_empty());
        assert_eq!(info.pending_messages, 0);
    }

    #[test]
    fn format_inspection_empty() {
        let info = BusInspection { topics: vec![], pending_messages: 0 };
        assert!(format_inspection(&info).contains("No active topics"));
    }

    #[test]
    fn format_inspection_with_topics() {
        let info = BusInspection {
            topics: vec![
                TopicInfo { name: "research.done".into(), subscriber_count: 3, pending_count: 1 },
            ],
            pending_messages: 1,
        };
        let out = format_inspection(&info);
        assert!(out.contains("research.done"));
        assert!(out.contains("TOPIC"));
    }
}
