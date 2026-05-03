use std::path::Path;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NestError {
    #[error("package already installed: '{0}'")]
    AlreadyInstalled(String),
    #[error("invalid signature for '{0}'")]
    InvalidSignature(String),
    #[error("invalid package: {0}")]
    InvalidPackage(String),
    #[error("invalid semver version '{0}': expected MAJOR.MINOR.PATCH")]
    InvalidVersion(String),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PackageType {
    Flight,
    Talon,
    Driver,
}

impl PackageType {
    fn as_str(&self) -> &'static str {
        match self {
            PackageType::Flight => "Flight",
            PackageType::Talon => "Talon",
            PackageType::Driver => "Driver",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "Flight" => Some(PackageType::Flight),
            "Talon" => Some(PackageType::Talon),
            "Driver" => Some(PackageType::Driver),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageListing {
    pub name: String,
    pub description: String,
    pub author: String,
    pub version: String,
    pub download_count: u64,
    pub package_type: PackageType,
    pub compatibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub package_type: PackageType,
    pub signature: String,
    pub installed_at: String,
    pub capabilities: Vec<String>,
}

fn is_valid_semver(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 { return false; }
    parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn expected_signature(name: &str, version: &str) -> String {
    let mut h = Sha256::new();
    h.update(format!("{name}:{version}").as_bytes());
    hex::encode(h.finalize())
}

pub fn make_signature(name: &str, version: &str) -> String {
    expected_signature(name, version)
}

fn verify_signature(name: &str, version: &str, signature: &str) -> bool {
    expected_signature(name, version) == signature
}

fn init_schema(db: &Connection) -> Result<(), NestError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS installed_packages (
            name          TEXT PRIMARY KEY,
            version       TEXT NOT NULL,
            package_type  TEXT NOT NULL,
            signature     TEXT NOT NULL,
            installed_at  TEXT NOT NULL,
            capabilities  TEXT
        );
        CREATE TABLE IF NOT EXISTS package_index (
            name           TEXT PRIMARY KEY,
            description    TEXT NOT NULL,
            author         TEXT NOT NULL,
            version        TEXT NOT NULL,
            download_count INTEGER NOT NULL DEFAULT 0,
            package_type   TEXT NOT NULL,
            compatibility  TEXT NOT NULL
        );",
    )?;
    Ok(())
}

pub struct NestClient {
    db: Connection,
}

impl NestClient {
    pub fn new(db: Connection) -> Self {
        init_schema(&db).expect("failed to initialise hawk-nest schema");
        Self { db }
    }

    pub fn search(&self, query: &str) -> Result<Vec<PackageListing>, NestError> {
        let pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = self.db.prepare(
            "SELECT name, description, author, version, download_count, package_type, compatibility \
             FROM package_index WHERE lower(name) LIKE ?1 OR lower(description) LIKE ?1",
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;

        let mut results = Vec::new();
        for row in rows {
            let (name, description, author, version, download_count, pkg_type_str, compatibility) = row?;
            let package_type = PackageType::from_str(&pkg_type_str).unwrap_or(PackageType::Flight);
            results.push(PackageListing { name, description, author, version, download_count, package_type, compatibility });
        }
        Ok(results)
    }

    pub fn install(&self, package_name: &str, listing: &PackageListing, signature: &str) -> Result<(), NestError> {
        if !verify_signature(package_name, &listing.version, signature) {
            return Err(NestError::InvalidSignature(package_name.to_string()));
        }

        let already: bool = self.db.query_row(
            "SELECT COUNT(*) FROM installed_packages WHERE name = ?1",
            params![package_name],
            |row| row.get::<_, i64>(0),
        )? > 0;

        if already {
            return Err(NestError::AlreadyInstalled(package_name.to_string()));
        }

        let now = Utc::now().to_rfc3339();
        self.db.execute(
            "INSERT INTO installed_packages (name, version, package_type, signature, installed_at, capabilities) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![package_name, listing.version, listing.package_type.as_str(), signature, now, "[]"],
        )?;
        Ok(())
    }

    pub fn publish(&self, package_path: &Path) -> Result<(), NestError> {
        let manifest_path = package_path.join("Agent_Manifest.toml");
        if !manifest_path.exists() {
            return Err(NestError::InvalidPackage("Agent_Manifest.toml not found at package root".to_string()));
        }

        let manifest_src = std::fs::read_to_string(&manifest_path)?;
        let version = extract_version_from_manifest(&manifest_src).ok_or_else(|| {
            NestError::InvalidPackage("version field missing from Agent_Manifest.toml".to_string())
        })?;

        if !is_valid_semver(&version) {
            return Err(NestError::InvalidVersion(version));
        }

        Ok(())
    }

    pub fn list_installed(&self) -> Vec<InstalledPackage> {
        let mut stmt = match self.db.prepare(
            "SELECT name, version, package_type, signature, installed_at, capabilities FROM installed_packages",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        });

        let mut packages = Vec::new();
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (name, version, pkg_type_str, signature, installed_at, caps_json) = row;
                let package_type = PackageType::from_str(&pkg_type_str).unwrap_or(PackageType::Flight);
                let capabilities: Vec<String> = serde_json::from_str(&caps_json).unwrap_or_default();
                packages.push(InstalledPackage { name, version, package_type, signature, installed_at, capabilities });
            }
        }
        packages
    }

    pub fn add_to_index(&self, listing: PackageListing) -> Result<(), NestError> {
        self.db.execute(
            "INSERT OR REPLACE INTO package_index \
             (name, description, author, version, download_count, package_type, compatibility) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![listing.name, listing.description, listing.author, listing.version, listing.download_count, listing.package_type.as_str(), listing.compatibility],
        )?;
        Ok(())
    }

    pub fn search_index(&self, query: &str) -> Vec<PackageListing> {
        self.search(query).unwrap_or_default()
    }
}

fn extract_version_from_manifest(src: &str) -> Option<String> {
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("version") {
            if let Some(eq_pos) = trimmed.find('=') {
                let val = trimmed[eq_pos + 1..].trim().trim_matches('"').to_string();
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn in_memory_client() -> NestClient {
        NestClient::new(Connection::open_in_memory().unwrap())
    }

    fn sample_listing(name: &str) -> PackageListing {
        PackageListing {
            name: name.to_string(),
            description: format!("{name} description"),
            author: "test-author".to_string(),
            version: "1.0.0".to_string(),
            download_count: 42,
            package_type: PackageType::Flight,
            compatibility: "hawk>=0.1".to_string(),
        }
    }

    #[test]
    fn search_returns_matching_packages() {
        let client = in_memory_client();
        client.add_to_index(sample_listing("web-scraper")).unwrap();
        client.add_to_index(sample_listing("data-pipeline")).unwrap();
        let results = client.search("web").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "web-scraper");
    }

    #[test]
    fn search_is_case_insensitive() {
        let client = in_memory_client();
        client.add_to_index(sample_listing("WebScraper")).unwrap();
        let results = client.search("webscraper").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_matches_description() {
        let client = in_memory_client();
        let mut listing = sample_listing("my-pkg");
        listing.description = "A powerful research tool".to_string();
        client.add_to_index(listing).unwrap();
        let results = client.search("research").unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_returns_empty_when_no_match() {
        let client = in_memory_client();
        client.add_to_index(sample_listing("alpha")).unwrap();
        assert!(client.search("zzz-no-match").unwrap().is_empty());
    }

    #[test]
    fn install_valid_signature_registers_package() {
        let client = in_memory_client();
        let listing = sample_listing("my-flight");
        let sig = make_signature("my-flight", "1.0.0");
        client.install("my-flight", &listing, &sig).unwrap();
        let installed = client.list_installed();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].name, "my-flight");
    }

    #[test]
    fn install_invalid_signature_rejected() {
        let client = in_memory_client();
        let listing = sample_listing("my-flight");
        let err = client.install("my-flight", &listing, "bad-sig").unwrap_err();
        assert!(matches!(err, NestError::InvalidSignature(_)));
    }

    #[test]
    fn install_duplicate_rejected() {
        let client = in_memory_client();
        let listing = sample_listing("my-flight");
        let sig = make_signature("my-flight", "1.0.0");
        client.install("my-flight", &listing, &sig).unwrap();
        let err = client.install("my-flight", &listing, &sig).unwrap_err();
        assert!(matches!(err, NestError::AlreadyInstalled(_)));
    }

    #[test]
    fn install_wrong_version_signature_rejected() {
        let client = in_memory_client();
        let listing = sample_listing("my-flight");
        let sig = make_signature("my-flight", "2.0.0");
        let err = client.install("my-flight", &listing, &sig).unwrap_err();
        assert!(matches!(err, NestError::InvalidSignature(_)));
    }

    #[test]
    fn publish_valid_package_succeeds() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Agent_Manifest.toml"), "[agent]\nname = \"test\"\nversion = \"1.2.3\"\n").unwrap();
        let client = in_memory_client();
        client.publish(dir.path()).unwrap();
    }

    #[test]
    fn publish_missing_manifest_rejected() {
        let dir = TempDir::new().unwrap();
        let client = in_memory_client();
        let err = client.publish(dir.path()).unwrap_err();
        assert!(matches!(err, NestError::InvalidPackage(_)));
        assert!(err.to_string().contains("Agent_Manifest.toml"));
    }

    #[test]
    fn publish_invalid_semver_rejected() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Agent_Manifest.toml"), "[agent]\nname = \"test\"\nversion = \"1.0\"\n").unwrap();
        let client = in_memory_client();
        let err = client.publish(dir.path()).unwrap_err();
        assert!(matches!(err, NestError::InvalidVersion(_)));
    }

    #[test]
    fn publish_missing_version_field_rejected() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Agent_Manifest.toml"), "[agent]\nname = \"test\"\n").unwrap();
        let client = in_memory_client();
        let err = client.publish(dir.path()).unwrap_err();
        assert!(matches!(err, NestError::InvalidPackage(_)));
    }

    #[test]
    fn publish_non_numeric_semver_rejected() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Agent_Manifest.toml"), "[agent]\nname = \"test\"\nversion = \"1.0.0-beta\"\n").unwrap();
        let client = in_memory_client();
        let err = client.publish(dir.path()).unwrap_err();
        assert!(matches!(err, NestError::InvalidVersion(_)));
    }

    #[test]
    fn search_index_returns_cached_metadata() {
        let client = in_memory_client();
        client.add_to_index(sample_listing("cached-pkg")).unwrap();
        let results = client.search_index("cached");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "cached-pkg");
    }

    #[test]
    fn add_to_index_upserts_existing_entry() {
        let client = in_memory_client();
        client.add_to_index(sample_listing("pkg")).unwrap();
        let mut updated = sample_listing("pkg");
        updated.download_count = 999;
        client.add_to_index(updated).unwrap();
        let results = client.search_index("pkg");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].download_count, 999);
    }

    #[test]
    fn valid_semver_accepted() {
        assert!(is_valid_semver("0.0.0"));
        assert!(is_valid_semver("1.2.3"));
        assert!(is_valid_semver("10.20.30"));
    }

    #[test]
    fn invalid_semver_rejected() {
        assert!(!is_valid_semver("1.0"));
        assert!(!is_valid_semver("1.0.0.0"));
        assert!(!is_valid_semver("1.0.0-beta"));
        assert!(!is_valid_semver("v1.0.0"));
        assert!(!is_valid_semver(""));
        assert!(!is_valid_semver("1.x.0"));
    }

    #[test]
    fn list_installed_empty_initially() {
        let client = in_memory_client();
        assert!(client.list_installed().is_empty());
    }

    #[test]
    fn list_installed_returns_multiple_packages() {
        let client = in_memory_client();
        for name in &["alpha", "beta", "gamma"] {
            let listing = sample_listing(name);
            let sig = make_signature(name, "1.0.0");
            client.install(name, &listing, &sig).unwrap();
        }
        assert_eq!(client.list_installed().len(), 3);
    }
}
