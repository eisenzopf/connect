//! Actual signed native replacement under a blocked final Store transaction.
//! Scope: target revocation before public activation, not process-crash recovery.
use super::*;
#[path = "activation/crash.rs"]
pub(super) mod crash;
#[cfg(feature = "test-support")]
#[path = "activation/data_observer.rs"]
pub(super) mod data_observer;
#[cfg(feature = "test-support")]
#[path = "activation/pre_core.rs"]
pub(super) mod pre_core;
#[cfg(feature = "test-support")]
#[path = "activation/publication.rs"]
pub(super) mod publication;

#[tokio::test]
async fn production_captured_replacement_revocation_never_publishes_ready_or_replays_movement() {
    exercise().await;
}

pub(super) async fn exercise() {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        true,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::RevokeBeforeReady,
    )
    .await;
}

pub(super) fn unpublished(f: &Fixture, candidate: &str, record: &str) {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let ready: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id=?1",
            [record],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ready, 0);
    let published: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND state IN ('ready','active')) OR EXISTS(SELECT 1 FROM native_replacement_lifetimes WHERE operation_record_id=?2)",
        rusqlite::params![candidate,record], |r| r.get(0),
    ).unwrap();
    assert!(
        !published,
        "blocked final publication must not expose Ready or a durable ordinary lifetime"
    );
}

pub(super) fn stopped_without_ready(f: &Fixture, candidate: &str, record: &OperationRecord) {
    let result = record.result.as_ref().unwrap();
    assert_eq!(result["activation_state"], "stopped");
    assert!(result["readiness"].is_null());
    assert!(result["physical_readiness"].is_object());
    assert_eq!(result["generation"], 2);
    for field in ["admitted_connection_ids", "retired_connection_ids"] {
        assert!(result[field]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == candidate));
    }
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let exact: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE l.operation_record_id=?1 AND c.id=?2 AND l.retired=1 AND l.generation=2 AND c.generation=3 AND c.state='retired' AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL)",
        rusqlite::params![record.record_id,candidate], |r| r.get(0),
    ).unwrap();
    assert!(exact);
    let ready: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE kind='connection_ready' AND operation_record_id=?1",
            [&record.record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ready, 0,
        "a stopped-before-activation route never emitted Ready"
    );
}
