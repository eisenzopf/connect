//! Exact dormant activation ledger keeps original scene revisions and IO.
use super::*;
use std::collections::BTreeMap;
fn observed(
    store: &Store,
    read: &crate::NativeReplacementSceneRead,
    sources: &[Arc<crate::NativeMediaOrigin>],
) -> BTreeMap<String, (u64, u64)> {
    let c = store.connection().unwrap();
    let mut values = BTreeMap::new();
    for source in sources {
        let revision: u64 = c.query_row(
            "SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",
            params![source.connection_id(),source.generation()], |row| row.get(0),
        ).unwrap();
        values.insert(source.runtime_connection_id().into(), (revision, revision));
    }
    values.insert(read.media().target().runtime_connection_id.clone(), (2, 3));
    values
}
fn key(store: &Store, record: &str) -> Key {
    let mut key = root_key(store, record, "activate", Stage::Begin);
    let mut expected: serde_json::Value = serde_json::from_str(&key.expected).unwrap();
    expected["candidate_only"] = serde_json::json!(true);
    key.expected = crate::provisioning::canonical_json(&expected).unwrap();
    key
}
#[test]
fn candidate_only_activation_preserves_original_io_stamp_and_connection_revisions() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let store = Arc::new(f.store);
    let id = record_id(&store, &command);
    record(&store, &applied(&store, &origin, &sources, Phase::Attach));
    let before: Vec<_> = sources
        .iter()
        .map(|source| {
            store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT readiness_json FROM connections WHERE id=?1",
                    [source.connection_id()],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        })
        .collect();
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let io = source_admitted(&source);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let cutoff = io.expires_at();
    let read = complete(&store, &origin, &sources);
    let audience = read
        .capture_candidate_only_activation_revisions(observed(&store, &read, &sources))
        .unwrap();
    assert_eq!(
        store
            .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Activate, &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let attempt = store
        .begin_native_replacement_candidate_activation(origin.clone(), &read)
        .unwrap();
    let expected = attempt.expected().unwrap();
    let capture = attempt
        .applied(expected)
        .unwrap()
        .with_audience_revisions(audience)
        .unwrap();
    drop(read);
    assert!(io.current() && epoch.current());
    let result = record(&store, &capture);
    assert_eq!(record(&store, &capture), result);
    assert_eq!(counts(&store, &id), (2, 2));
    assert!(io.current() && epoch.current());
    assert_eq!(io.expires_at(), cutoff);
    assert!(source.admit_original(stamp).unwrap().current());
    for (source, before) in sources.iter().zip(before) {
        let after: String = store
            .connection()
            .unwrap()
            .query_row(
                "SELECT readiness_json FROM connections WHERE id=?1",
                [source.connection_id()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            after, before,
            "candidate activation rewrote an original Connection"
        );
    }
    let read = complete(&store, &origin, &sources);
    let ready = store
        .begin_native_replacement_scene_media_phase(origin.clone(), Phase::Readiness, &read)
        .unwrap();
    let expected = ready.expected().unwrap();
    drop(read);
    record(&store, &ready.applied(expected).unwrap());
    assert!(io.current() && epoch.current());
    drop(io);
    drop(source);
    watcher.shutdown().unwrap();
}
#[test]
fn candidate_only_revision_capture_rejects_outside_change_missing_extra_and_wrong_before() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    record(
        &f.store,
        &applied(&f.store, &origin, &sources, Phase::Attach),
    );
    let read = complete(&f.store, &origin, &sources);
    let good = observed(&f.store, &read, &sources);
    let outside = sources[0].runtime_connection_id().to_owned();
    let target = read.media().target().runtime_connection_id.clone();
    for mode in 0..6 {
        let mut changed = good.clone();
        match mode {
            0 => {
                changed.get_mut(&outside).unwrap().1 += 1;
            }
            1 => {
                changed.remove(&outside);
            }
            2 => {
                changed.insert("foreign-controller".into(), (1, 1));
            }
            3 => {
                changed.get_mut(&outside).unwrap().0 += 1;
            }
            4 => {
                changed.get_mut(&target).unwrap().1 = 2;
            }
            _ => {
                changed.get_mut(&target).unwrap().1 = 4;
            }
        }
        assert!(read
            .capture_candidate_only_activation_revisions(changed)
            .is_err());
    }
    assert!(read
        .capture_candidate_only_activation_revisions(good)
        .is_ok());
    assert_eq!(counts(&f.store, &record_id(&f.store, &command)), (1, 1));
}
#[test]
fn candidate_activation_requires_confirmed_attach_and_same_complete_current_scene() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let read = complete(&f.store, &origin, &sources);
    assert_eq!(
        f.store
            .begin_native_replacement_candidate_activation(origin.clone(), &read)
            .unwrap_err()
            .code,
        "not-ready"
    );
    drop(read);
    assert_eq!(counts(&f.store, &record_id(&f.store, &command)), (0, 0));
    record(
        &f.store,
        &applied(&f.store, &origin, &sources, Phase::Attach),
    );
    let read = complete(&f.store, &origin, &sources);
    let (foreign, _, _, _, foreign_origin, _) = resource_fixture_scene();
    assert_eq!(
        f.store
            .begin_native_replacement_candidate_activation(foreign_origin, &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
    drop(foreign);
    let cutoff = read.expires_at();
    let until = Instant::now() + Duration::from_secs(6);
    while crate::now() < cutoff && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(crate::now() >= cutoff);
    assert!(!read.current());
    assert_eq!(
        f.store
            .begin_native_replacement_candidate_activation(origin, &read)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn uncertain_candidate_activation_retains_writer_and_real_io_drain() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let store = Arc::new(f.store);
    record(&store, &applied(&store, &origin, &sources, Phase::Attach));
    let read = complete(&store, &origin, &sources);
    let capture = store
        .begin_native_replacement_candidate_activation(origin.clone(), &read)
        .unwrap()
        .uncertain();
    drop(read);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let source = source_snapshot(&store, &sources[0]);
    let io = source_admitted(&source);
    let epoch = io.packet_epoch().unwrap();
    blocked_capture(&store, &capture, || !epoch.current());
    assert!(!epoch.current());
    drop(io);
    drop(source);
    assert_eq!(record(&store, &capture).state, OperationState::Unknown);
    assert_eq!(counts(&store, &record_id(&store, &command)), (2, 2));
    watcher.shutdown().unwrap();
}
#[test]
fn candidate_activation_unit_is_exact_and_cannot_rewrite_original_rows_or_replace_victim() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let id = record_id(&f.store, &command);
    record(
        &f.store,
        &applied(&f.store, &origin, &sources, Phase::Attach),
    );
    let key = key(&f.store, &id);
    let held = writer(&f.store, &id);
    let c = f.store.connection_with_authority(&held).unwrap();
    c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES('candidate-activation-victim',?1,'native-replacement-activate',?2,'none','prepared',?3)",params![id,key.effect,key.expected]).unwrap();
    drop(c);
    drop(held);
    let read = complete(&f.store, &origin, &sources);
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(effect_birth(&tx, &key).is_err());
    tx.rollback().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .native_phase_ledger(&f.store, &tx, read.snapshot_interval(), copy_key(&key))
        .is_err());
    let permit = f
        .store
        .authority_writer_boundary
        .native_candidate_activation_ledger(
            &f.store,
            &tx,
            read.snapshot_interval(),
            copy_key(&key),
            None,
        )
        .unwrap();
    assert!(tx.execute("INSERT OR REPLACE INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-activate',?1,'none','prepared',?3)",params![key.effect,id,key.expected]).is_err());
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM operation_effects WHERE id='candidate-activation-victim'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    for sql in [
        "UPDATE operations SET state='unknown' WHERE record_id=?1",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        "UPDATE operations SET lease_owner='foreign' WHERE record_id=?1",
        "UPDATE operations SET expires_at=expires_at+1 WHERE record_id=?1",
    ] {
        assert!(tx.execute(sql, [&id]).is_err());
    }
    assert!(tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',99) WHERE id=?1",[sources[0].connection_id()]).is_err());
    assert!(tx.execute(r#"INSERT INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,'activate',?2,?3,'{"candidate_only":true,"sources":{}}')"#,params![id,key.effect,key.runtime]).is_err());
    drop(permit);
    tx.rollback().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(effect_birth(&tx, &key).is_err());
    tx.rollback().unwrap();
}

#[test]
fn candidate_activation_audience_permit_binds_exact_json_and_preserves_unique_row() {
    let (f, _, command, _, origin, sources) = resource_fixture_scene();
    let id = record_id(&f.store, &command);
    record(
        &f.store,
        &applied(&f.store, &origin, &sources, Phase::Attach),
    );
    let read = complete(&f.store, &origin, &sources);
    let audience = read
        .capture_candidate_only_activation_revisions(observed(&f.store, &read, &sources))
        .unwrap();
    let attempt = f
        .store
        .begin_native_replacement_candidate_activation(origin, &read)
        .unwrap();
    let expected = crate::provisioning::canonical_json(&attempt.expected().unwrap()).unwrap();
    let mut key = root_key(&f.store, &id, "activate", Stage::Confirm);
    let (attempt_id,effect):(String,String)=f.store.connection().unwrap().query_row(
        "SELECT attempt_id,effect_id FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase='activate'",
        [&id],|row|Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    key.attempt = attempt_id;
    key.effect = effect;
    key.expected = expected;
    let target = read.media().target();
    let body = audience
        .candidate_activation_json(
            &id,
            &key.runtime,
            &target.runtime_connection_id,
            &target.connection.connection_id,
            target.connection.generation,
        )
        .unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .native_candidate_activation_ledger(
            &f.store,
            &tx,
            read.snapshot_interval(),
            copy_key(&key),
            None
        )
        .is_err());
    let permit = f
        .store
        .authority_writer_boundary
        .native_candidate_activation_ledger(
            &f.store,
            &tx,
            read.snapshot_interval(),
            copy_key(&key),
            Some(&audience),
        )
        .unwrap();
    tx.execute("UPDATE native_replacement_media_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='activate'",params![id,key.expected]).unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [&key.effect],
    )
    .unwrap();
    let insert="INSERT INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,'activate',?2,?3,?4)";
    let mut wrong: serde_json::Value = serde_json::from_str(&body).unwrap();
    wrong["sources"][sources[0].runtime_connection_id()][3] = serde_json::json!(99);
    assert!(tx
        .execute(
            insert,
            params![
                id,
                key.effect,
                key.runtime,
                crate::provisioning::canonical_json(&wrong).unwrap()
            ]
        )
        .is_err());
    tx.execute(insert, params![id, key.effect, key.runtime, body])
        .unwrap();
    assert!(tx.execute("INSERT OR REPLACE INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,'activate',?2,?3,?4)",params![id,key.effect,key.runtime,body]).is_err());
    assert_eq!(tx.query_row("SELECT capture_json FROM native_replacement_audiences WHERE operation_record_id=?1 AND phase='activate'",[&id],|row|row.get::<_,String>(0)).unwrap(),body);
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    drop(read);
    // This controlled transaction performed no external activation or commit.
    // Settle the same original submitted attempt through its Writer fallback.
    assert_eq!(
        record(&f.store, &attempt.uncertain()).state,
        OperationState::Unknown
    );
}
