//! Actual native retirement after the point observation, before final SQL commit.
//! Resource custody must reject publication and retain the one known movement.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[tokio::test]
async fn production_native_retirement_between_observation_and_publication_never_emits_ready() {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::RetireAtPublication,
    )
    .await;
}

#[tokio::test]
async fn production_native_retirement_during_publication_orders_ready_before_retired_without_replay(
) {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::RetireDuringPublication,
    )
    .await;
}

pub(in super::super) fn arm(f: &Fixture, record: &str) -> Arc<AtomicBool> {
    arm_kind(f, record, false)
}

pub(in super::super) fn arm_during(f: &Fixture, record: &str) -> Arc<AtomicBool> {
    arm_kind(f, record, true)
}

fn arm_kind(f: &Fixture, record: &str, during: bool) -> Arc<AtomicBool> {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let raw: String = c.query_row(
        "SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='start' AND started=1",
        [record], |r| r.get(0),
    ).unwrap();
    let observed: Value = serde_json::from_str(&raw).unwrap();
    let runtime = observed["candidate"].as_str().unwrap();
    assert_eq!(observed["generation"], 2);
    if during {
        f.server
            .service
            .retire_canonical_replacement_during_publication_once_for_test(record, runtime)
            .unwrap()
    } else {
        f.server
            .service
            .retire_canonical_replacement_at_publication_once_for_test(record, runtime)
            .unwrap()
    }
}

pub(in super::super) async fn verify(
    f: &Fixture,
    credential: &str,
    command: &TransitionCommand,
    candidate: &str,
    controller: &mut CanonicalHttpClient,
    peers: &mut [Peer],
    armed: Arc<AtomicBool>,
    before: Vec<(String, String, u64, Option<String>)>,
) {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    let stopped = loop {
        let record = status(f, credential, &command.operation_id).await;
        if record.state == OperationState::Completed {
            super::stopped_without_ready(f, candidate, &record);
            if f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .replacement_preparations
                == 0
            {
                break record;
            }
        }
        assert!(
            tokio::time::Instant::now() < until,
            "publication retirement did not settle: {record:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        !armed.load(Ordering::Acquire),
        "exact publication interleaving was never reached"
    );
    assert_eq!(stopped.effects.len(), 7);
    assert!(stopped
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed
            && e.attempt == 1
            && e.submission_generation == 1));
    assert_eq!(
        stopped
            .effects
            .iter()
            .filter(|e| e.provider == "native-replacement-commit")
            .count(),
        1
    );
    assert!(stopped
        .effects
        .iter()
        .all(|e| e.compensates_effect_id.is_none()));
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let capture: String = c.query_row(
        "SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",
        [&stopped.record_id],|r|r.get(0),
    ).unwrap();
    let capture: Value = serde_json::from_str(&capture).unwrap();
    assert!(
        capture["movement"].is_object(),
        "known movement must not become non-submission"
    );
    drop(c);
    wait_slots(f, 1).await;
    let retained = |rows: Vec<(String, String, u64, Option<String>)>| {
        rows.into_iter()
            .filter(|row| row.0 == "codec-source")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        retained(super::pre_core::original_bindings(f)),
        retained(before.clone())
    );
    peers[1].live = false;
    peers[2].live = false;
    // Only the retained peer survives the known move plus candidate retirement.
    // This proves it can still send without a forbidden receiver, not duplex.
    probe(
        peers,
        0,
        &[],
        "publication-retirement-retains-original-remote",
    )
    .await;
    controller.transition(command.clone()).await.unwrap();
    let repeated = status(f, credential, &command.operation_id).await;
    assert_eq!(repeated.record_id, stopped.record_id);
    assert_eq!(repeated.state, stopped.state);
    assert_eq!(repeated.revision, stopped.revision);
    assert_eq!(repeated.effects, stopped.effects);
    assert_eq!(repeated.result, stopped.result);
    super::stopped_without_ready(f, candidate, &repeated);
    assert_eq!(
        retained(super::pre_core::original_bindings(f)),
        retained(before)
    );
    wait_slots(f, 1).await;
    eprintln!("native publication retirement: exact interleaving consumed, captured movement once, no Ready, stopped result/exact target retirement, retained native binding unchanged and original public replay identical");
}

/// Cancellation starts inside the actual native publication callback. The hook
/// proves retire() is fenced on metadata while its original token is cancelled.
/// A committed Ready remains historical truth; ordinary peer shutdown then
/// drives the existing adapter cleanup and Store retirement projection.
pub(in super::super) async fn verify_during(
    f: &Fixture,
    credential: &str,
    command: &TransitionCommand,
    candidate: &str,
    controller: &mut CanonicalHttpClient,
    peers: &mut [Peer],
    armed: Arc<AtomicBool>,
    before: Vec<(String, String, u64, Option<String>)>,
) {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    let done = loop {
        let record = status(f, credential, &command.operation_id).await;
        if record.state == OperationState::Completed {
            break record;
        }
        assert!(
            !matches!(
                record.state,
                OperationState::Failed | OperationState::Unknown | OperationState::Cancelled
            ),
            "publication failed: {record:?}"
        );
        assert!(
            tokio::time::Instant::now() < until,
            "publication did not complete: {record:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        !armed.load(Ordering::Acquire),
        "overlap hook was not reached"
    );
    assert_eq!(done.result.as_ref().unwrap()["activation_state"], "ready");
    assert_eq!(done.result.as_ref().unwrap()["connection_id"], candidate);
    assert_eq!(done.effects.len(), 7);
    assert!(done
        .effects
        .iter()
        .all(|e| e.state == EffectState::Confirmed
            && e.attempt == 1
            && e.submission_generation == 1
            && e.compensates_effect_id.is_none()));
    assert_eq!(
        done.effects
            .iter()
            .filter(|e| e.provider == "native-replacement-commit")
            .count(),
        1
    );
    peers[1].live = false;
    peers[2].live = false;
    // Close by reference; the owner drains every client after this audit.
    peers[2].client.close();
    wait_slots(f, 1).await;
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let retired: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE l.operation_record_id=?1 AND c.id=?2 AND l.retired=1 AND l.generation=2 AND c.generation=3 AND c.state='retired' AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL)",
            rusqlite::params![done.record_id,candidate], |r| r.get(0),
        ).unwrap();
        if retired {
            let (ready_last, retired_first): (Option<u64>, Option<u64>) = c.query_row(
                "SELECT max(CASE WHEN kind='connection_ready' THEN sequence END),min(CASE WHEN kind='connection_retired' THEN sequence END) FROM journal WHERE json_extract(data_json,'$.connection_id')=?1",
                [candidate], |r| Ok((r.get(0)?,r.get(1)?)),
            ).unwrap();
            assert!(ready_last.is_some() && retired_first.is_some());
            assert!(
                ready_last.unwrap() < retired_first.unwrap(),
                "Ready must precede retirement"
            );
            let duplicate_ready: u64 = c.query_row(
                "SELECT count(*) FROM (SELECT recipient,count(*) AS n FROM journal WHERE kind='connection_ready' AND json_extract(data_json,'$.connection_id')=?1 GROUP BY recipient HAVING n<>1)",
                [candidate],|r|r.get(0),
            ).unwrap();
            assert_eq!(duplicate_ready, 0);
            break;
        }
        drop(c);
        assert!(
            tokio::time::Instant::now() < until,
            "exact ordinary lifetime did not retire"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let retained = |rows: Vec<(String, String, u64, Option<String>)>| {
        rows.into_iter()
            .filter(|r| r.0 == "codec-source")
            .collect::<Vec<_>>()
    };
    assert_eq!(
        retained(super::pre_core::original_bindings(f)),
        retained(before.clone())
    );
    let events_before_replay = publication_events(f, candidate);
    controller.transition(command.clone()).await.unwrap();
    let replay = status(f, credential, &command.operation_id).await;
    assert_eq!(replay.record_id, done.record_id);
    assert_eq!(replay.state, done.state);
    assert_eq!(replay.revision, done.revision);
    assert_eq!(replay.effects, done.effects);
    assert_eq!(replay.result, done.result);
    assert_eq!(publication_events(f, candidate), events_before_replay);
    assert_eq!(
        retained(super::pre_core::original_bindings(f)),
        retained(before)
    );
    wait_slots(f, 1).await;
    eprintln!("native publication overlap: real retirement fenced during callback, cancellation observed before SQL commit, Ready ordered before exact retirement, one movement and unchanged original replay");
}

fn publication_events(f: &Fixture, candidate: &str) -> Vec<(u64, String, String)> {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let mut q = c.prepare("SELECT sequence,recipient,kind FROM journal WHERE json_extract(data_json,'$.connection_id')=?1 AND kind IN ('connection_ready','connection_retired') ORDER BY sequence").unwrap();
    let events = q
        .query_map([candidate], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    events
}
