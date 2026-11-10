//! Real Store custody and rollback; native absence is an explicit fixture fact.
use super::super::candidate::Absent;
use super::*;

#[test]
fn pre_core_refusal_retains_one_attempt_and_allows_only_exact_candidate_cleanup() {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let record_id = attempt.activation().origin().offer_record_id().to_owned();
    let gone = Absent(
        read.media().target().runtime_connection_id.clone(),
        read.media().target().connection.generation,
    );
    let refusal = attempt.not_submitted();
    drop(read);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let effects = m.f.count("operation_effects");

    // The started reservation itself is never proof of non-submission.
    assert_eq!(
        m.f.store
            .record_native_replacement_started_candidate_stopped(&m.started, &gone)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    // Historical negative metadata needs shared Conversation authority. A
    // packet reader is compatible; an actual writer must defer this recording.
    let held =
        m.f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                m.sources[0].conversation_id(),
                crate::AuthorityMode::Write,
            )
            .unwrap()])
            .unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    drop(held);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    // Failure of the second durable write must roll back the first marker.
    m.f.store
        .connection()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER pre_core_record_fault BEFORE UPDATE OF state ON operation_effects
         WHEN OLD.provider='native-replacement-commit' AND NEW.state='failed'
         BEGIN SELECT RAISE(ABORT,'pre-Core recording fault'); END;",
        )
        .unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    m.f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER pre_core_record_fault")
        .unwrap();
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);

    let packet_reader =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    let recorded =
        m.f.store
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap();
    assert!(
        packet_reader.current(),
        "historical refusal recording must not invalidate retained media"
    );
    drop(packet_reader);
    assert_eq!(
        recorded.state,
        OperationState::Preparing,
        "cleanup still pending"
    );
    let failed = recorded
        .effects
        .iter()
        .filter(|e| e.provider == "native-replacement-commit")
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].state, EffectState::Failed);
    assert_eq!(m.f.count("operation_effects"), effects);
    let recorded_snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap(),
        recorded
    );
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        recorded_snapshot
    );
    for wrong in [
        Absent("wrong-native".into(), gone.1),
        Absent(gone.0.clone(), gone.1 + 1),
    ] {
        assert_eq!(
            m.f.store
                .record_native_replacement_started_candidate_stopped(&m.started, &wrong)
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert_eq!(
            Store::inspect_snapshot(m.f.store.path()).unwrap(),
            recorded_snapshot
        );
    }
    let settled =
        m.f.store
            .record_native_replacement_started_candidate_stopped(&m.started, &gone)
            .unwrap();
    assert_eq!(settled.state, OperationState::Failed);
    let final_snapshot = Store::inspect_snapshot(m.f.store.path()).unwrap();
    assert_eq!(
        m.f.store
            .record_native_replacement_started_candidate_stopped(&m.started, &gone)
            .unwrap(),
        settled
    );
    assert_eq!(
        m.f.store
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap(),
        settled
    );
    assert!(m
        .f
        .store
        .claim_transition(&m.key, &m.command.operation_id, 60)
        .unwrap()
        .is_none());
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        final_snapshot
    );
    for source in &m.sources {
        assert!(
            m.f.store
                .try_native_media_read(source, None)
                .unwrap()
                .current(),
            "original and retained programmes remain authorized"
        );
    }
    let c = m.f.store.connection().unwrap();
    let actual:(String,u64,u64,String)=c.query_row(
        "SELECT e.state,e.attempt,e.submission_generation,s.observation_json FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='commit'",
        [&record_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
    ).unwrap();
    assert_eq!(
        actual,
        (
            "failed".into(),
            1,
            1,
            r#"{"submission":"not-started"}"#.into()
        )
    );
    let ready: u64 = c
        .query_row(
            "SELECT count(*) FROM journal WHERE operation_record_id=?1 AND kind='connection_ready'",
            [&record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ready, 0);
}

#[test]
fn pre_core_cleanup_exception_never_settles_unknown_or_confirmed_movement() {
    for known in [false, true] {
        let m = fixture(true);
        let read =
            m.f.store
                .try_native_replacement_scene_write(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &read)
                .unwrap();
        let gone = Absent(
            read.media().target().runtime_connection_id.clone(),
            read.media().target().connection.generation,
        );
        if known {
            let expected = attempt.expected().unwrap();
            let observed = ready(&read, &expected);
            let capture = attempt.committed(expected, &observed).unwrap();
            drop(read);
            m.f.store
                .record_native_replacement_committed(&capture)
                .unwrap();
        } else {
            let capture = attempt.uncertain();
            drop(read);
            m.f.store
                .native_replacement_commit_uncertain(&capture)
                .unwrap();
        }
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        assert_eq!(
            m.f.store
                .record_native_replacement_started_candidate_stopped(&m.started, &gone)
                .unwrap_err()
                .code,
            "reconciliation-required"
        );
        assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    }
}

#[test]
fn live_negative_capture_resolves_unknown_metadata_without_crossing_runtime_ownership() {
    let m = fixture(true);
    let read =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let record_id = attempt.activation().origin().offer_record_id().to_owned();
    let gone = Absent(
        read.media().target().runtime_connection_id.clone(),
        read.media().target().connection.generation,
    );
    let refusal = attempt.not_submitted();
    drop(read);
    // Model the metadata state after a generic timeout/recovery sweep. The
    // original process still owns its definite negative capture. This is not
    // a captured Core error and does not simulate a physical process crash.
    {
        let mut c = m.f.store.connection().unwrap();
        let tx = c.transaction().unwrap();
        tx.execute("UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND provider='native-replacement-commit'",[&record_id]).unwrap();
        tx.execute(
            "UPDATE operations SET state='unknown' WHERE record_id=?1",
            [&record_id],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(
        m.f.store
            .record_native_replacement_started_candidate_stopped(&m.started, &gone)
            .unwrap_err()
            .code,
        "reconciliation-required"
    );
    let captured =
        m.f.store
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap();
    assert_eq!(
        captured.state,
        OperationState::Unknown,
        "cleanup remains pending"
    );
    assert_eq!(
        m.f.store
            .record_native_replacement_started_candidate_stopped(&m.started, &gone)
            .unwrap()
            .state,
        OperationState::Failed
    );
    let path = m.f.store.path().to_owned();
    drop(m.f.store);
    let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
    let before = Store::inspect_snapshot(&path).unwrap();
    assert_eq!(
        successor
            .record_native_replacement_commit_not_submitted(&refusal)
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), before);
}
