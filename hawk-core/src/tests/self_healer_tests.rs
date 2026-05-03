use crate::db::init_database;
use crate::self_healer::{HealingOutcome, SelfHealer};
use tempfile::NamedTempFile;

fn healer(max_retries: u32) -> (NamedTempFile, SelfHealer) {
    let f = NamedTempFile::new().unwrap();
    let db = init_database(f.path()).unwrap();
    (f, SelfHealer::new(db, max_retries))
}

fn failing_healer(max_retries: u32) -> (NamedTempFile, SelfHealer) {
    let f = NamedTempFile::new().unwrap();
    let db = init_database(f.path()).unwrap();
    (f, SelfHealer::new_with_simulator(db, max_retries, true))
}

#[test]
fn healing_reverts_to_snapshot_before_retry() {
    let (_f, h) = healer(3);
    let outcome = h.attempt_healing(1, "test error").unwrap();
    assert!(matches!(outcome, HealingOutcome::Recovered { attempt: 1, .. }));
}

#[test]
fn retry_limit_is_enforced() {
    let (_f, h) = failing_healer(3);
    let outcome = h.attempt_healing(2, "persistent error").unwrap();
    assert!(matches!(outcome, HealingOutcome::Escalated { attempts: 3, .. }));
}

#[test]
fn custom_retry_limit_respected() {
    let (_f, h) = failing_healer(5);
    let outcome = h.attempt_healing(3, "err").unwrap();
    assert!(matches!(outcome, HealingOutcome::Escalated { attempts: 5, .. }));
}

#[test]
fn successful_healing_records_recovery() {
    let (_f, h) = healer(3);
    let outcome = h.attempt_healing(10, "recoverable").unwrap();
    match outcome {
        HealingOutcome::Recovered { attempt, adjustment } => {
            assert_eq!(attempt, 1);
            assert_eq!(adjustment, "reduce_context");
        }
        _ => panic!("expected Recovered"),
    }
}

#[test]
fn exhausted_attempts_escalate_to_user() {
    let (_f, h) = failing_healer(3);
    let outcome = h.attempt_healing(20, "fatal").unwrap();
    assert!(matches!(outcome, HealingOutcome::Escalated { .. }));
}

#[test]
fn escalated_outcome_carries_original_error() {
    let (_f, h) = failing_healer(3);
    let outcome = h.attempt_healing(20, "fatal error msg").unwrap();
    match outcome {
        HealingOutcome::Escalated { last_error, .. } => assert_eq!(last_error, "fatal error msg"),
        _ => panic!("expected Escalated"),
    }
}

#[test]
fn healing_events_logged_in_sqlite() {
    let (_f, h) = healer(3);
    h.attempt_healing(30, "db test error").unwrap();
    let history = h.get_history(30).unwrap();
    assert_eq!(history.len(), 1);
    let ev = &history[0];
    assert_eq!(ev.agent_pid, 30);
    assert_eq!(ev.original_error, "db test error");
    assert_eq!(ev.outcome, "Success");
    assert!(!ev.timestamp.is_empty());
}

#[test]
fn failure_events_logged_in_sqlite() {
    let (_f, h) = failing_healer(3);
    h.attempt_healing(31, "fail error").unwrap();
    let history = h.get_history(31).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].outcome, "Failure");
    assert_eq!(history[0].attempt_number, 3);
}

#[test]
fn adjustment_field_reflects_strategy() {
    let (_f, h) = healer(3);
    h.attempt_healing(40, "err").unwrap();
    let history = h.get_history(40).unwrap();
    assert_eq!(history[0].adjustment, "reduce_context");
}

#[test]
fn get_history_returns_chronological_events() {
    let (_f, h) = failing_healer(3);
    h.attempt_healing(50, "first").unwrap();
    h.attempt_healing(50, "second").unwrap();
    let history = h.get_history(50).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].original_error, "first");
    assert_eq!(history[1].original_error, "second");
}

#[test]
fn get_history_filters_by_agent_pid() {
    let (_f, h) = failing_healer(3);
    h.attempt_healing(60, "agent-60-err").unwrap();
    h.attempt_healing(61, "agent-61-err").unwrap();
    let h60 = h.get_history(60).unwrap();
    let h61 = h.get_history(61).unwrap();
    assert_eq!(h60.len(), 1);
    assert_eq!(h61.len(), 1);
    assert_eq!(h60[0].agent_pid, 60);
    assert_eq!(h61[0].agent_pid, 61);
}
