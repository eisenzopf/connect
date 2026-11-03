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

pub(in super::super) fn arm(f: &Fixture, record: &str) -> Arc<AtomicBool> {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let raw: String = c.query_row(
        "SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='start' AND started=1",
        [record], |r| r.get(0),
    ).unwrap();
    let observed: Value = serde_json::from_str(&raw).unwrap();
    let runtime = observed["candidate"].as_str().unwrap();
    assert_eq!(observed["generation"], 2);
    f.server
        .service
        .retire_canonical_replacement_at_publication_once_for_test(record, runtime)
        .unwrap()
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
