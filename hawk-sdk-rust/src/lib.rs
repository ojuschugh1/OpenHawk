// hawk-sdk-rust: client library for agent developers

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hawk_bus::{BusMessage, MessageBus};
use hawk_memory::{InMemoryStore, MemoryScope, SharedMemory};
use thiserror::Error;
use tokio::sync::mpsc::Receiver;

pub use hawk_memory::MemoryScope as Scope;

#[derive(Debug, Error)]
pub enum SdkError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("bus error: {0}")]
    Bus(#[from] hawk_bus::BusError),
    #[error("memory error: {0}")]
    Memory(#[from] hawk_memory::MemoryError),
    #[error("handler already registered for method: {0}")]
    DuplicateHandler(String),
}

type Handler = Box<dyn Fn(BusMessage) + Send + Sync>;

struct HandlerRegistry {
    handlers: HashMap<String, Handler>,
}

impl HandlerRegistry {
    fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    fn register(&mut self, method: &str, handler: Handler) -> Result<(), SdkError> {
        if self.handlers.contains_key(method) {
            return Err(SdkError::DuplicateHandler(method.to_owned()));
        }
        self.handlers.insert(method.to_owned(), handler);
        Ok(())
    }

    fn dispatch(&self, msg: &BusMessage) {
        if let Some(h) = self.handlers.get(&msg.method) {
            h(msg.clone());
        }
    }
}

pub struct HawkClient {
    pid: u32,
    bus: Arc<MessageBus>,
    memory: Arc<InMemoryStore>,
    handlers: Arc<Mutex<HandlerRegistry>>,
}

impl HawkClient {
    pub fn connect(pid: u32) -> Result<Self, SdkError> {
        Ok(Self {
            pid,
            bus: Arc::new(MessageBus::new()),
            memory: Arc::new(InMemoryStore::new()),
            handlers: Arc::new(Mutex::new(HandlerRegistry::new())),
        })
    }

    pub async fn publish(&self, topic: &str, payload: serde_json::Value) -> Result<(), SdkError> {
        let method = payload
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let jsonrpc = payload
            .get("jsonrpc")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();

        if jsonrpc != "2.0" {
            return Err(SdkError::Validation(format!(
                "jsonrpc must be \"2.0\", got {:?}",
                jsonrpc
            )));
        }
        if method.is_empty() {
            return Err(SdkError::Validation("method must not be empty".into()));
        }

        let msg = BusMessage {
            jsonrpc,
            method,
            params: payload
                .get("params")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            id: payload.get("id").and_then(|v| v.as_u64()),
        };

        self.bus.publish(topic, msg).await?;
        Ok(())
    }

    pub fn subscribe(&self, topic: &str) -> Result<Receiver<BusMessage>, SdkError> {
        Ok(self.bus.subscribe(self.pid, topic)?)
    }

    pub fn memory_get(&self, key: &str) -> Result<Option<Vec<u8>>, SdkError> {
        let entry = self.memory.query(key)?;
        Ok(entry.map(|e| e.value))
    }

    pub fn memory_set(&self, scope: MemoryScope, key: &str, value: &[u8]) -> Result<(), SdkError> {
        self.memory.store(scope, key, value, self.pid)?;
        Ok(())
    }

    pub fn register_handler(&self, method: &str, handler: Handler) -> Result<(), SdkError> {
        self.handlers.lock().unwrap().register(method, handler)
    }

    pub fn dispatch(&self, msg: &BusMessage) {
        self.handlers.lock().unwrap().dispatch(msg);
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
}

// Python bindings stub (task 18.2) — see module comment for PyO3 interface sketch
pub mod python_bindings {}

// TypeScript/Node.js bindings stub (task 18.3) — see module comment for napi-rs interface sketch
pub mod typescript_bindings {}

// ── SDK scaffold templates (task 18.4) ────────────────────────────────────────

pub mod scaffold {
    #[derive(Debug)]
    pub struct ScaffoldFiles {
        pub files: Vec<(String, String)>,
    }

    pub fn generate(language: &str, agent_name: &str) -> Result<ScaffoldFiles, String> {
        match language {
            "rust" => Ok(rust_scaffold(agent_name)),
            "python" => Ok(python_scaffold(agent_name)),
            "typescript" => Ok(typescript_scaffold(agent_name)),
            other => Err(format!(
                "unsupported language: {other}. Use rust, python, or typescript"
            )),
        }
    }

    fn manifest(name: &str, entry: &str) -> String {
        format!(
            "[agent]\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"\"\nframework = \"custom\"\nentry_command = \"{entry}\"\n\n[permissions]\nfilesystem = []\nnetwork = []\ncommands = []\nsecrets = []\n\n[resources]\ncpu_percent = 25\nmemory_mb = 256\nmax_open_fds = 32\n"
        )
    }

    fn rust_scaffold(name: &str) -> ScaffoldFiles {
        let main_rs = format!(
            "use hawk_sdk_rust::{{HawkClient, Scope}};\n\n#[tokio::main]\nasync fn main() -> anyhow::Result<()> {{\n    let pid = std::process::id();\n    let client = HawkClient::connect(pid)?;\n    client.register_handler(\"task.run\", Box::new(|msg| println!(\"Received: {{:?}}\", msg.method)))?;\n    let mut rx = client.subscribe(\"{name}.events\")?;\n    client.publish(\"{name}.status\", serde_json::json!({{\"jsonrpc\": \"2.0\", \"method\": \"agent.ready\", \"params\": {{\"agent\": \"{name}\"}}}})).await?;\n    while let Some(msg) = rx.recv().await {{ client.dispatch(&msg); }}\n    Ok(())\n}}\n"
        );
        let cargo_toml = format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nhawk-sdk-rust = {{ path = \"../hawk-sdk-rust\" }}\ntokio = {{ version = \"1\", features = [\"full\"] }}\nserde_json = \"1\"\nanyhow = \"1\"\n"
        );
        ScaffoldFiles {
            files: vec![
                ("Agent_Manifest.toml".into(), manifest(name, "cargo run")),
                ("src/main.rs".into(), main_rs),
                ("Cargo.toml".into(), cargo_toml),
            ],
        }
    }

    fn python_scaffold(name: &str) -> ScaffoldFiles {
        let main_py = format!(
            "import os\nimport hawk_bus\n\ndef on_task_run(msg):\n    print(f\"Received: {{msg['method']}}\")\n\ndef main():\n    pid = os.getpid()\n    client = hawk_bus.HawkClient.connect(pid)\n    client.register_handler(\"task.run\", on_task_run)\n    rx = client.subscribe(\"{name}.events\")\n    client.publish(\"{name}.status\", {{\"jsonrpc\": \"2.0\", \"method\": \"agent.ready\", \"params\": {{\"agent\": \"{name}\"}}}})\n    for msg in rx:\n        client.dispatch(msg)\n\nif __name__ == \"__main__\":\n    main()\n"
        );
        ScaffoldFiles {
            files: vec![
                (
                    "Agent_Manifest.toml".into(),
                    manifest(name, "python main.py"),
                ),
                ("main.py".into(), main_py),
                ("requirements.txt".into(), "hawk-sdk\n".into()),
            ],
        }
    }

    fn typescript_scaffold(name: &str) -> ScaffoldFiles {
        let index_ts = format!(
            "import {{ HawkClient, BusMessage }} from 'hawk-sdk';\n\nasync function main() {{\n  const client = HawkClient.connect(process.pid);\n  client.registerHandler('task.run', (msg: BusMessage) => console.log('Received:', msg.method));\n  const rx = client.subscribe('{name}.events');\n  await client.publish('{name}.status', {{ jsonrpc: '2.0', method: 'agent.ready', params: {{ agent: '{name}' }} }});\n  for await (const msg of rx) {{ client.dispatch(msg); }}\n}}\n\nmain().catch(console.error);\n"
        );
        let package_json = format!(
            "{{\n  \"name\": \"{name}\",\n  \"version\": \"0.1.0\",\n  \"main\": \"dist/index.js\",\n  \"scripts\": {{ \"build\": \"tsc\", \"start\": \"node dist/index.js\" }},\n  \"dependencies\": {{ \"hawk-sdk\": \"^0.1.0\" }},\n  \"devDependencies\": {{ \"typescript\": \"^5.0.0\", \"@types/node\": \"^20.0.0\" }}\n}}\n"
        );
        ScaffoldFiles {
            files: vec![
                (
                    "Agent_Manifest.toml".into(),
                    manifest(name, "node dist/index.js"),
                ),
                ("src/index.ts".into(), index_ts),
                ("package.json".into(), package_json),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hawk_memory::MemoryScope;

    fn client() -> HawkClient {
        HawkClient::connect(1234).unwrap()
    }

    #[test]
    fn connect_returns_client_with_correct_pid() {
        let c = HawkClient::connect(42).unwrap();
        assert_eq!(c.pid(), 42);
    }

    #[tokio::test]
    async fn publish_valid_message_delivers_to_subscriber() {
        let c = client();
        let mut rx = c.subscribe("test.topic").unwrap();
        c.publish(
            "test.topic",
            serde_json::json!({"jsonrpc": "2.0", "method": "do.thing", "params": {}}),
        )
        .await
        .unwrap();
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.method, "do.thing");
    }

    #[tokio::test]
    async fn publish_wrong_jsonrpc_version_returns_validation_error() {
        let c = client();
        let err = c
            .publish(
                "t",
                serde_json::json!({"jsonrpc": "1.0", "method": "x", "params": {}}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SdkError::Validation(_)));
        assert!(err.to_string().contains("jsonrpc"));
    }

    #[tokio::test]
    async fn publish_empty_method_returns_validation_error() {
        let c = client();
        let err = c
            .publish(
                "t",
                serde_json::json!({"jsonrpc": "2.0", "method": "", "params": {}}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, SdkError::Validation(_)));
        assert!(err.to_string().contains("method"));
    }

    #[tokio::test]
    async fn publish_missing_jsonrpc_field_returns_validation_error() {
        let c = client();
        let err = c
            .publish("t", serde_json::json!({"method": "x", "params": {}}))
            .await
            .unwrap_err();
        assert!(matches!(err, SdkError::Validation(_)));
    }

    #[tokio::test]
    async fn publish_missing_method_field_returns_validation_error() {
        let c = client();
        let err = c
            .publish("t", serde_json::json!({"jsonrpc": "2.0", "params": {}}))
            .await
            .unwrap_err();
        assert!(matches!(err, SdkError::Validation(_)));
    }

    #[test]
    fn memory_set_and_get_global_scope_round_trip() {
        let c = client();
        c.memory_set(MemoryScope::Global, "answer", b"42").unwrap();
        assert_eq!(c.memory_get("answer").unwrap(), Some(b"42".to_vec()));
    }

    #[test]
    fn memory_set_and_get_agent_scope_round_trip() {
        let c = client();
        c.memory_set(MemoryScope::Agent(1234), "priv", b"secret")
            .unwrap();
        assert_eq!(c.memory_get("priv").unwrap(), Some(b"secret".to_vec()));
    }

    #[test]
    fn memory_get_missing_key_returns_none() {
        let c = client();
        assert_eq!(c.memory_get("nonexistent").unwrap(), None);
    }

    #[test]
    fn register_handler_and_dispatch_calls_handler() {
        let c = client();
        let called = Arc::new(Mutex::new(false));
        let called_clone = Arc::clone(&called);
        c.register_handler(
            "ping",
            Box::new(move |_msg| {
                *called_clone.lock().unwrap() = true;
            }),
        )
        .unwrap();
        let msg = BusMessage {
            jsonrpc: "2.0".into(),
            method: "ping".into(),
            params: serde_json::json!({}),
            id: None,
        };
        c.dispatch(&msg);
        assert!(*called.lock().unwrap());
    }

    #[test]
    fn dispatch_unknown_method_does_not_panic() {
        let c = client();
        let msg = BusMessage {
            jsonrpc: "2.0".into(),
            method: "unknown".into(),
            params: serde_json::json!({}),
            id: None,
        };
        c.dispatch(&msg);
    }

    #[test]
    fn register_duplicate_handler_returns_error() {
        let c = client();
        c.register_handler("method.x", Box::new(|_| {})).unwrap();
        let err = c
            .register_handler("method.x", Box::new(|_| {}))
            .unwrap_err();
        assert!(matches!(err, SdkError::DuplicateHandler(_)));
    }

    #[test]
    fn scaffold_rust_generates_expected_files() {
        let s = scaffold::generate("rust", "my-agent").unwrap();
        let names: Vec<&str> = s.files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"Agent_Manifest.toml"));
        assert!(names.contains(&"src/main.rs"));
        assert!(names.contains(&"Cargo.toml"));
    }

    #[test]
    fn scaffold_python_generates_expected_files() {
        let s = scaffold::generate("python", "my-agent").unwrap();
        let names: Vec<&str> = s.files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"Agent_Manifest.toml"));
        assert!(names.contains(&"main.py"));
    }

    #[test]
    fn scaffold_typescript_generates_expected_files() {
        let s = scaffold::generate("typescript", "my-agent").unwrap();
        let names: Vec<&str> = s.files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"Agent_Manifest.toml"));
        assert!(names.contains(&"src/index.ts"));
    }

    #[test]
    fn scaffold_unsupported_language_returns_error() {
        let err = scaffold::generate("java", "my-agent").unwrap_err();
        assert!(err.contains("unsupported language"));
    }

    #[test]
    fn scaffold_manifest_contains_agent_name() {
        let s = scaffold::generate("rust", "cool-agent").unwrap();
        let manifest = s
            .files
            .iter()
            .find(|(n, _)| n == "Agent_Manifest.toml")
            .unwrap();
        assert!(manifest.1.contains("cool-agent"));
    }
}
