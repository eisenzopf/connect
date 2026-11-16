//! Actual first Prepare capture and SQLite Unit. No native/Core IO is modeled
//! as a grant; the receipt uses the retained controlled Store capture fixture.
use super::*;
use connect_protocol::transitions::{EffectState, OperationState};

fn captured(
    count: usize,
    timeout_seconds: u32,
) -> (
    Fixture,
    String,
    TransitionCommand,
    Arc<NativeReplacementPrepared>,
    Vec<Arc<crate::NativeMediaOrigin>>,
) {
    let (f, key, command, origin, sources) = setup_origin(count);
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    let mut request = custody::request();
    request.timeout_seconds = timeout_seconds;
    request.participant_revision = f.store.try_connection().unwrap().query_row(
        "SELECT json_extract(readiness_json,'$.participant_revision') FROM connections WHERE id=?1",
        [sources[0].connection_id()], |row|row.get::<_,Option<u64>>(0),
    ).unwrap().unwrap_or(1);
    let lease = f
        .store
        .prepare_native_replacement(read, request)
        .unwrap()
        .unwrap();
    let expected = lease.plan().unwrap()["expected_preparation"].clone();
    let receipt = Arc::new(custody::begin(&f, lease).prepared(expected).unwrap());
    (f, key, command, receipt, sources)
}
fn counts(f: &Fixture) -> [u64; 6] {
    [
        f.count("journal"),
        f.count("operation_effects"),
        f.count("operation_observations"),
        f.count("native_replacement_public_video_reservations"),
        f.count("native_replacement_public_video_phases"),
        f.count("native_replacement_public_video_terminals"),
    ]
}
fn prepare_confirmed(record: &connect_protocol::transitions::OperationRecord) -> bool {
    let effects = record
        .effects
        .iter()
        .filter(|effect| effect.provider == "native-replacement-prepare")
        .collect::<Vec<_>>();
    effects.len() == 1
        && effects[0].state == EffectState::Confirmed
        && effects[0].attempt == 1
        && effects[0].compensates_effect_id.is_none()
}
#[test]
fn public_video_eligibility_is_cold_same_original_read_exact_profile_and_preserves_busy_foreign_expired_errors(
) {
    for count in [0, 1, 2] {
        let (f, _key, _command, origin, sources) = setup_origin(count);
        let mut read = f
            .store
            .try_native_replacement_preparation_read(
                &origin,
                &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
                &sources[1],
            )
            .unwrap();
        let before = Store::inspect_snapshot(f.store.path()).unwrap();
        let original_counts = counts(&f);
        assert_eq!(
            f.store
                .native_replacement_public_video_eligible(&read)
                .unwrap(),
            count == 1
        );
        assert!(read.current());
        // Same held SQL owner makes try_connection refuse immediately. No new
        // Reader, queued SQL, fallback classification or mutation is allowed.
        let sql = f.store.try_connection().unwrap();
        assert_eq!(
            f.store
                .native_replacement_public_video_eligible(&read)
                .err()
                .unwrap()
                .code,
            "authority-busy"
        );
        assert!(read.current());
        drop(sql);
        let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
        assert!(foreign
            .native_replacement_public_video_eligible(&read)
            .is_err());
        assert!(read.current());
        // Tighten THIS original interval, never acquire a replacement grant.
        assert!(read.limit(crate::now()).is_err());
        assert_eq!(
            f.store
                .native_replacement_public_video_eligible(&read)
                .err()
                .unwrap()
                .code,
            "forbidden"
        );
        assert_eq!(counts(&f), original_counts);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    }
    // A real default native target without video is not implicit opt-in.
    let (f, _key, _command, origin, sources) = custody::setup();
    let read = f
        .store
        .try_native_replacement_preparation_read(
            &origin,
            &crate::NativeReplacementSourceOrigin::Native(sources[0].clone()),
            &sources[1],
        )
        .unwrap();
    assert!(!f
        .store
        .native_replacement_public_video_eligible(&read)
        .unwrap());
    assert!(read.current());
}
#[test]
fn public_video_atomic_first_confirmation_exposes_intent_with_same_original_record_and_replay_under_compatible_source_read(
) {
    let (f, key, command, receipt, sources) = captured(1, 60);
    let before = same_original_snapshot(&f, &key, &command);
    assert!(!prepare_confirmed(&before.operation));
    assert!(before.intent.is_none());
    let packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let (record, reservation) = f
        .store
        .record_native_replacement_prepared_with_public_video_intent(receipt.clone())
        .unwrap();
    assert!(prepare_confirmed(&record));
    assert_eq!(record.state, OperationState::Preparing);
    assert!(packet.current());
    let after = same_original_snapshot(&f, &key, &command);
    assert_eq!(record, after.operation);
    assert!(after.phase.is_none());
    assert_eq!(
        serde_json::to_value(after.intent.unwrap()).unwrap(),
        serde_json::to_value(reservation.intent()).unwrap()
    );
    let before_replay = Store::inspect_snapshot(f.store.path()).unwrap();
    let recorded_counts = counts(&f);
    let (replayed, same) = f
        .store
        .record_native_replacement_prepared_with_public_video_intent(receipt)
        .unwrap();
    assert_eq!(replayed, record);
    assert_eq!(reservation.expires_at(), same.expires_at());
    assert_eq!(
        serde_json::to_value(reservation.intent()).unwrap(),
        serde_json::to_value(same.intent()).unwrap()
    );
    assert_eq!(counts(&f), recorded_counts);
    assert_eq!(
        Store::inspect_snapshot(f.store.path()).unwrap(),
        before_replay
    );
    assert!(packet.current());
}
#[test]
fn public_video_atomic_failure_after_both_kernels_rolls_back_every_fact_then_retries_same_original_capture(
) {
    let (f, key, command, receipt, sources) = captured(1, 60);
    let before = same_original_snapshot(&f, &key, &command);
    let snapshot = Store::inspect_snapshot(f.store.path()).unwrap();
    let original_counts = counts(&f);
    let packet = f.store.try_native_media_read(&sources[0], None).unwrap();
    let failed = f.store.test_with_public_video_atomic_rollback(|| {
        f.store
            .record_native_replacement_prepared_with_public_video_intent(receipt.clone())
    });
    assert_eq!(failed.err().unwrap().code, "storage-unavailable");
    let unchanged = same_original_snapshot(&f, &key, &command);
    assert_eq!(unchanged.operation, before.operation);
    assert!(unchanged.intent.is_none() && unchanged.phase.is_none());
    assert!(!prepare_confirmed(&unchanged.operation));
    assert_eq!(counts(&f), original_counts);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), snapshot);
    assert!(packet.current());
    let (record, intent) = f
        .store
        .record_native_replacement_prepared_with_public_video_intent(receipt)
        .unwrap();
    assert_eq!(record.record_id, before.operation.record_id);
    assert!(prepare_confirmed(&record));
    assert_eq!(intent.operation_record_id(), record.record_id);
    assert_eq!(f.count("native_replacement_public_video_reservations"), 1);
    assert!(packet.current());
}
#[test]
fn public_video_atomic_optin_refuses_legacy_confirmation_cancelled_unknown_expiry_and_ambiguous_mapping_without_mint(
) {
    for mode in 0..4 {
        let (f, key, command, receipt, _sources) = captured(
            if mode == 3 { 2 } else { 1 },
            if mode == 2 { 1 } else { 60 },
        );
        if mode == 0 {
            f.store
                .record_native_replacement_prepared(&receipt)
                .unwrap();
        }
        if mode == 1 {
            custody::cancel(&f, &key, &command);
        }
        if mode == 2 {
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let before = same_original_snapshot(&f, &key, &command);
        if mode == 1 {
            assert_eq!(before.operation.state, OperationState::Unknown);
            assert!(before.operation.cancel_requested);
        }
        let snapshot = Store::inspect_snapshot(f.store.path()).unwrap();
        let original_counts = counts(&f);
        let refused = f
            .store
            .record_native_replacement_prepared_with_public_video_intent(receipt.clone())
            .err()
            .unwrap();
        assert_eq!(
            refused.code,
            if mode == 0 {
                "conflict"
            } else if mode == 3 {
                "unsupported-media-profile"
            } else {
                "forbidden"
            },
            "mode {mode}"
        );
        assert_eq!(counts(&f), original_counts);
        assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), snapshot);
        let after = same_original_snapshot(&f, &key, &command);
        assert_eq!(after.operation, before.operation);
        assert!(after.intent.is_none() && after.phase.is_none());
        assert_eq!(f.count("native_replacement_public_video_reservations"), 0);
        if mode == 1 {
            let historical = f
                .store
                .record_native_replacement_prepared(&receipt)
                .unwrap();
            assert_eq!(historical.state, OperationState::Unknown);
            assert!(prepare_confirmed(&historical));
            assert_eq!(
                f.count("native_replacement_public_video_reservations"),
                0,
                "historical cleanup cannot opt in"
            );
        }
    }
}
