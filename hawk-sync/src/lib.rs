// hawk-sync: cross-device sync engine with AES-256-GCM encryption

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Argon2, Params};
use rand::RngCore;
use std::collections::VecDeque;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SyncItem {
    Agent(String),
    MemoryNamespace(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConflictStrategy {
    LastWriterWins,
    Manual,
    Merge,
}

#[derive(Debug, Clone)]
pub enum PeerStatus {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct PeerDevice {
    pub device_id: String,
    pub last_sync: Option<String>,
    pub status: PeerStatus,
    pub pending_changes: usize,
}

#[derive(Debug, Default)]
pub struct SyncResult {
    pub synced_items: usize,
    pub conflicts_resolved: usize,
}

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("sync not enabled — call enable() first")]
    NotEnabled,
    #[error("key derivation failed: {0}")]
    KeyDerivation(String),
    #[error("encryption failed")]
    Encryption,
    #[error("decryption failed")]
    Decryption,
    #[error("ciphertext too short")]
    CiphertextTooShort,
}

#[derive(Debug)]
pub struct ConflictEntry {
    pub key: String,
    pub discarded_value: Vec<u8>,
    pub winner_ts: u64,
}

pub struct SyncEngine {
    key: Option<[u8; 32]>,
    pub selected: Vec<SyncItem>,
    conflict_strategy: ConflictStrategy,
    queue: VecDeque<(String, Vec<u8>)>,
    conflict_log: Vec<ConflictEntry>,
}

impl SyncEngine {
    pub fn new() -> Self {
        Self {
            key: None,
            selected: Vec::new(),
            conflict_strategy: ConflictStrategy::LastWriterWins,
            queue: VecDeque::new(),
            conflict_log: Vec::new(),
        }
    }

    pub fn enable(&mut self, shared_secret: &str) -> Result<(), SyncError> {
        let salt = b"openhawk-sync-v1";
        let params = Params::new(65536, 3, 1, Some(32))
            .map_err(|e| SyncError::KeyDerivation(e.to_string()))?;
        let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut key = [0u8; 32];
        argon2
            .hash_password_into(shared_secret.as_bytes(), salt, &mut key)
            .map_err(|e| SyncError::KeyDerivation(e.to_string()))?;
        self.key = Some(key);
        Ok(())
    }

    /// Stub — real impl would use mDNS
    pub fn discover_peers(&self) -> Vec<PeerDevice> {
        Vec::new()
    }

    pub fn select_for_sync(&mut self, item: SyncItem) {
        if !self.selected.contains(&item) {
            self.selected.push(item);
        }
    }

    pub fn is_selected(&self, item: &SyncItem) -> bool {
        self.selected.contains(item)
    }

    pub fn sync(&self) -> Result<SyncResult, SyncError> {
        if self.key.is_none() {
            return Err(SyncError::NotEnabled);
        }
        Ok(SyncResult {
            synced_items: self.selected.len(),
            conflicts_resolved: 0,
        })
    }

    pub fn set_conflict_strategy(&mut self, strategy: ConflictStrategy) {
        self.conflict_strategy = strategy;
    }

    pub fn get_conflict_strategy(&self) -> &ConflictStrategy {
        &self.conflict_strategy
    }

    pub fn resolve_conflict(
        &mut self,
        key: &str,
        local_value: &[u8],
        local_ts: u64,
        remote_value: &[u8],
        remote_ts: u64,
    ) -> Vec<u8> {
        match self.conflict_strategy {
            ConflictStrategy::LastWriterWins => {
                if remote_ts > local_ts {
                    self.conflict_log.push(ConflictEntry {
                        key: key.to_string(),
                        discarded_value: local_value.to_vec(),
                        winner_ts: remote_ts,
                    });
                    remote_value.to_vec()
                } else {
                    self.conflict_log.push(ConflictEntry {
                        key: key.to_string(),
                        discarded_value: remote_value.to_vec(),
                        winner_ts: local_ts,
                    });
                    local_value.to_vec()
                }
            }
            ConflictStrategy::Manual => local_value.to_vec(),
            ConflictStrategy::Merge => {
                let mut merged = local_value.to_vec();
                merged.extend_from_slice(remote_value);
                merged
            }
        }
    }

    pub fn queue_change(&mut self, key: &str, value: &[u8]) {
        self.queue.push_back((key.to_string(), value.to_vec()));
    }

    pub fn get_queued_count(&self) -> usize {
        self.queue.len()
    }

    pub fn flush_queue(&mut self) -> Vec<(String, Vec<u8>)> {
        self.queue.drain(..).collect()
    }

    pub fn conflict_log(&self) -> &[ConflictEntry] {
        &self.conflict_log
    }

    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, SyncError> {
        let key_bytes = self.key.ok_or(SyncError::NotEnabled)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, data)
            .map_err(|_| SyncError::Encryption)?;
        let mut out = Vec::with_capacity(12 + ciphertext.len());
        out.extend_from_slice(&nonce_bytes);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, SyncError> {
        if data.len() < 12 {
            return Err(SyncError::CiphertextTooShort);
        }
        let key_bytes = self.key.ok_or(SyncError::NotEnabled)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
        let nonce = Nonce::from_slice(&data[..12]);
        cipher
            .decrypt(nonce, &data[12..])
            .map_err(|_| SyncError::Decryption)
    }
}

impl Default for SyncEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_engine() -> SyncEngine {
        let mut e = SyncEngine::new();
        e.enable("test-shared-secret").unwrap();
        e
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let engine = enabled_engine();
        let plaintext = b"hello, sync world";
        let ciphertext = engine.encrypt(plaintext).unwrap();
        let recovered = engine.decrypt(&ciphertext).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_call() {
        let engine = enabled_engine();
        let data = b"same data";
        let c1 = engine.encrypt(data).unwrap();
        let c2 = engine.encrypt(data).unwrap();
        assert_ne!(c1, c2);
    }

    #[test]
    fn decrypt_fails_without_enable() {
        let engine = SyncEngine::new();
        // Use 12+ bytes so we get NotEnabled, not CiphertextTooShort
        let data = vec![0u8; 20];
        assert!(matches!(engine.decrypt(&data), Err(SyncError::NotEnabled)));
    }

    #[test]
    fn decrypt_fails_on_short_input() {
        let engine = enabled_engine();
        assert!(matches!(
            engine.decrypt(b"short"),
            Err(SyncError::CiphertextTooShort)
        ));
    }

    #[test]
    fn decrypt_fails_on_tampered_ciphertext() {
        let engine = enabled_engine();
        let mut ct = engine.encrypt(b"data").unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0xff;
        assert!(matches!(engine.decrypt(&ct), Err(SyncError::Decryption)));
    }

    #[test]
    fn discover_peers_returns_empty_stub() {
        let engine = SyncEngine::new();
        assert!(engine.discover_peers().is_empty());
    }

    #[test]
    fn select_for_sync_marks_item() {
        let mut engine = SyncEngine::new();
        let item = SyncItem::Agent("my-agent".to_string());
        assert!(!engine.is_selected(&item));
        engine.select_for_sync(item.clone());
        assert!(engine.is_selected(&item));
    }

    #[test]
    fn select_for_sync_deduplicates() {
        let mut engine = SyncEngine::new();
        let item = SyncItem::MemoryNamespace("ns1".to_string());
        engine.select_for_sync(item.clone());
        engine.select_for_sync(item.clone());
        assert_eq!(engine.selected.len(), 1);
    }

    #[test]
    fn unselected_item_not_synced() {
        let mut engine = enabled_engine();
        engine.select_for_sync(SyncItem::Agent("agent-a".to_string()));
        let result = engine.sync().unwrap();
        assert_eq!(result.synced_items, 1);
        assert!(!engine.is_selected(&SyncItem::Agent("agent-b".to_string())));
    }

    #[test]
    fn sync_requires_enable() {
        let engine = SyncEngine::new();
        assert!(matches!(engine.sync(), Err(SyncError::NotEnabled)));
    }

    #[test]
    fn last_writer_wins_remote_newer() {
        let mut engine = enabled_engine();
        let winner = engine.resolve_conflict("key", b"local", 100, b"remote", 200);
        assert_eq!(winner, b"remote");
        assert_eq!(engine.conflict_log().len(), 1);
        assert_eq!(engine.conflict_log()[0].discarded_value, b"local");
    }

    #[test]
    fn last_writer_wins_local_newer() {
        let mut engine = enabled_engine();
        let winner = engine.resolve_conflict("key", b"local", 300, b"remote", 200);
        assert_eq!(winner, b"local");
        assert_eq!(engine.conflict_log()[0].discarded_value, b"remote");
    }

    #[test]
    fn manual_strategy_keeps_local() {
        let mut engine = enabled_engine();
        engine.set_conflict_strategy(ConflictStrategy::Manual);
        let winner = engine.resolve_conflict("key", b"local", 100, b"remote", 999);
        assert_eq!(winner, b"local");
    }

    #[test]
    fn merge_strategy_concatenates() {
        let mut engine = enabled_engine();
        engine.set_conflict_strategy(ConflictStrategy::Merge);
        let winner = engine.resolve_conflict("key", b"abc", 1, b"def", 2);
        assert_eq!(winner, b"abcdef");
    }

    #[test]
    fn queue_change_increments_count() {
        let mut engine = SyncEngine::new();
        assert_eq!(engine.get_queued_count(), 0);
        engine.queue_change("k1", b"v1");
        engine.queue_change("k2", b"v2");
        assert_eq!(engine.get_queued_count(), 2);
    }

    #[test]
    fn flush_queue_drains_all() {
        let mut engine = SyncEngine::new();
        engine.queue_change("k1", b"v1");
        engine.queue_change("k2", b"v2");
        let flushed = engine.flush_queue();
        assert_eq!(flushed.len(), 2);
        assert_eq!(engine.get_queued_count(), 0);
    }

    #[test]
    fn flush_queue_preserves_order() {
        let mut engine = SyncEngine::new();
        engine.queue_change("first", b"1");
        engine.queue_change("second", b"2");
        let flushed = engine.flush_queue();
        assert_eq!(flushed[0].0, "first");
        assert_eq!(flushed[1].0, "second");
    }

    #[test]
    fn conflict_strategy_default_is_lww() {
        let engine = SyncEngine::new();
        assert_eq!(
            *engine.get_conflict_strategy(),
            ConflictStrategy::LastWriterWins
        );
    }

    #[test]
    fn set_conflict_strategy_persists() {
        let mut engine = SyncEngine::new();
        engine.set_conflict_strategy(ConflictStrategy::Manual);
        assert_eq!(*engine.get_conflict_strategy(), ConflictStrategy::Manual);
    }

    #[test]
    fn enable_with_different_secrets_produces_different_keys() {
        let mut e1 = SyncEngine::new();
        let mut e2 = SyncEngine::new();
        e1.enable("secret-one").unwrap();
        e2.enable("secret-two").unwrap();
        let ct = e1.encrypt(b"data").unwrap();
        assert!(e2.decrypt(&ct).is_err());
    }
}
