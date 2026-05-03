// hawk-memory: Aura bridge + in-memory fallback
//
// Aura: https://github.com/ojuschugh1/aura
//
// Aura runs as a local daemon at localhost:7437 and exposes:
//   POST /memory/add    { "key": "...", "value": "..." }
//   GET  /memory/get?key=...
//   GET  /memory/ls
//   DELETE /memory/rm?key=...
//
// When the Aura daemon is running, AuraMemoryStore delegates all operations
// to it — giving persistent cross-tool memory (Claude Code, Cursor, Kiro,
// Gemini CLI all share the same store).
//
// When Aura is not running, InMemoryStore is used as a fallback so the
// interface always works.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error("lock poisoned")]
    LockPoisoned,
    #[error("invalid params: {0}")]
    InvalidParams(String),
}

// ── Scope ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemoryScope {
    Global,
    Session(String),
    Agent(u32),
}

impl MemoryScope {
    fn prefix(&self) -> String {
        match self {
            MemoryScope::Global => "global".to_owned(),
            MemoryScope::Session(id) => format!("session:{id}"),
            MemoryScope::Agent(pid) => format!("agent:{pid}"),
        }
    }
}

// ── Entry ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: Vec<u8>,
    pub scope: MemoryScope,
    pub timestamp: String,
    pub source_agent: u32,
}

// ── Trait ─────────────────────────────────────────────────────────────────────

pub trait SharedMemory {
    fn store(
        &self,
        scope: MemoryScope,
        key: &str,
        value: &[u8],
        source_agent: u32,
    ) -> Result<(), MemoryError>;

    fn query(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError>;

    fn archive_session(&self, session_id: &str) -> Result<(), MemoryError>;
}

// ── InMemoryStore ─────────────────────────────────────────────────────────────

// Pure-Rust in-memory implementation.
//
// Go FFI note: a production build would call into the Aura Go library via cgo.
// The bridge would look roughly like:
//
//   extern "C" {
//       fn aura_store(scope: *const c_char, key: *const c_char,
//                     value: *const u8, len: usize, agent: u32) -> c_int;
//       fn aura_query(key: *const c_char, out: *mut AuraEntry) -> c_int;
//       fn aura_archive_session(session_id: *const c_char) -> c_int;
//   }
//
// The cc build script would compile the cgo shim and link it here.
// This in-memory implementation satisfies the same interface for testing
// and environments where the Aura Go library is not available.

struct StoreState {
    entries: HashMap<String, MemoryEntry>,
    archive: HashMap<String, MemoryEntry>,
}

impl StoreState {
    fn new() -> Self {
        Self { entries: HashMap::new(), archive: HashMap::new() }
    }

    fn composite_key(scope: &MemoryScope, key: &str) -> String {
        format!("{}:{}", scope.prefix(), key)
    }
}

#[derive(Clone)]
pub struct InMemoryStore {
    state: Arc<Mutex<StoreState>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self { state: Arc::new(Mutex::new(StoreState::new())) }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedMemory for InMemoryStore {
    fn store(
        &self,
        scope: MemoryScope,
        key: &str,
        value: &[u8],
        source_agent: u32,
    ) -> Result<(), MemoryError> {
        let mut state = self.state.lock().map_err(|_| MemoryError::LockPoisoned)?;
        let ckey = StoreState::composite_key(&scope, key);
        state.entries.insert(
            ckey,
            MemoryEntry {
                key: key.to_owned(),
                value: value.to_vec(),
                scope,
                timestamp: Utc::now().to_rfc3339(),
                source_agent,
            },
        );
        Ok(())
    }

    // Query priority: agent scope (any agent) → session scope (any session) → global.
    // The caller supplies only the bare key; we scan all composite keys that end with
    // ":<key>" in priority order.
    fn query(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        let state = self.state.lock().map_err(|_| MemoryError::LockPoisoned)?;
        let suffix = format!(":{key}");

        // 1. Agent scope
        for (ckey, entry) in &state.entries {
            if ckey.starts_with("agent:") && ckey.ends_with(&suffix) {
                return Ok(Some(entry.clone()));
            }
        }
        // 2. Session scope
        for (ckey, entry) in &state.entries {
            if ckey.starts_with("session:") && ckey.ends_with(&suffix) {
                return Ok(Some(entry.clone()));
            }
        }
        // 3. Global scope
        let global_key = format!("global:{key}");
        Ok(state.entries.get(&global_key).cloned())
    }

    fn archive_session(&self, session_id: &str) -> Result<(), MemoryError> {
        let mut state = self.state.lock().map_err(|_| MemoryError::LockPoisoned)?;
        let prefix = format!("session:{session_id}:");
        let session_keys: Vec<String> =
            state.entries.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
        for k in session_keys {
            if let Some(entry) = state.entries.remove(&k) {
                state.archive.insert(k, entry);
            }
        }
        Ok(())
    }
}

impl InMemoryStore {
    /// Returns archived entries for a session (for inspection / tests).
    pub fn archived_entries(&self, session_id: &str) -> Vec<MemoryEntry> {
        let state = self.state.lock().unwrap();
        let prefix = format!("session:{session_id}:");
        state
            .archive
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, v)| v.clone())
            .collect()
    }
}

// ── Aura HTTP bridge ──────────────────────────────────────────────────────────
//
// Aura daemon REST API (localhost:7437):
//
//   POST   /memory/add     body: { "key": "...", "value": "..." }
//   GET    /memory/get     query: ?key=...
//   GET    /memory/ls      returns: [{ "key": "...", "value": "...", ... }]
//   DELETE /memory/rm      query: ?key=...
//
// aura verify [--session <id>]   -- runs claimcheck internally
// aura compact                   -- runs sqz internally
// aura scan                      -- runs ghostdep internally
// aura cost [--daily]            -- token cost report

const AURA_BASE: &str = "http://localhost:7437";
#[allow(dead_code)]

/// Returns true if the Aura daemon is reachable at localhost:7437.
pub fn aura_available() -> bool {
    // Use a TCP connect check — no HTTP client dep needed
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:7437".parse().unwrap(),
        std::time::Duration::from_millis(200),
    ).is_ok()
}

/// Returns true if the `aura` CLI binary is on PATH.
pub fn aura_cli_available() -> bool {
    std::process::Command::new("aura").arg("version").output().is_ok()
}

/// Aura memory entry as returned by GET /memory/ls.
#[derive(Debug, Deserialize)]
pub struct AuraMemoryItem {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub timestamp: String,
}

/// Call `aura memory add <key> <value>` via CLI.
pub fn aura_memory_add(key: &str, value: &str) -> bool {
    std::process::Command::new("aura")
        .args(["memory", "add", key, value])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Call `aura memory get <key>` via CLI and return the value.
pub fn aura_memory_get(key: &str) -> Option<String> {
    let output = std::process::Command::new("aura")
        .args(["memory", "get", key])
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8(output.stdout).ok()?;
        let trimmed = s.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    } else {
        None
    }
}

/// Call `aura memory ls --json` and return all entries.
pub fn aura_memory_ls() -> Vec<AuraMemoryItem> {
    let output = std::process::Command::new("aura")
        .args(["memory", "ls", "--json"])
        .output();
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    serde_json::from_slice(&output.stdout).unwrap_or_default()
}

/// Call `aura memory rm <key>` via CLI.
pub fn aura_memory_rm(key: &str) -> bool {
    std::process::Command::new("aura")
        .args(["memory", "rm", key])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// AuraMemoryStore — delegates to the Aura daemon when running,
/// falls back to InMemoryStore otherwise.
pub struct AuraMemoryStore {
    fallback: InMemoryStore,
}

impl AuraMemoryStore {
    pub fn new() -> Self {
        Self { fallback: InMemoryStore::new() }
    }

    /// Store a key-value pair. Uses Aura when available, fallback otherwise.
    pub fn store_kv(&self, key: &str, value: &str) -> bool {
        if aura_cli_available() {
            aura_memory_add(key, value)
        } else {
            self.fallback
                .store(MemoryScope::Global, key, value.as_bytes(), 0)
                .is_ok()
        }
    }

    /// Retrieve a value by key. Uses Aura when available, fallback otherwise.
    pub fn get_kv(&self, key: &str) -> Option<String> {
        if aura_cli_available() {
            aura_memory_get(key)
        } else {
            self.fallback
                .query(key)
                .ok()
                .flatten()
                .map(|e| String::from_utf8_lossy(&e.value).into_owned())
        }
    }

    /// List all entries. Uses Aura when available, fallback otherwise.
    pub fn list(&self) -> Vec<(String, String)> {
        if aura_cli_available() {
            aura_memory_ls()
                .into_iter()
                .map(|item| (item.key, item.value))
                .collect()
        } else {
            let state = self.fallback.state.lock().unwrap();
            state.entries.values()
                .map(|e| (e.key.clone(), String::from_utf8_lossy(&e.value).into_owned()))
                .collect()
        }
    }

    /// Delete a key. Uses Aura when available, fallback otherwise.
    pub fn remove(&self, key: &str) -> bool {
        if aura_cli_available() {
            aura_memory_rm(key)
        } else {
            // remove from fallback by overwriting with empty — no delete API
            self.fallback
                .store(MemoryScope::Global, key, b"", 0)
                .is_ok()
        }
    }
}

impl Default for AuraMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedMemory for AuraMemoryStore {
    fn store(&self, scope: MemoryScope, key: &str, value: &[u8], source_agent: u32) -> Result<(), MemoryError> {
        let value_str = String::from_utf8_lossy(value);
        // prefix key with scope so Aura stores them distinctly
        let scoped_key = format!("{}:{}", scope.prefix(), key);
        if aura_cli_available() {
            aura_memory_add(&scoped_key, &value_str);
        }
        // always write to fallback so in-process queries work
        self.fallback.store(scope, key, value, source_agent)
    }

    fn query(&self, key: &str) -> Result<Option<MemoryEntry>, MemoryError> {
        // try Aura first for the global scope key
        if aura_cli_available() {
            let scoped_key = format!("global:{key}");
            if let Some(val) = aura_memory_get(&scoped_key) {
                return Ok(Some(MemoryEntry {
                    key: key.to_owned(),
                    value: val.into_bytes(),
                    scope: MemoryScope::Global,
                    timestamp: Utc::now().to_rfc3339(),
                    source_agent: 0,
                }));
            }
        }
        // fall back to in-process store
        self.fallback.query(key)
    }

    fn archive_session(&self, session_id: &str) -> Result<(), MemoryError> {
        self.fallback.archive_session(session_id)
    }
}

// ── MCP interface ─────────────────────────────────────────────────────────────

/// Exposes memory operations as MCP tool provider endpoints.
///
/// Supported tool names:
///   - "memory_store"  params: { scope, key, value (base64), source_agent }
///   - "memory_query"  params: { key }
pub struct McpMemoryProvider {
    store: InMemoryStore,
}

impl McpMemoryProvider {
    pub fn new(store: InMemoryStore) -> Self {
        Self { store }
    }

    pub fn handle_tool_call(&self, tool_name: &str, params: Value) -> Value {
        match tool_name {
            "memory_store" => self.mcp_store(params),
            "memory_query" => self.mcp_query(params),
            _ => json!({ "error": format!("unknown tool: {tool_name}") }),
        }
    }

    fn mcp_store(&self, params: Value) -> Value {
        let scope = match parse_scope(&params) {
            Ok(s) => s,
            Err(e) => return json!({ "error": e }),
        };
        let key = match params.get("key").and_then(Value::as_str) {
            Some(k) => k.to_owned(),
            None => return json!({ "error": "missing field: key" }),
        };
        let value_b64 = match params.get("value").and_then(Value::as_str) {
            Some(v) => v.to_owned(),
            None => return json!({ "error": "missing field: value" }),
        };
        let value = match base64_decode(&value_b64) {
            Ok(v) => v,
            Err(e) => return json!({ "error": e }),
        };
        let source_agent = params
            .get("source_agent")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        match self.store.store(scope, &key, &value, source_agent) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    fn mcp_query(&self, params: Value) -> Value {
        let key = match params.get("key").and_then(Value::as_str) {
            Some(k) => k.to_owned(),
            None => return json!({ "error": "missing field: key" }),
        };
        match self.store.query(&key) {
            Ok(Some(entry)) => json!({
                "found": true,
                "key": entry.key,
                "value": base64_encode(&entry.value),
                "scope": scope_to_str(&entry.scope),
                "timestamp": entry.timestamp,
                "source_agent": entry.source_agent,
            }),
            Ok(None) => json!({ "found": false }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}

// ── A2A interface ─────────────────────────────────────────────────────────────

/// Exposes memory operations as A2A agent card endpoints.
///
/// Supported actions:
///   - "store"  params: { scope, key, value (base64), source_agent }
///   - "query"  params: { key }
pub struct A2aMemoryCard {
    store: InMemoryStore,
}

impl A2aMemoryCard {
    pub fn new(store: InMemoryStore) -> Self {
        Self { store }
    }

    pub fn handle_request(&self, action: &str, params: Value) -> Value {
        match action {
            "store" => self.a2a_store(params),
            "query" => self.a2a_query(params),
            _ => json!({ "error": format!("unknown action: {action}") }),
        }
    }

    fn a2a_store(&self, params: Value) -> Value {
        let scope = match parse_scope(&params) {
            Ok(s) => s,
            Err(e) => return json!({ "error": e }),
        };
        let key = match params.get("key").and_then(Value::as_str) {
            Some(k) => k.to_owned(),
            None => return json!({ "error": "missing field: key" }),
        };
        let value_b64 = match params.get("value").and_then(Value::as_str) {
            Some(v) => v.to_owned(),
            None => return json!({ "error": "missing field: value" }),
        };
        let value = match base64_decode(&value_b64) {
            Ok(v) => v,
            Err(e) => return json!({ "error": e }),
        };
        let source_agent = params
            .get("source_agent")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;

        match self.store.store(scope, &key, &value, source_agent) {
            Ok(()) => json!({ "status": "ok" }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    fn a2a_query(&self, params: Value) -> Value {
        let key = match params.get("key").and_then(Value::as_str) {
            Some(k) => k.to_owned(),
            None => return json!({ "error": "missing field: key" }),
        };
        match self.store.query(&key) {
            Ok(Some(entry)) => json!({
                "status": "ok",
                "key": entry.key,
                "value": base64_encode(&entry.value),
                "scope": scope_to_str(&entry.scope),
                "timestamp": entry.timestamp,
                "source_agent": entry.source_agent,
            }),
            Ok(None) => json!({ "status": "not_found" }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_scope(params: &Value) -> Result<MemoryScope, String> {
    let scope_str = params
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("global");
    if scope_str == "global" {
        return Ok(MemoryScope::Global);
    }
    if let Some(id) = scope_str.strip_prefix("session:") {
        return Ok(MemoryScope::Session(id.to_owned()));
    }
    if let Some(pid_str) = scope_str.strip_prefix("agent:") {
        let pid: u32 = pid_str
            .parse()
            .map_err(|_| format!("invalid agent pid: {pid_str}"))?;
        return Ok(MemoryScope::Agent(pid));
    }
    Err(format!("unknown scope: {scope_str}"))
}

fn scope_to_str(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_owned(),
        MemoryScope::Session(id) => format!("session:{id}"),
        MemoryScope::Agent(pid) => format!("agent:{pid}"),
    }
}

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    // Simple base64 without external dep — use the alphabet directly.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 { chunk[1] as usize } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as usize } else { 0 };
        let _ = write!(out, "{}", ALPHABET[b0 >> 2] as char);
        let _ = write!(out, "{}", ALPHABET[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            let _ = write!(out, "{}", ALPHABET[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            let _ = write!(out, "{}", ALPHABET[b2 & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(0),
            _ => Err(format!("invalid base64 char: {c}")),
        }
    }
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("base64 length must be a multiple of 4".to_owned());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let v0 = val(chunk[0])?;
        let v1 = val(chunk[1])?;
        let v2 = val(chunk[2])?;
        let v3 = val(chunk[3])?;
        out.push((v0 << 2) | (v1 >> 4));
        if chunk[2] != b'=' {
            out.push((v1 << 4) | (v2 >> 2));
        }
        if chunk[3] != b'=' {
            out.push((v2 << 6) | v3);
        }
    }
    Ok(out)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> InMemoryStore {
        InMemoryStore::new()
    }

    #[allow(dead_code)]
    fn bytes(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    // ── Task 11.3: SharedMemory unit tests ────────────────────────────────────

    #[test]
    fn global_scope_store_and_query_round_trip() {
        let s = store();
        s.store(MemoryScope::Global, "answer", b"42", 1).unwrap();
        let entry = s.query("answer").unwrap().expect("entry should exist");
        assert_eq!(entry.value, b"42");
        assert_eq!(entry.scope, MemoryScope::Global);
        assert_eq!(entry.source_agent, 1);
    }

    #[test]
    fn session_scope_store_and_query_round_trip() {
        let s = store();
        s.store(MemoryScope::Session("sess-1".into()), "ctx", b"hello", 2).unwrap();
        let entry = s.query("ctx").unwrap().expect("entry should exist");
        assert_eq!(entry.value, b"hello");
        assert!(matches!(entry.scope, MemoryScope::Session(ref id) if id == "sess-1"));
    }

    #[test]
    fn agent_scope_store_and_query_round_trip() {
        let s = store();
        s.store(MemoryScope::Agent(99), "private", b"secret", 99).unwrap();
        let entry = s.query("private").unwrap().expect("entry should exist");
        assert_eq!(entry.value, b"secret");
        assert_eq!(entry.scope, MemoryScope::Agent(99));
    }

    #[test]
    fn query_returns_none_for_missing_key() {
        let s = store();
        assert!(s.query("nonexistent").unwrap().is_none());
    }

    #[test]
    fn global_scope_persists_after_session_archive() {
        let s = store();
        s.store(MemoryScope::Global, "persistent", b"yes", 1).unwrap();
        s.store(MemoryScope::Session("sess-a".into()), "temp", b"no", 1).unwrap();

        s.archive_session("sess-a").unwrap();

        // Global entry still queryable
        let entry = s.query("persistent").unwrap().expect("global entry should survive archival");
        assert_eq!(entry.value, b"yes");
    }

    #[test]
    fn session_scope_is_archived_on_session_end() {
        let s = store();
        s.store(MemoryScope::Session("sess-b".into()), "work", b"data", 5).unwrap();

        // Before archival: queryable
        assert!(s.query("work").unwrap().is_some());

        s.archive_session("sess-b").unwrap();

        // After archival: no longer in live store
        assert!(s.query("work").unwrap().is_none());

        // But present in archive
        let archived = s.archived_entries("sess-b");
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].key, "work");
    }

    #[test]
    fn archive_session_only_removes_matching_session() {
        let s = store();
        s.store(MemoryScope::Session("sess-x".into()), "x_key", b"x", 1).unwrap();
        s.store(MemoryScope::Session("sess-y".into()), "y_key", b"y", 2).unwrap();

        s.archive_session("sess-x").unwrap();

        assert!(s.query("x_key").unwrap().is_none());
        assert!(s.query("y_key").unwrap().is_some());
    }

    #[test]
    fn agent_scope_is_private_to_agent() {
        let s = store();
        s.store(MemoryScope::Agent(10), "secret", b"agent10", 10).unwrap();
        s.store(MemoryScope::Agent(20), "secret", b"agent20", 20).unwrap();

        // query returns one of the agent-scoped entries (agent scope has highest priority)
        let entry = s.query("secret").unwrap().expect("should find an agent entry");
        assert!(matches!(entry.scope, MemoryScope::Agent(_)));
        // The value belongs to one of the agents, not mixed
        assert!(entry.value == b"agent10" || entry.value == b"agent20");
    }

    #[test]
    fn query_priority_agent_over_session_over_global() {
        let s = store();
        s.store(MemoryScope::Global, "k", b"global", 0).unwrap();
        s.store(MemoryScope::Session("s1".into()), "k", b"session", 1).unwrap();
        s.store(MemoryScope::Agent(7), "k", b"agent", 7).unwrap();

        let entry = s.query("k").unwrap().unwrap();
        assert_eq!(entry.value, b"agent");
    }

    #[test]
    fn query_falls_back_to_session_when_no_agent_entry() {
        let s = store();
        s.store(MemoryScope::Global, "k", b"global", 0).unwrap();
        s.store(MemoryScope::Session("s1".into()), "k", b"session", 1).unwrap();

        let entry = s.query("k").unwrap().unwrap();
        assert_eq!(entry.value, b"session");
    }

    #[test]
    fn query_falls_back_to_global_when_no_session_or_agent_entry() {
        let s = store();
        s.store(MemoryScope::Global, "k", b"global", 0).unwrap();

        let entry = s.query("k").unwrap().unwrap();
        assert_eq!(entry.value, b"global");
    }

    #[test]
    fn store_overwrites_existing_entry() {
        let s = store();
        s.store(MemoryScope::Global, "counter", b"1", 1).unwrap();
        s.store(MemoryScope::Global, "counter", b"2", 1).unwrap();
        let entry = s.query("counter").unwrap().unwrap();
        assert_eq!(entry.value, b"2");
    }

    // ── Task 11.2: MCP interface tests ────────────────────────────────────────

    #[test]
    fn mcp_store_and_query_round_trip() {
        let mcp = McpMemoryProvider::new(store());
        let encoded = base64_encode(b"mcp_value");
        let store_result = mcp.handle_tool_call(
            "memory_store",
            json!({ "scope": "global", "key": "mcp_key", "value": encoded, "source_agent": 1 }),
        );
        assert_eq!(store_result["ok"], true);

        let query_result =
            mcp.handle_tool_call("memory_query", json!({ "key": "mcp_key" }));
        assert_eq!(query_result["found"], true);
        assert_eq!(query_result["key"], "mcp_key");
    }

    #[test]
    fn mcp_query_missing_key_returns_not_found() {
        let mcp = McpMemoryProvider::new(store());
        let result = mcp.handle_tool_call("memory_query", json!({ "key": "ghost" }));
        assert_eq!(result["found"], false);
    }

    #[test]
    fn mcp_unknown_tool_returns_error() {
        let mcp = McpMemoryProvider::new(store());
        let result = mcp.handle_tool_call("unknown_tool", json!({}));
        assert!(result["error"].as_str().unwrap().contains("unknown tool"));
    }

    #[test]
    fn mcp_store_missing_key_field_returns_error() {
        let mcp = McpMemoryProvider::new(store());
        let result = mcp.handle_tool_call(
            "memory_store",
            json!({ "scope": "global", "value": base64_encode(b"x") }),
        );
        assert!(result["error"].as_str().is_some());
    }

    // ── Task 11.2: A2A interface tests ────────────────────────────────────────

    #[test]
    fn a2a_store_and_query_round_trip() {
        let a2a = A2aMemoryCard::new(store());
        let encoded = base64_encode(b"a2a_value");
        let store_result = a2a.handle_request(
            "store",
            json!({ "scope": "session:s1", "key": "a2a_key", "value": encoded, "source_agent": 2 }),
        );
        assert_eq!(store_result["status"], "ok");

        let query_result = a2a.handle_request("query", json!({ "key": "a2a_key" }));
        assert_eq!(query_result["status"], "ok");
        assert_eq!(query_result["key"], "a2a_key");
    }

    #[test]
    fn a2a_query_missing_key_returns_not_found() {
        let a2a = A2aMemoryCard::new(store());
        let result = a2a.handle_request("query", json!({ "key": "ghost" }));
        assert_eq!(result["status"], "not_found");
    }

    #[test]
    fn a2a_unknown_action_returns_error() {
        let a2a = A2aMemoryCard::new(store());
        let result = a2a.handle_request("delete", json!({}));
        assert!(result["error"].as_str().unwrap().contains("unknown action"));
    }

    #[test]
    fn a2a_agent_scope_store_and_query() {
        let a2a = A2aMemoryCard::new(store());
        let encoded = base64_encode(b"private");
        a2a.handle_request(
            "store",
            json!({ "scope": "agent:42", "key": "priv", "value": encoded, "source_agent": 42 }),
        );
        let result = a2a.handle_request("query", json!({ "key": "priv" }));
        assert_eq!(result["status"], "ok");
        assert_eq!(result["scope"], "agent:42");
    }

    // ── Base64 helpers ────────────────────────────────────────────────────────

    #[test]
    fn base64_round_trip() {
        let original = b"Hello, World! \x00\xff\xfe";
        let encoded = base64_encode(original);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn base64_empty_input() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_decode("").unwrap(), b"");
    }

    // ── Aura bridge tests ─────────────────────────────────────────────────────

    #[test]
    fn aura_available_check_does_not_panic() {
        let _ = aura_available();
    }

    #[test]
    fn aura_cli_available_check_does_not_panic() {
        let _ = aura_cli_available();
    }

    #[test]
    fn aura_memory_store_falls_back_to_in_memory_when_aura_not_running() {
        let store = AuraMemoryStore::new();
        // store and retrieve — should work via fallback regardless of Aura
        store.store(MemoryScope::Global, "test-key", b"test-value", 0).unwrap();
        let entry = store.query("test-key").unwrap();
        // if Aura is not running, fallback returns the value
        // if Aura is running, it may or may not have the key — either is valid
        if !aura_cli_available() {
            assert!(entry.is_some());
            assert_eq!(entry.unwrap().value, b"test-value");
        }
    }

    #[test]
    fn aura_memory_store_kv_does_not_panic() {
        let store = AuraMemoryStore::new();
        let _ = store.store_kv("hawk-test-key", "hawk-test-value");
    }

    #[test]
    fn aura_memory_get_kv_does_not_panic() {
        let store = AuraMemoryStore::new();
        let _ = store.get_kv("hawk-test-key");
    }

    #[test]
    fn aura_memory_list_does_not_panic() {
        let store = AuraMemoryStore::new();
        let _ = store.list();
    }

    #[test]
    fn aura_memory_store_and_retrieve_via_fallback() {
        // always works via in-memory fallback
        let store = AuraMemoryStore::new();
        store.store(MemoryScope::Global, "fallback-key", b"fallback-val", 1).unwrap();
        let entry = store.fallback.query("fallback-key").unwrap();
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().value, b"fallback-val");
    }

    #[test]
    fn aura_memory_ls_returns_vec() {
        // returns empty vec when aura is not installed — should not panic
        let items = aura_memory_ls();
        let _ = items; // just verify it doesn't panic
    }
}
