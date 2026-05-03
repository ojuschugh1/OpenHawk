// hawk-savepoint: filesystem snapshot engine ("Perch")

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;

use hawk_core::error::HawkError;

// ── Strategy ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotStrategy {
    ApfsReflink,
    BtrfsCow,
    FileCopyFallback,
}

impl SnapshotStrategy {
    pub fn detect(working_dir: &Path) -> Self {
        detect_strategy(working_dir)
    }

    fn as_db_str(&self) -> &'static str {
        match self {
            SnapshotStrategy::ApfsReflink => "apfs_reflink",
            SnapshotStrategy::BtrfsCow => "btrfs_cow",
            SnapshotStrategy::FileCopyFallback => "file_copy",
        }
    }
}

#[cfg(target_os = "macos")]
fn detect_strategy(working_dir: &Path) -> SnapshotStrategy {
    // Verify the working directory is actually on APFS before claiming reflink
    // support. A path on a USB drive (FAT32/exFAT), HFS+, or a network mount
    // would fail at reflink time. We use statvfs to read the filesystem type.
    if is_apfs(working_dir) {
        SnapshotStrategy::ApfsReflink
    } else {
        SnapshotStrategy::FileCopyFallback
    }
}

#[cfg(target_os = "macos")]
fn is_apfs(working_dir: &Path) -> bool {
    use std::ffi::CString;

    let canonical = match working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let path_cstr = match CString::new(canonical.to_string_lossy().as_bytes()) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // statfs(2) gives us f_fstypename on macOS
    #[repr(C)]
    struct StatFs {
        f_bsize: u32,
        f_iosize: i32,
        f_blocks: u64,
        f_bfree: u64,
        f_bavail: u64,
        f_files: u64,
        f_ffree: u64,
        f_fsid: [i32; 2],
        f_owner: u32,
        f_type: u32,
        f_flags: u64,
        f_fssubtype: u32,
        f_fstypename: [u8; 16],
        f_mntonname: [u8; 1024],
        f_mntfromname: [u8; 1024],
        f_reserved: [u32; 8],
    }

    extern "C" {
        fn statfs(path: *const libc::c_char, buf: *mut StatFs) -> libc::c_int;
    }

    let mut buf = std::mem::MaybeUninit::<StatFs>::uninit();
    let ret = unsafe { statfs(path_cstr.as_ptr(), buf.as_mut_ptr()) };
    if ret != 0 {
        return false;
    }
    let buf = unsafe { buf.assume_init() };
    let name_bytes = &buf.f_fstypename;
    let end = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_bytes.len());
    let fs_name = std::str::from_utf8(&name_bytes[..end]).unwrap_or("");
    fs_name == "apfs"
}

#[cfg(target_os = "linux")]
fn detect_strategy(working_dir: &Path) -> SnapshotStrategy {
    if is_btrfs(working_dir) {
        SnapshotStrategy::BtrfsCow
    } else {
        SnapshotStrategy::FileCopyFallback
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn detect_strategy(_working_dir: &Path) -> SnapshotStrategy {
    SnapshotStrategy::FileCopyFallback
}

#[cfg(target_os = "linux")]
fn is_btrfs(working_dir: &Path) -> bool {
    use std::io::{BufRead, BufReader};

    let canonical = match working_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let dir_str = canonical.to_string_lossy();

    let file = match fs::File::open("/proc/mounts") {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut best_match_len = 0usize;
    let mut best_is_btrfs = false;

    for line in BufReader::new(file).lines().flatten() {
        let parts: Vec<&str> = line.splitn(4, ' ').collect();
        if parts.len() < 3 {
            continue;
        }
        let mount_point = parts[1];
        let fs_type = parts[2];
        if dir_str.starts_with(mount_point) && mount_point.len() > best_match_len {
            best_match_len = mount_point.len();
            best_is_btrfs = fs_type == "btrfs";
        }
    }

    best_is_btrfs
}

// ── Metadata ──────────────────────────────────────────────────────────────────

pub struct SnapshotMetadata {
    pub id: String,
    pub timestamp: String,
    pub agent_pid: u32,
    pub task_description: String,
    pub file_count: u32,
    pub strategy: SnapshotStrategy,
    pub working_dir: String,
    pub session_id: String,
}

// ── Rollback / Diff types ─────────────────────────────────────────────────────

pub struct RollbackResult {
    pub snapshot_id: String,
    pub files_restored: u32,
}

pub struct FileDiff {
    pub path: String,
    pub change_type: ChangeType,
    pub old_hash: Option<String>,
    pub new_hash: Option<String>,
}

pub enum ChangeType {
    Added,
    Modified,
    Deleted,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct SnapshotEngine {
    pub db: Connection,
    pub snapshot_base_dir: PathBuf,
    pub strategy: SnapshotStrategy,
}

impl SnapshotEngine {
    pub fn new(db: Connection, snapshot_base_dir: PathBuf) -> Self {
        let strategy = SnapshotStrategy::detect(&snapshot_base_dir);
        if strategy == SnapshotStrategy::FileCopyFallback {
            eprintln!("hawk-savepoint: WARNING — COW not available, falling back to file-copy");
        }
        Self {
            db,
            snapshot_base_dir,
            strategy,
        }
    }

    pub fn create_snapshot(
        &self,
        working_dir: &Path,
        agent_pid: u32,
        task_desc: &str,
        session_id: &str,
    ) -> Result<String, HawkError> {
        let snapshot_id = Uuid::new_v4().to_string();
        let timestamp = chrono::Utc::now().to_rfc3339();

        let dest_dir = self.snapshot_base_dir.join(&snapshot_id);
        fs::create_dir_all(&dest_dir)
            .map_err(|e| HawkError::Snapshot(format!("create snapshot dir: {e}")))?;

        let mut file_entries: Vec<(String, String, u64)> = Vec::new();

        for entry in WalkDir::new(working_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let abs_src = entry.path();
            let rel_path = abs_src
                .strip_prefix(working_dir)
                .map_err(|e| HawkError::Snapshot(format!("strip prefix: {e}")))?;

            let (hash, size) = hash_file(abs_src)?;

            let abs_dest = dest_dir.join(rel_path);
            if let Some(parent) = abs_dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| HawkError::Snapshot(format!("create dir: {e}")))?;
            }

            copy_file(abs_src, &abs_dest, &self.strategy)?;
            file_entries.push((rel_path.to_string_lossy().into_owned(), hash, size));
        }

        let file_count = file_entries.len() as u32;

        self.db
            .execute(
                "INSERT INTO snapshots \
                 (id, timestamp, agent_pid, task_description, file_count, strategy, working_dir, session_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    snapshot_id,
                    timestamp,
                    agent_pid,
                    task_desc,
                    file_count,
                    self.strategy.as_db_str(),
                    working_dir.to_string_lossy().as_ref(),
                    session_id,
                ],
            )
            .map_err(|e| HawkError::Database(e.to_string()))?;

        for (rel_path, hash, size) in &file_entries {
            self.db
                .execute(
                    "INSERT INTO snapshot_files (snapshot_id, file_path, hash, size_bytes) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![snapshot_id, rel_path, hash, size],
                )
                .map_err(|e| HawkError::Database(e.to_string()))?;
        }

        Ok(snapshot_id)
    }

    pub fn rollback(&self, snapshot_id: &str) -> Result<RollbackResult, HawkError> {
        // Fetch working_dir for this snapshot
        let working_dir: String = self
            .db
            .query_row(
                "SELECT working_dir FROM snapshots WHERE id = ?1",
                params![snapshot_id],
                |r| r.get(0),
            )
            .map_err(|_| HawkError::NotFound(format!("snapshot {snapshot_id}")))?;

        let working_dir = PathBuf::from(&working_dir);
        let snap_dir = self.snapshot_base_dir.join(snapshot_id);

        // Collect snapshot manifest: rel_path → hash
        let mut snap_files: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let mut stmt = self
                .db
                .prepare("SELECT file_path, hash FROM snapshot_files WHERE snapshot_id = ?1")
                .map_err(|e| HawkError::Database(e.to_string()))?;
            let rows = stmt
                .query_map(params![snapshot_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| HawkError::Database(e.to_string()))?;
            for row in rows {
                let (path, hash) = row.map_err(|e| HawkError::Database(e.to_string()))?;
                snap_files.insert(path, hash);
            }
        }

        // Restore every file from the snapshot directory
        let mut files_restored: u32 = 0;
        for rel_path in snap_files.keys() {
            let src = snap_dir.join(rel_path);
            let dest = working_dir.join(rel_path);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| HawkError::Snapshot(format!("create dir: {e}")))?;
            }
            copy_file(&src, &dest, &self.strategy)?;
            files_restored += 1;
        }

        // Remove files present in working_dir that were not in the snapshot
        for entry in WalkDir::new(&working_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let rel = entry
                .path()
                .strip_prefix(&working_dir)
                .map_err(|e| HawkError::Snapshot(format!("strip prefix: {e}")))?
                .to_string_lossy()
                .into_owned();
            if !snap_files.contains_key(&rel) {
                fs::remove_file(entry.path())
                    .map_err(|e| HawkError::Snapshot(format!("remove file: {e}")))?;
            }
        }

        Ok(RollbackResult {
            snapshot_id: snapshot_id.to_owned(),
            files_restored,
        })
    }

    pub fn rollback_latest(&self, agent_pid: u32) -> Result<RollbackResult, HawkError> {
        let snapshot_id: String = self
            .db
            .query_row(
                "SELECT id FROM snapshots WHERE agent_pid = ?1 \
                 ORDER BY timestamp DESC LIMIT 1",
                params![agent_pid],
                |r| r.get(0),
            )
            .map_err(|_| HawkError::NotFound(format!("no snapshots for agent {agent_pid}")))?;
        self.rollback(&snapshot_id)
    }

    pub fn diff(&self, snapshot_id: &str) -> Result<Vec<FileDiff>, HawkError> {
        let working_dir: String = self
            .db
            .query_row(
                "SELECT working_dir FROM snapshots WHERE id = ?1",
                params![snapshot_id],
                |r| r.get(0),
            )
            .map_err(|_| HawkError::NotFound(format!("snapshot {snapshot_id}")))?;

        let working_dir = PathBuf::from(&working_dir);

        // Load snapshot manifest
        let mut snap_files: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let mut stmt = self
                .db
                .prepare("SELECT file_path, hash FROM snapshot_files WHERE snapshot_id = ?1")
                .map_err(|e| HawkError::Database(e.to_string()))?;
            let rows = stmt
                .query_map(params![snapshot_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|e| HawkError::Database(e.to_string()))?;
            for row in rows {
                let (path, hash) = row.map_err(|e| HawkError::Database(e.to_string()))?;
                snap_files.insert(path, hash);
            }
        }

        // Hash current working directory files
        let mut current_files: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        if working_dir.exists() {
            for entry in WalkDir::new(&working_dir)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let rel = entry
                    .path()
                    .strip_prefix(&working_dir)
                    .map_err(|e| HawkError::Snapshot(format!("strip prefix: {e}")))?
                    .to_string_lossy()
                    .into_owned();
                let (hash, _) = hash_file(entry.path())?;
                current_files.insert(rel, hash);
            }
        }

        let mut diffs = Vec::new();

        // Files in snapshot: Deleted or Modified
        for (path, old_hash) in &snap_files {
            match current_files.get(path) {
                None => diffs.push(FileDiff {
                    path: path.clone(),
                    change_type: ChangeType::Deleted,
                    old_hash: Some(old_hash.clone()),
                    new_hash: None,
                }),
                Some(new_hash) if new_hash != old_hash => diffs.push(FileDiff {
                    path: path.clone(),
                    change_type: ChangeType::Modified,
                    old_hash: Some(old_hash.clone()),
                    new_hash: Some(new_hash.clone()),
                }),
                _ => {}
            }
        }

        // Files in current but not in snapshot: Added
        for (path, new_hash) in &current_files {
            if !snap_files.contains_key(path) {
                diffs.push(FileDiff {
                    path: path.clone(),
                    change_type: ChangeType::Added,
                    old_hash: None,
                    new_hash: Some(new_hash.clone()),
                });
            }
        }

        Ok(diffs)
    }

    pub fn list_snapshots(
        &self,
        agent_pid: Option<u32>,
    ) -> Result<Vec<SnapshotMetadata>, HawkError> {
        let mut stmt = match agent_pid {
            Some(_) => self.db.prepare(
                "SELECT id, timestamp, agent_pid, task_description, file_count, strategy, working_dir, session_id \
                 FROM snapshots WHERE agent_pid = ?1 ORDER BY timestamp DESC",
            ),
            None => self.db.prepare(
                "SELECT id, timestamp, agent_pid, task_description, file_count, strategy, working_dir, session_id \
                 FROM snapshots ORDER BY timestamp DESC",
            ),
        }
        .map_err(|e| HawkError::Database(e.to_string()))?;

        let map_row = |r: &rusqlite::Row<'_>| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        };

        let rows: Vec<_> = match agent_pid {
            Some(pid) => stmt
                .query_map(params![pid], map_row)
                .map_err(|e| HawkError::Database(e.to_string()))?
                .collect(),
            None => stmt
                .query_map([], map_row)
                .map_err(|e| HawkError::Database(e.to_string()))?
                .collect(),
        };

        let mut result = Vec::new();
        for row in rows {
            let (id, timestamp, pid, task_desc, file_count, strategy_str, working_dir, session_id) =
                row.map_err(|e| HawkError::Database(e.to_string()))?;
            let strategy = match strategy_str.as_str() {
                "apfs_reflink" => SnapshotStrategy::ApfsReflink,
                "btrfs_cow" => SnapshotStrategy::BtrfsCow,
                _ => SnapshotStrategy::FileCopyFallback,
            };
            result.push(SnapshotMetadata {
                id,
                timestamp,
                agent_pid: pid,
                task_description: task_desc,
                file_count,
                strategy,
                working_dir,
                session_id,
            });
        }
        Ok(result)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn hash_file(path: &Path) -> Result<(String, u64), HawkError> {
    let mut file =
        fs::File::open(path).map_err(|e| HawkError::Snapshot(format!("open file: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut size = 0u64;
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| HawkError::Snapshot(format!("read file: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hex::encode(hasher.finalize()), size))
}

fn copy_file(src: &Path, dest: &Path, strategy: &SnapshotStrategy) -> Result<(), HawkError> {
    match strategy {
        SnapshotStrategy::FileCopyFallback => {
            eprintln!(
                "hawk-savepoint: WARNING — file-copy fallback for {}",
                src.display()
            );
            fs::copy(src, dest).map_err(|e| HawkError::Snapshot(format!("file copy: {e}")))?;
        }
        // COW variants also use std::fs::copy for now; reflink-copy can be wired in later
        SnapshotStrategy::ApfsReflink | SnapshotStrategy::BtrfsCow => {
            fs::copy(src, dest).map_err(|e| HawkError::Snapshot(format!("file copy: {e}")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_engine(snapshot_base: &Path) -> SnapshotEngine {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.execute_batch(hawk_core::db::SCHEMA).unwrap();
        SnapshotEngine {
            db: conn,
            snapshot_base_dir: snapshot_base.to_path_buf(),
            strategy: SnapshotStrategy::FileCopyFallback,
        }
    }

    fn write_file(dir: &Path, name: &str, content: &[u8]) {
        let mut f = fs::File::create(dir.join(name)).unwrap();
        f.write_all(content).unwrap();
    }

    #[test]
    fn test_snapshot_creates_files_and_db_rows() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        write_file(work.path(), "a.txt", b"hello");
        write_file(work.path(), "b.txt", b"world");

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('sess1', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        let id = engine
            .create_snapshot(work.path(), 42, "test task", "sess1")
            .unwrap();

        assert!(snap_base.path().join(&id).join("a.txt").exists());
        assert!(snap_base.path().join(&id).join("b.txt").exists());

        let file_count: i64 = engine
            .db
            .query_row(
                "SELECT file_count FROM snapshots WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(file_count, 2);

        let manifest_count: i64 = engine
            .db
            .query_row(
                "SELECT COUNT(*) FROM snapshot_files WHERE snapshot_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(manifest_count, 2);
    }

    #[test]
    fn test_snapshot_records_correct_hash() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        let content = b"deterministic content";
        write_file(work.path(), "f.txt", content);

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('sess2', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        let id = engine
            .create_snapshot(work.path(), 1, "hash test", "sess2")
            .unwrap();

        let stored_hash: String = engine
            .db
            .query_row(
                "SELECT hash FROM snapshot_files WHERE snapshot_id = ?1 AND file_path = 'f.txt'",
                params![id],
                |r| r.get(0),
            )
            .unwrap();

        let mut hasher = Sha256::new();
        hasher.update(content);
        let expected = hex::encode(hasher.finalize());
        assert_eq!(stored_hash, expected);
    }

    #[test]
    fn test_snapshot_empty_dir() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('sess3', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        let id = engine
            .create_snapshot(work.path(), 99, "empty", "sess3")
            .unwrap();

        let file_count: i64 = engine
            .db
            .query_row(
                "SELECT file_count FROM snapshots WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(file_count, 0);
    }

    #[test]
    fn test_strategy_db_strings() {
        assert_eq!(SnapshotStrategy::ApfsReflink.as_db_str(), "apfs_reflink");
        assert_eq!(SnapshotStrategy::BtrfsCow.as_db_str(), "btrfs_cow");
        assert_eq!(SnapshotStrategy::FileCopyFallback.as_db_str(), "file_copy");
    }

    #[test]
    fn test_hash_file_consistent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("x.bin");
        fs::write(&path, b"abc123").unwrap();

        let (h1, s1) = hash_file(&path).unwrap();
        let (h2, s2) = hash_file(&path).unwrap();
        assert_eq!(h1, h2);
        assert_eq!(s1, s2);
        assert_eq!(s1, 6);
    }

    // ── rollback ──────────────────────────────────────────────────────────────

    #[test]
    fn test_rollback_restores_files() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        write_file(work.path(), "a.txt", b"original");
        write_file(work.path(), "b.txt", b"also original");

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s1', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        let id = engine
            .create_snapshot(work.path(), 1, "before", "s1")
            .unwrap();

        // Mutate working dir after snapshot
        write_file(work.path(), "a.txt", b"modified");
        write_file(work.path(), "c.txt", b"new file");

        let result = engine.rollback(&id).unwrap();
        assert_eq!(result.snapshot_id, id);
        assert_eq!(result.files_restored, 2);

        // a.txt should be restored
        assert_eq!(fs::read(work.path().join("a.txt")).unwrap(), b"original");
        // b.txt should still be there
        assert_eq!(
            fs::read(work.path().join("b.txt")).unwrap(),
            b"also original"
        );
        // c.txt was added after snapshot — should be removed
        assert!(!work.path().join("c.txt").exists());
    }

    #[test]
    fn test_rollback_latest_picks_most_recent() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        write_file(work.path(), "v.txt", b"v1");

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s2', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        // First snapshot
        engine
            .create_snapshot(work.path(), 7, "snap1", "s2")
            .unwrap();

        // Modify and take second snapshot
        write_file(work.path(), "v.txt", b"v2");
        engine
            .create_snapshot(work.path(), 7, "snap2", "s2")
            .unwrap();

        // Modify again
        write_file(work.path(), "v.txt", b"v3");

        let result = engine.rollback_latest(7).unwrap();
        assert_eq!(result.files_restored, 1);
        // Should restore to v2 (most recent snapshot)
        assert_eq!(fs::read(work.path().join("v.txt")).unwrap(), b"v2");
    }

    #[test]
    fn test_rollback_unknown_snapshot_returns_error() {
        let snap_base = TempDir::new().unwrap();
        let engine = make_engine(snap_base.path());
        assert!(engine.rollback("nonexistent-id").is_err());
    }

    // ── diff ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_diff_added_modified_deleted() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        write_file(work.path(), "keep.txt", b"same");
        write_file(work.path(), "modify.txt", b"before");
        write_file(work.path(), "delete.txt", b"will be gone");

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s3', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        let id = engine
            .create_snapshot(work.path(), 2, "diff test", "s3")
            .unwrap();

        // Apply changes
        write_file(work.path(), "modify.txt", b"after");
        fs::remove_file(work.path().join("delete.txt")).unwrap();
        write_file(work.path(), "added.txt", b"new");

        let diffs = engine.diff(&id).unwrap();

        let find = |name: &str| diffs.iter().find(|d| d.path == name);

        let modified = find("modify.txt").expect("modify.txt should be Modified");
        assert!(matches!(modified.change_type, ChangeType::Modified));
        assert!(modified.old_hash.is_some());
        assert!(modified.new_hash.is_some());

        let deleted = find("delete.txt").expect("delete.txt should be Deleted");
        assert!(matches!(deleted.change_type, ChangeType::Deleted));
        assert!(deleted.old_hash.is_some());
        assert!(deleted.new_hash.is_none());

        let added = find("added.txt").expect("added.txt should be Added");
        assert!(matches!(added.change_type, ChangeType::Added));
        assert!(added.old_hash.is_none());
        assert!(added.new_hash.is_some());

        // keep.txt should not appear in diffs
        assert!(find("keep.txt").is_none());
    }

    #[test]
    fn test_diff_no_changes_returns_empty() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        write_file(work.path(), "f.txt", b"unchanged");

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s4', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        let id = engine
            .create_snapshot(work.path(), 3, "no change", "s4")
            .unwrap();

        let diffs = engine.diff(&id).unwrap();
        assert!(diffs.is_empty());
    }

    // ── list_snapshots ────────────────────────────────────────────────────────

    #[test]
    fn test_list_snapshots_all() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s5', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        engine.create_snapshot(work.path(), 10, "t1", "s5").unwrap();
        engine.create_snapshot(work.path(), 20, "t2", "s5").unwrap();

        let all = engine.list_snapshots(None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_list_snapshots_filtered_by_agent() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s6', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        engine
            .create_snapshot(work.path(), 11, "agent11", "s6")
            .unwrap();
        engine
            .create_snapshot(work.path(), 22, "agent22", "s6")
            .unwrap();
        engine
            .create_snapshot(work.path(), 11, "agent11-2", "s6")
            .unwrap();

        let for_11 = engine.list_snapshots(Some(11)).unwrap();
        assert_eq!(for_11.len(), 2);
        assert!(for_11.iter().all(|s| s.agent_pid == 11));

        let for_22 = engine.list_snapshots(Some(22)).unwrap();
        assert_eq!(for_22.len(), 1);
    }

    #[test]
    fn test_list_snapshots_ordered_desc() {
        let work = TempDir::new().unwrap();
        let snap_base = TempDir::new().unwrap();

        let engine = make_engine(snap_base.path());
        engine
            .db
            .execute(
                "INSERT INTO sessions (id, started_at, status) VALUES ('s7', datetime('now'), 'Active')",
                [],
            )
            .unwrap();

        engine
            .create_snapshot(work.path(), 5, "first", "s7")
            .unwrap();
        engine
            .create_snapshot(work.path(), 5, "second", "s7")
            .unwrap();

        let snaps = engine.list_snapshots(Some(5)).unwrap();
        assert_eq!(snaps.len(), 2);
        // DESC order: most recent first
        assert!(snaps[0].timestamp >= snaps[1].timestamp);
    }
}
