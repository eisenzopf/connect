//! Actual published native audio custody around an opaque controlled Core capture.
//! Store metadata fixture: no decoded media/Core effect execution is claimed here.
use super::*;
use std::collections::BTreeMap;

fn capture(m: &Movement) -> crate::NativeReplacementCommitted {
    let write =
        m.f.store
            .try_native_replacement_scene_write(&m.origin, &m.sources)
            .unwrap();
    let mut observed = BTreeMap::new();
    {
        let c = m.f.store.connection().unwrap();
        for source in &m.sources {
            let before: u64 = c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![source.connection_id(),source.generation()],|r|r.get(0)).unwrap();
            observed.insert(source.runtime_connection_id().to_owned(), before + 1);
        }
    }
    // fixture(true) confirmed private Activate without an audience receipt; its
    // stored private candidate revision is2, as audience::origins derives.
    observed.insert(write.media().target().runtime_connection_id.clone(), 3);
    let audience = write.capture_commit_revisions(observed).unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &write)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let captured = attempt
        .committed(expected.clone(), &ready(&write, &expected))
        .unwrap()
        .with_audience_revisions(audience)
        .unwrap();
    drop(write);
    captured
}
fn publication(
    store: &Arc<Store>,
    source: &Arc<NativeMediaOrigin>,
) -> (
    crate::NativeMediaSnapshot,
    crate::media_authorization::OwnedWatcher,
) {
    let weak = Arc::downgrade(store);
    let origin = source.clone();
    let snapshot = store
        .register_native_media_snapshot_for(source, None, move || {
            weak.upgrade()
                .ok_or_else(Error::forbidden)?
                .try_native_media_read(&origin, None)
        })
        .unwrap();
    let watcher = store.start_media_authorization_watcher().unwrap();
    (snapshot, watcher)
}
fn admit(snapshot: &crate::NativeMediaSnapshot) -> crate::NativeMediaRead {
    let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match snapshot.admit() {
            Ok(io) => return io,
            Err(e) if e.code == "authority-busy" && std::time::Instant::now() < cutoff => {
                std::thread::yield_now()
            }
            Err(e) => panic!("original audio snapshot setup refused: {e}"),
        }
    }
}
// Capture admission is deliberately a nonblocking current SQL/file interval.
// A watcher can own primary SQL even while this exact published media epoch is
// healthy. Retry only that same immutable historical observation, never a Core
// effect, new operation ID, different receipt or terminal error.
fn record_same_capture(
    store: &Store,
    captured: &crate::NativeReplacementCommitted,
    original_current: impl Fn() -> bool,
) -> crate::Result<connect_protocol::transitions::OperationRecord> {
    let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut busy = 0;
    loop {
        match store.record_native_replacement_committed(captured) {
            Err(error)
                if error.code == "authority-busy"
                    && original_current()
                    && std::time::Instant::now() < cutoff =>
            {
                busy += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            result => {
                if busy != 0 || result.is_err() {
                    eprintln!(
                        "same original capture metadata busy_retries={busy} error={:?} diagnostics={}",
                        result.as_ref().err(), captured.projection_diagnostics()
                    );
                }
                return result;
            }
        }
    }
}
#[test]
fn first_positive_commit_capture_preserves_unaffected_audio_io_and_original_queued_epoch() {
    let m = fixture(true);
    let captured = capture(&m);
    let source = m.sources[1].clone();
    let store = Arc::new(m.f.store);
    let (snapshot, mut watcher) = publication(&store, &source);
    let io = admit(&snapshot);
    let has_audio = io.target().offer["streams_offered"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| {
            s["kind"] == "audio"
                && s["codec_preferences"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|v| v == "opus"))
        });
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let result = record_same_capture(&store, &captured, || io.current() && epoch.current());
    let survived = io.current() && epoch.current();
    let original = snapshot.admit_original(stamp);
    let original_survived = original.as_ref().is_ok_and(|v| v.current());
    drop(original);
    drop(io);
    let queued_survived = epoch.current();
    watcher.shutdown().unwrap();
    assert!(
        has_audio,
        "retained source fixture must grant actual audio media"
    );
    assert!(
        survived && original_survived && queued_survived,
        "first positive historical Commit capture revoked the original queued unaffected audio epoch despite changing no current packet permission"
    );
    let first = result.unwrap();
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let external = rusqlite::Connection::open(store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let replay = store.record_native_replacement_committed(&captured);
    external.execute_batch("ROLLBACK").unwrap();
    assert_eq!(replay.unwrap(), first);
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}

#[test]
fn positive_commit_capture_does_not_preserve_epoch_across_real_credential_revocation() {
    let m = fixture(true);
    let captured = capture(&m);
    let source = m.sources[1].clone();
    let credential = m.f.keys[1].clone();
    let store = Arc::new(m.f.store);
    let (snapshot, mut watcher) = publication(&store, &source);
    let io = admit(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let captured_result =
        record_same_capture(&store, &captured, || io.current() && epoch.current());
    let before_revoke = io.current() && epoch.current();
    let identity = store.revalidate(&credential).unwrap().identity_id;
    let scope = crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write).unwrap();
    let refused = store.try_acquire_authority(std::slice::from_ref(&scope));
    let code = refused.err().map(|e| e.code);
    let invalidated = !epoch.current();
    watcher.shutdown().unwrap();
    drop(io);
    let writer = store
        .acquire_authority(
            &[scope],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    store
        .connection_with_authority(&writer)
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [credential])
        .unwrap();
    drop(writer);
    assert!(
        captured_result.is_ok() && before_revoke,
        "original positive capture refused before actual revoke: {captured_result:?}"
    );
    assert_eq!(code.as_deref(), Some("authority-busy"));
    assert!(invalidated && !epoch.current() && snapshot.admit_original(stamp).is_err());
}

#[test]
fn unknown_cancelled_and_retired_commit_captures_keep_writer_custody() {
    for change in [
        "unknown",
        "cancel",
        "retained-retired",
        "source-revoked",
        "revision-mismatch",
        "generation-mismatch",
    ] {
        let m = fixture(true);
        let captured = capture(&m);
        let writer = m.f.controlled_conversation_writer(0);
        {
            let c = m.f.store.connection_with_authority(&writer).unwrap();
            match change {
                "unknown" => {
                    c.execute(
                        "UPDATE operations SET state='unknown' WHERE record_id=?1",
                        [&captured.commit_ledger_key().record],
                    )
                    .unwrap();
                }
                "cancel" => {
                    c.execute(
                        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                        [&captured.commit_ledger_key().record],
                    )
                    .unwrap();
                }
                "retained-retired" => {
                    c.execute(
                        "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
                        [m.sources[1].connection_id()],
                    )
                    .unwrap();
                }
                "source-revoked" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&m.f.keys[0]],
                    )
                    .unwrap();
                }
                "revision-mismatch" => {
                    c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',99) WHERE id=?1",[m.sources[1].connection_id()]).unwrap();
                }
                "generation-mismatch" => {
                    c.execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [m.sources[1].connection_id()],
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
        }
        drop(writer);
        let c = m.f.store.connection().unwrap();
        assert!(
            !crate::authority_writer_boundary::phase_ledger::commit::eligible(
                &m.f.store, &c, &captured
            )
            .unwrap(),
            "{change} entered positive Reader path"
        );
        drop(c);
        // A real same-domain Reader proves this fallback cannot silently borrow
        // Read for historical settlement. It is not a positive packet grant.
        let held =
            m.f.store
                .try_acquire_authority(&[crate::AuthorityScope::conversation(
                    &m.f.cid,
                    crate::AuthorityMode::Read,
                )
                .unwrap()])
                .unwrap();
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let refused = m.f.store.record_native_replacement_committed(&captured);
        let unchanged = Store::inspect_snapshot(m.f.store.path()).unwrap() == before;
        drop(held);
        assert_eq!(refused.unwrap_err().code, "authority-busy", "{change}");
        assert!(unchanged, "{change}");
        // Same immutable observation, no physical effect re-execution.
        let historical = record_same_capture(&m.f.store, &captured, || true);
        assert!(
            historical.is_ok(),
            "historical {change} capture refused: {historical:?}"
        );
        assert!(m
            .f
            .store
            .try_native_media_read(captured.activation().origin(), None)
            .is_err());
    }
}

#[test]
fn naturally_expired_positive_commit_capture_remains_historical_writer_only() {
    let scene = super::super::allocation::prepared_target_for_scene_named_with_timeout(
        &[],
        super::super::custody::setup(),
        "replacement",
        Some(2),
    );
    let m = fixture_prepared(true, scene);
    let captured = capture(&m);
    let key = captured.commit_ledger_key();
    let c = m.f.store.connection().unwrap();
    assert!(
        crate::authority_writer_boundary::phase_ledger::commit::eligible(&m.f.store, &c, &captured)
            .unwrap()
    );
    let expires: i64 = c
        .query_row(
            "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1",
            [&key.record],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    while crate::now() < expires {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let c = m.f.store.connection().unwrap();
    assert!(
        !crate::authority_writer_boundary::phase_ledger::commit::eligible(
            &m.f.store, &c, &captured
        )
        .unwrap()
    );
    drop(c);
    let held =
        m.f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &m.f.cid,
                crate::AuthorityMode::Read,
            )
            .unwrap()])
            .unwrap();
    let deferred = m.f.store.record_native_replacement_committed(&captured);
    drop(held);
    assert_eq!(deferred.unwrap_err().code, "authority-busy");
    assert!(m
        .f
        .store
        .record_native_replacement_committed(&captured)
        .is_ok());
    assert!(m
        .f
        .store
        .try_native_media_read(captured.activation().origin(), None)
        .is_err());
}

#[test]
fn positive_commit_permit_rejects_foreign_units_arbitrary_rows_and_unique_victims() {
    let m = fixture(true);
    let captured = capture(&m);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let proof = read.snapshot_interval().clone();
    drop(read);
    let boundary = &m.f.store.authority_writer_boundary;
    let mut foreign = rusqlite::Connection::open(m.f.store.path()).unwrap();
    let foreign_tx = foreign
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(boundary
        .native_commit_capture_ledger(&m.f.store, &foreign_tx, &proof, &captured)
        .is_err());
    foreign_tx.rollback().unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let key = captured.commit_ledger_key();
    let mut c = m.f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let incomplete =
        m.f.store
            .try_acquire_authority(&[crate::AuthorityScope::conversation(
                &m.f.cid,
                crate::AuthorityMode::Read,
            )
            .unwrap()])
            .unwrap();
    assert!(
        boundary
            .native_commit_capture_ledger(&m.f.store, &tx, &incomplete, &captured)
            .is_err(),
        "CID-only Read borrowed complete capture authority"
    );
    drop(incomplete);
    let permit = boundary
        .native_commit_capture_ledger(&m.f.store, &tx, &proof, &captured)
        .unwrap();
    let other = Fixture::new();
    assert!(other
        .store
        .authority_writer_boundary
        .native_commit_capture_ledger(&m.f.store, &tx, &proof, &captured)
        .is_err());
    // Establish every real prerequisite before testing Connection deltas;
    // otherwise the missing audience row would make those negatives vacuous.
    let target = captured.activation().target();
    let (audience_body, _) = captured
        .commit_ledger_audience()
        .unwrap()
        .positive_commit_json(
            &m.f.store,
            &tx,
            &key.record,
            &key.runtime,
            &target.connection.connection_id,
            &target.runtime_connection_id,
            target.connection.generation,
        )
        .unwrap()
        .unwrap();
    tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",params![key.record,key.expected]).unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [&key.effect],
    )
    .unwrap();
    tx.execute("INSERT INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,'commit',?2,?3,?4)",params![key.record,key.effect,key.runtime,audience_body]).unwrap();
    for sql in [
        "UPDATE native_replacement_steps SET observation_json='{}' WHERE operation_record_id=?1 AND phase='commit'",
        "UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND provider='native-replacement-commit'",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        "UPDATE operations SET lease_expires_at=lease_expires_at+1 WHERE record_id=?1",
        "UPDATE operations SET state='completed' WHERE record_id=?1",
        "INSERT OR REPLACE INTO operations SELECT * FROM operations WHERE record_id=?1",
    ] {
        assert!(
            tx.execute(sql, [&key.record]).is_err(),
            "permit authorized unrelated SQL: {sql}"
        );
    }
    let source = m.sources[1].connection_id();
    let old_json: String = tx
        .query_row(
            "SELECT readiness_json FROM connections WHERE id=?1",
            [source],
            |r| r.get(0),
        )
        .unwrap();
    let next_json: String = tx
        .query_row(
            "SELECT json_set(?1,'$.stream_revision',json_extract(?1,'$.stream_revision')+1)",
            [&old_json],
            |r| r.get(0),
        )
        .unwrap();
    let exact: i64 = tx.query_row(&format!("SELECT {}('commit-connection-revision',?1,?2,'commit',json_array(?3,?4,?5,?6,?7),NULL,NULL)",crate::authority_writer_boundary::phase_ledger::FUNCTION),params![key.record,key.effect,source,m.sources[1].generation(),key.runtime,old_json,next_json],|r|r.get(0)).unwrap();
    assert_eq!(
        exact, 1,
        "actual audience prerequisite must admit its exact pinned TEXT tuple"
    );
    let value: serde_json::Value = serde_json::from_str(&next_json).unwrap();
    let reordered = format!(
        "{{{}}}",
        value
            .as_object()
            .unwrap()
            .iter()
            .rev()
            .map(|(key, value)| format!(
                "{}:{}",
                serde_json::to_string(key).unwrap(),
                serde_json::to_string(value).unwrap()
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    for changed_text in [format!(" {next_json}"), reordered] {
        assert_ne!(changed_text, next_json);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&changed_text).unwrap(),
            value
        );
        assert!(
            tx.execute(
                "UPDATE connections SET readiness_json=?2 WHERE id=?1",
                params![source, changed_text]
            )
            .is_err(),
            "permit accepted changed JSON TEXT representation"
        );
        assert_eq!(
            tx.query_row(
                "SELECT readiness_json FROM connections WHERE id=?1",
                [source],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            old_json
        );
    }
    assert!(tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.forged',1) WHERE id=?1",[source]).is_err());
    assert!(tx
        .execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [source]
        )
        .is_err());
    assert!(tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',99) WHERE id=?1",[source]).is_err());
    assert!(tx.execute("INSERT OR REPLACE INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,'commit',?2,?3,'{}')",params![key.record,key.effect,key.runtime]).is_err());
    // json_set's actual NEW register has JSON subtype74 even though its SQL
    // typeof is TEXT. Only that ephemeral subtype may be erased by the guard.
    assert_eq!(tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',json_extract(readiness_json,'$.stream_revision')+1) WHERE id=?1",[source]).unwrap(),1);
    assert_eq!(
        tx.query_row(
            "SELECT readiness_json FROM connections WHERE id=?1",
            [source],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        next_json
    );
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    drop(proof);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}

#[test]
fn commit_capture_permit_rollback_nonce_and_commit_boundaries_remain_exact() {
    let m = fixture(true);
    let captured = capture(&m);
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let proof = read.snapshot_interval().clone();
    drop(read);
    let boundary = &m.f.store.authority_writer_boundary;
    let key = captured.commit_ledger_key();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let mut c = m.f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let stale = boundary
        .native_commit_capture_ledger(&m.f.store, &tx, &proof, &captured)
        .unwrap();
    tx.execute_batch("ROLLBACK; BEGIN IMMEDIATE").unwrap();
    let current = boundary
        .native_commit_capture_ledger(&m.f.store, &tx, &proof, &captured)
        .unwrap();
    drop(stale);
    let exact: i64 = tx
        .query_row(
            &format!(
                "SELECT {}('commit-step-confirm',?1,?2,'commit',?3,?4,NULL)",
                crate::authority_writer_boundary::phase_ledger::FUNCTION
            ),
            params![key.record, key.effect, key.expected, key.attempt],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exact, 1, "stale Drop cleared the new exact permit");
    tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",params![key.record,key.expected]).unwrap();
    // A read-only SQL transaction would not invoke commit_hook; this actual
    // write transaction does, and no permit may escape it.
    assert!(tx.execute_batch("COMMIT").is_err());
    drop(current);
    drop(tx);
    drop(c);
    drop(proof);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
}

#[test]
fn existing_or_claimed_dormant_candidate_cannot_enter_positive_commit_reader_path() {
    for state in ["preparing", "retired", "claimed"] {
        let m = if state == "claimed" {
            let (f, key, command, prepared, request, sources, _) =
                super::super::candidate::prepared_scene();
            fixture_prepared(true, (f, key, command, prepared, request, sources))
        } else {
            fixture(true)
        };
        let captured = capture(&m);
        let target = captured.activation().target();
        if state != "claimed" {
            let writer = m.f.controlled_conversation_writer(0);
            let c = m.f.store.connection_with_authority(&writer).unwrap();
            // Controlled collision created only after the original fresh
            // reservation. No target media/readiness or lifetime is fabricated.
            c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,revision) SELECT ?1,session_id,participant_id,membership_epoch,identity_id,tenant_id,?2,1,'preparing',0 FROM connections WHERE id=?3",params![target.connection.connection_id,target.connection.endpoint_id,m.sources[0].connection_id()]).unwrap();
            if state == "retired" {
                c.execute(
                    "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
                    [&target.connection.connection_id],
                )
                .unwrap();
            }
            drop(c);
            drop(writer);
        }
        let c = m.f.store.connection().unwrap();
        assert!(
            !crate::authority_writer_boundary::phase_ledger::commit::eligible(
                &m.f.store, &c, &captured
            )
            .unwrap(),
            "{state} candidate used positive Reader"
        );
        drop(c);
        let held =
            m.f.store
                .try_acquire_authority(&[crate::AuthorityScope::conversation(
                    &m.f.cid,
                    crate::AuthorityMode::Read,
                )
                .unwrap()])
                .unwrap();
        let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
        let refused = m.f.store.record_native_replacement_committed(&captured);
        let unchanged = Store::inspect_snapshot(m.f.store.path()).unwrap() == before;
        drop(held);
        assert_eq!(refused.unwrap_err().code, "authority-busy", "{state}");
        assert!(unchanged, "{state}");
        let historical = record_same_capture(&m.f.store, &captured, || true);
        assert!(
            historical.is_ok(),
            "historical {state} capture refused: {historical:?}"
        );
        assert!(m
            .f
            .store
            .try_native_media_read(captured.activation().origin(), None)
            .is_err());
    }
}

mod source_retirement;

mod commit_sql;
