use rusqlite::Connection;

use crate::db::init_database;
use crate::pattern_detector::PatternDetector;

fn in_memory_detector(retention_days: u32) -> PatternDetector {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(crate::db::SCHEMA).unwrap();
    PatternDetector::new(conn, retention_days)
}

fn detector_with_db_file(retention_days: u32) -> (tempfile::NamedTempFile, PatternDetector) {
    let f = tempfile::NamedTempFile::new().unwrap();
    let conn = init_database(f.path()).unwrap();
    (f, PatternDetector::new(conn, retention_days))
}

#[test]
fn record_action_stores_sequence_in_order() {
    let mut d = in_memory_detector(90);
    d.record_action("open file");
    d.record_action("edit file");
    d.record_action("save file");
    for _ in 0..4 {
        d.record_action("open file");
        d.record_action("edit file");
        d.record_action("save file");
    }
    let patterns = d.detect_patterns();
    assert!(
        patterns.iter().any(|p| p.action_sequence == vec!["open file", "edit file", "save file"]),
        "expected the 3-action sequence to be detected"
    );
}

#[test]
fn detect_patterns_requires_min_3_actions() {
    let mut d = in_memory_detector(90);
    for _ in 0..10 {
        d.record_action("a");
        d.record_action("b");
    }
    let patterns = d.detect_patterns();
    assert!(patterns.iter().all(|p| p.action_sequence.len() >= 3));
}

#[test]
fn detect_patterns_requires_min_5_occurrences() {
    let mut d = in_memory_detector(90);
    for _ in 0..4 {
        d.record_action("x");
        d.record_action("y");
        d.record_action("z");
    }
    let patterns = d.detect_patterns();
    assert!(patterns.is_empty(), "4 occurrences should not trigger detection");
}

#[test]
fn detect_patterns_triggers_at_exactly_5_occurrences() {
    let mut d = in_memory_detector(90);
    for _ in 0..5 {
        d.record_action("hawk run agent");
        d.record_action("hawk verify session");
        d.record_action("hawk undo");
    }
    let patterns = d.detect_patterns();
    assert!(patterns.iter().any(|p| p.occurrence_count >= 5));
}

#[test]
fn detect_patterns_persists_to_sqlite() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("git add .");
        d.record_action("git commit");
        d.record_action("git push");
    }
    d.detect_patterns();
    let records = d.list_patterns().unwrap();
    assert!(!records.is_empty());
}

#[test]
fn detect_patterns_upserts_on_second_call() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("a");
        d.record_action("b");
        d.record_action("c");
    }
    d.detect_patterns();
    for _ in 0..3 {
        d.record_action("a");
        d.record_action("b");
        d.record_action("c");
    }
    d.detect_patterns();
    let records = d.list_patterns().unwrap();
    let matching: Vec<_> = records.iter().filter(|r| r.action_sequence == vec!["a", "b", "c"]).collect();
    assert_eq!(matching.len(), 1, "same sequence must not create duplicate rows");
    assert!(matching[0].occurrence_count >= 5);
}

#[test]
fn accept_pattern_returns_valid_toml_manifest() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("open browser");
        d.record_action("navigate to url");
        d.record_action("extract data");
    }
    let patterns = d.detect_patterns();
    let id = &patterns[0].id;
    let manifest = d.accept_pattern(id).unwrap();
    assert!(manifest.contains("[agent]"));
    assert!(manifest.contains("name ="));
    assert!(manifest.contains("[permissions]"));
    assert!(manifest.contains("[pattern]"));
}

#[test]
fn accept_pattern_updates_status_to_accepted() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("step1");
        d.record_action("step2");
        d.record_action("step3");
    }
    let patterns = d.detect_patterns();
    let id = patterns[0].id.clone();
    d.accept_pattern(&id).unwrap();
    let records = d.list_patterns().unwrap();
    let rec = records.iter().find(|r| r.id == id).unwrap();
    assert_eq!(rec.status, "Accepted");
}

#[test]
fn accept_pattern_returns_error_for_unknown_id() {
    let d = in_memory_detector(90);
    assert!(d.accept_pattern("nonexistent-id").is_err());
}

#[test]
fn decline_pattern_updates_status_to_declined() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("cmd1");
        d.record_action("cmd2");
        d.record_action("cmd3");
    }
    let patterns = d.detect_patterns();
    let id = patterns[0].id.clone();
    d.decline_pattern(&id).unwrap();
    let records = d.list_patterns().unwrap();
    let rec = records.iter().find(|r| r.id == id).unwrap();
    assert_eq!(rec.status, "Declined");
}

#[test]
fn decline_pattern_returns_error_for_unknown_id() {
    let d = in_memory_detector(90);
    assert!(d.decline_pattern("no-such-id").is_err());
}

#[test]
fn reset_declined_re_enables_all_declined_patterns() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("p1a");
        d.record_action("p1b");
        d.record_action("p1c");
    }
    for _ in 0..5 {
        d.record_action("p2a");
        d.record_action("p2b");
        d.record_action("p2c");
    }
    let patterns = d.detect_patterns();
    assert!(patterns.len() >= 2);
    for p in &patterns {
        d.decline_pattern(&p.id).unwrap();
    }
    let reset_count = d.reset_declined().unwrap();
    assert!(reset_count >= 2);
    let records = d.list_patterns().unwrap();
    assert!(records.iter().all(|r| r.status != "Declined"));
}

#[test]
fn reset_declined_returns_zero_when_nothing_declined() {
    let d = in_memory_detector(90);
    assert_eq!(d.reset_declined().unwrap(), 0);
}

#[test]
fn cleanup_expired_removes_past_expiry_rows() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(crate::db::SCHEMA).unwrap();
    let yesterday = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO patterns (id, action_sequence, occurrence_count, last_occurrence, status, created_at, expires_at) \
         VALUES ('expired-id', '[\"a\",\"b\",\"c\"]', 5, ?1, 'Detected', ?2, ?3)",
        rusqlite::params![now, now, yesterday],
    ).unwrap();
    let d = PatternDetector::new(conn, 90);
    let deleted = d.cleanup_expired().unwrap();
    assert_eq!(deleted, 1);
    assert!(d.list_patterns().unwrap().iter().all(|r| r.id != "expired-id"));
}

#[test]
fn cleanup_expired_keeps_non_expired_rows() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("keep1");
        d.record_action("keep2");
        d.record_action("keep3");
    }
    d.detect_patterns();
    let deleted = d.cleanup_expired().unwrap();
    assert_eq!(deleted, 0);
    assert!(!d.list_patterns().unwrap().is_empty());
}

#[test]
fn list_patterns_returns_empty_when_no_patterns() {
    let d = in_memory_detector(90);
    assert!(d.list_patterns().unwrap().is_empty());
}

#[test]
fn list_patterns_includes_all_fields() {
    let (_f, mut d) = detector_with_db_file(90);
    for _ in 0..5 {
        d.record_action("hawk run");
        d.record_action("hawk ps");
        d.record_action("hawk stop 1");
    }
    d.detect_patterns();
    let records = d.list_patterns().unwrap();
    assert!(!records.is_empty());
    let r = &records[0];
    assert!(!r.id.is_empty());
    assert!(!r.action_sequence.is_empty());
    assert!(r.occurrence_count >= 5);
    assert!(!r.last_occurrence.is_empty());
    assert!(!r.status.is_empty());
}
