// hawk-vault: local encrypted secrets storage (AES-256-GCM)

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use thiserror::Error;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("authentication failed")]
    AuthFailed,
    #[error("key not found: {0}")]
    NotFound(String),
    #[error("encryption error: {0}")]
    Encryption(String),
    #[error("decryption error: {0}")]
    Decryption(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("keychain error: {0}")]
    Keychain(String),
    #[error("key derivation error: {0}")]
    KeyDerivation(String),
}

pub type Result<T> = std::result::Result<T, VaultError>;

// ── Auth types ────────────────────────────────────────────────────────────────

pub enum AuthCredential {
    Passphrase(String),
    SystemKeychain,
}

#[derive(Clone)]
pub struct AuthToken {
    key: [u8; 32],
}

// ── On-disk format ────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct VaultFile {
    entries: HashMap<String, VaultEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct VaultEntry {
    nonce: String,      // hex-encoded 12-byte nonce
    ciphertext: String, // hex-encoded ciphertext
}

// ── SecretsVault trait ────────────────────────────────────────────────────────

pub trait SecretsVault {
    fn authenticate(&mut self, credential: AuthCredential) -> Result<AuthToken>;
    fn set(&mut self, key: &str, value: &[u8], auth: &AuthToken) -> Result<()>;
    fn get(&self, key: &str, auth: &AuthToken) -> Result<Vec<u8>>;
    fn delete(&mut self, key: &str, auth: &AuthToken) -> Result<()>;
    fn list_keys(&self) -> Vec<String>;
}

// ── Implementation ────────────────────────────────────────────────────────────

pub struct Vault {
    pub vault_path: PathBuf,
    auth_key: Option<[u8; 32]>,
}

impl Vault {
    pub fn new(vault_path: impl Into<PathBuf>) -> Self {
        Self { vault_path: vault_path.into(), auth_key: None }
    }

    pub fn default_path() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".hawk")
            .join("vault.enc")
    }

    pub(crate) fn load_file(&self) -> Result<VaultFile> {
        if !self.vault_path.exists() {
            return Ok(VaultFile::default());
        }
        let data = std::fs::read_to_string(&self.vault_path)?;
        Ok(serde_json::from_str(&data)?)
    }

    fn save_file(&self, vf: &VaultFile) -> Result<()> {
        if let Some(parent) = self.vault_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(vf)?;
        std::fs::write(&self.vault_path, data)?;
        Ok(())
    }

    fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| VaultError::Encryption(e.to_string()))?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| VaultError::Encryption(e.to_string()))?;
        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    fn decrypt(key: &[u8; 32], nonce_bytes: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| VaultError::Decryption(e.to_string()))?;
        let nonce = Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| VaultError::Decryption(e.to_string()))
    }

    fn derive_key(passphrase: &str, salt: &str) -> Result<[u8; 32]> {
        let params = Params::new(65536, 3, 1, Some(32))
            .map_err(|e| VaultError::KeyDerivation(e.to_string()))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        // Pad salt to at least 8 bytes (argon2 minimum)
        let mut salt_bytes = [0u8; 16];
        let src = salt.as_bytes();
        let len = src.len().min(16);
        salt_bytes[..len].copy_from_slice(&src[..len]);
        if len < 8 {
            for i in len..8 {
                salt_bytes[i] = 0x5a;
            }
        }

        let mut key = [0u8; 32];
        argon2
            .hash_password_into(passphrase.as_bytes(), &salt_bytes[..16], &mut key)
            .map_err(|e| VaultError::KeyDerivation(e.to_string()))?;
        Ok(key)
    }
}

impl SecretsVault for Vault {
    fn authenticate(&mut self, credential: AuthCredential) -> Result<AuthToken> {
        let key = match credential {
            AuthCredential::Passphrase(ref pass) => {
                Vault::derive_key(pass, pass)?
            }
            AuthCredential::SystemKeychain => get_or_create_keychain_key()?,
        };
        self.auth_key = Some(key);
        Ok(AuthToken { key })
    }

    fn set(&mut self, key: &str, value: &[u8], auth: &AuthToken) -> Result<()> {
        let (nonce, ciphertext) = Self::encrypt(&auth.key, value)?;
        let entry = VaultEntry {
            nonce: hex::encode(&nonce),
            ciphertext: hex::encode(&ciphertext),
        };
        let mut vf = self.load_file()?;
        vf.entries.insert(key.to_string(), entry);
        self.save_file(&vf)
    }

    fn get(&self, key: &str, auth: &AuthToken) -> Result<Vec<u8>> {
        let vf = self.load_file()?;
        let entry = vf
            .entries
            .get(key)
            .ok_or_else(|| VaultError::NotFound(key.to_string()))?;
        let nonce = hex::decode(&entry.nonce)
            .map_err(|e| VaultError::Decryption(e.to_string()))?;
        let ciphertext = hex::decode(&entry.ciphertext)
            .map_err(|e| VaultError::Decryption(e.to_string()))?;
        Self::decrypt(&auth.key, &nonce, &ciphertext)
    }

    fn delete(&mut self, key: &str, _auth: &AuthToken) -> Result<()> {
        let mut vf = self.load_file()?;
        if vf.entries.remove(key).is_none() {
            return Err(VaultError::NotFound(key.to_string()));
        }
        self.save_file(&vf)
    }

    fn list_keys(&self) -> Vec<String> {
        self.load_file()
            .map(|vf| vf.entries.keys().cloned().collect())
            .unwrap_or_default()
    }
}

// ── Keychain helper ───────────────────────────────────────────────────────────

fn get_or_create_keychain_key() -> Result<[u8; 32]> {
    const SERVICE: &str = "hawk-vault";
    const ACCOUNT: &str = "hawk-vault-master";

    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| VaultError::Keychain(e.to_string()))?;

    match entry.get_password() {
        Ok(hex_key) => {
            let bytes = hex::decode(&hex_key)
                .map_err(|e| VaultError::Keychain(e.to_string()))?;
            if bytes.len() != 32 {
                return Err(VaultError::Keychain("invalid key length in keychain".into()));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        Err(_) => {
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            entry
                .set_password(&hex::encode(&key))
                .map_err(|e| VaultError::Keychain(e.to_string()))?;
            Ok(key)
        }
    }
}

// ── CLI integration functions ─────────────────────────────────────────────────

pub fn vault_set(vault: &mut Vault, key: &str, value: &[u8], auth: &AuthToken) -> Result<()> {
    vault.set(key, value, auth)
}

/// Returns raw bytes for environment injection only — never log or print.
pub fn vault_get(vault: &Vault, key: &str, auth: &AuthToken) -> Result<Vec<u8>> {
    vault.get(key, auth)
}

pub fn vault_delete(vault: &mut Vault, key: &str, auth: &AuthToken) -> Result<()> {
    vault.delete(key, auth)
}

/// Returns key names only, never values.
pub fn vault_list(vault: &Vault) -> Vec<String> {
    vault.list_keys()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn temp_vault() -> (Vault, PathBuf) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        drop(tmp);
        let vault = Vault::new(&path);
        (vault, path)
    }

    fn auth_passphrase(vault: &mut Vault, pass: &str) -> AuthToken {
        vault
            .authenticate(AuthCredential::Passphrase(pass.to_string()))
            .expect("authentication should succeed")
    }

    #[test]
    fn test_round_trip() {
        let (mut vault, _path) = temp_vault();
        let token = auth_passphrase(&mut vault, "correct-horse-battery-staple");
        let plaintext = b"super-secret-api-key-12345";
        vault.set("MY_KEY", plaintext, &token).unwrap();
        let recovered = vault.get("MY_KEY", &token).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn test_wrong_key_cannot_decrypt() {
        let (mut vault, _path) = temp_vault();
        let token_a = auth_passphrase(&mut vault, "passphrase-a");
        vault.set("SECRET", b"value", &token_a).unwrap();

        let mut vault2 = Vault::new(vault.vault_path.clone());
        let token_b = auth_passphrase(&mut vault2, "passphrase-b");
        assert!(vault2.get("SECRET", &token_b).is_err());
    }

    #[test]
    fn test_list_keys_no_values() {
        let (mut vault, _path) = temp_vault();
        let token = auth_passphrase(&mut vault, "pass");
        vault.set("KEY_A", b"value_a", &token).unwrap();
        vault.set("KEY_B", b"value_b", &token).unwrap();

        let keys = vault.list_keys();
        assert!(keys.contains(&"KEY_A".to_string()));
        assert!(keys.contains(&"KEY_B".to_string()));
        assert_eq!(keys.len(), 2);

        let listed = vault_list(&vault);
        assert!(listed.contains(&"KEY_A".to_string()));
        assert!(listed.contains(&"KEY_B".to_string()));
    }

    #[test]
    fn test_delete_removes_key() {
        let (mut vault, _path) = temp_vault();
        let token = auth_passphrase(&mut vault, "pass");
        vault.set("TO_DELETE", b"gone", &token).unwrap();
        assert!(vault.list_keys().contains(&"TO_DELETE".to_string()));

        vault.delete("TO_DELETE", &token).unwrap();
        assert!(!vault.list_keys().contains(&"TO_DELETE".to_string()));
        assert!(matches!(vault.get("TO_DELETE", &token), Err(VaultError::NotFound(_))));
    }

    #[test]
    fn test_invalid_passphrase_cannot_decrypt() {
        let (mut vault, path) = temp_vault();
        let good_token = auth_passphrase(&mut vault, "correct-pass");
        vault.set("SECRET", b"my-secret", &good_token).unwrap();

        let mut vault2 = Vault::new(&path);
        let bad_token = auth_passphrase(&mut vault2, "wrong-pass");
        assert!(vault2.get("SECRET", &bad_token).is_err());
    }

    #[test]
    fn test_delete_nonexistent_key() {
        let (mut vault, _path) = temp_vault();
        let token = auth_passphrase(&mut vault, "pass");
        assert!(matches!(vault.delete("DOES_NOT_EXIST", &token), Err(VaultError::NotFound(_))));
    }

    #[test]
    fn test_cli_helpers() {
        let (mut vault, _path) = temp_vault();
        let token = auth_passphrase(&mut vault, "cli-pass");

        vault_set(&mut vault, "CLI_KEY", b"cli-value", &token).unwrap();
        assert!(vault_list(&vault).contains(&"CLI_KEY".to_string()));

        let val = vault_get(&vault, "CLI_KEY", &token).unwrap();
        assert_eq!(val, b"cli-value");

        vault_delete(&mut vault, "CLI_KEY", &token).unwrap();
        assert!(!vault_list(&vault).contains(&"CLI_KEY".to_string()));
    }

    #[test]
    fn test_nonce_uniqueness() {
        let (mut vault, _path) = temp_vault();
        let token = auth_passphrase(&mut vault, "pass");
        vault.set("K1", b"same-value", &token).unwrap();
        vault.set("K2", b"same-value", &token).unwrap();

        let vf = vault.load_file().unwrap();
        let e1 = &vf.entries["K1"];
        let e2 = &vf.entries["K2"];
        assert_ne!(e1.nonce, e2.nonce);
        assert_ne!(e1.ciphertext, e2.ciphertext);
    }
}
