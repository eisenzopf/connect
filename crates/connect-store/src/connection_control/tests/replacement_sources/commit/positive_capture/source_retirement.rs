//! Real original-native cleanup before capture, then a NEW healthy audio epoch.
//! Controlled Store/Core observations only; no sample delivery/decoder claim.
use super::*;

fn ready_with_lease(
    f: &Fixture,
    actor: usize,
) -> (Arc<NativeMediaOrigin>, crate::NativeOfferLease) {
    let request = f.native_request_for(actor, 600, &format!("commit-retired-{actor}"));
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(
            &f.keys[actor],
            &proof,
            Some(connect_protocol::provisioning::EndpointTransport::Quic),
        )
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &lease)
        .unwrap();
    drop(proof);
    // Same controlled ready fixture as consultation_reads::ready_request,
    // retaining its ORIGINAL real lease solely for the real cleanup API below.
    let writer = f.controlled_conversation_writer(actor);
    let mut c = f.store.connection_with_authority(&writer).unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
        [lease.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    tx.commit().unwrap();
    drop(c);
    drop(writer);
    (origin, lease)
}

fn movement() -> (
    Movement,
    crate::NativeOfferLease,
    crate::NativeReplacementOrigin,
) {
    use crate::connection_control::tests::replacement_sources as fixtures;
    let f = Fixture::new();
    let (old, lease) = ready_with_lease(&f, 0);
    let retained = crate::connection_control::tests::consultation_reads::ready(&f, 1);
    let target = fixtures::endpoint(&f, vec!["audio".into()], "replacement");
    let key = fixtures::key(&f);
    let command = TransitionCommand {
        operation_id: "move-native-endpoint".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(old.connection_id().into()),
        expected_revision: 3,
        intent: connect_protocol::transitions::TransitionIntent::Replace {
            endpoint_id: Some(target),
            candidate_connid: None,
            retained_connid: retained.connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let scene = fixtures::allocation::prepared_target_for_scene(
        &[],
        (f, key, command, origin.clone(), vec![old, retained]),
    );
    (fixture_prepared(true, scene), lease, origin)
}

fn retire_original(m: &Movement, lease: &crate::NativeOfferLease) {
    let absent = crate::connection_control::tests::replacement_sources::candidate::Absent(
        lease.runtime_connection_id().into(),
        lease.generation(),
    );
    m.f.store
        .settle_native_offer_cleanup(lease, &absent)
        .unwrap();
    let c = m.f.store.connection().unwrap();
    let retired:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN connection_offers mapping ON mapping.connection_id=v.id JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.operation_record_id=?2 WHERE v.id=?1 AND v.state='retired' AND v.generation=?3+1 AND v.observed_runtime_id IS NULL AND v.observed_at IS NULL AND v.readiness_json IS NULL AND l.runtime_epoch=?4 AND l.generation=?3 AND l.retired=1 AND mapping.runtime_connection_id=?5)",params![lease.connection_id(),lease.record_id(),lease.generation(),m.f.store.runtime_epoch(),lease.runtime_connection_id()],|r|r.get(0)).unwrap();
    let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id='session' AND participant_id=?1 AND state='present')",[&m.f.pids[0]],|r|r.get(0)).unwrap();
    drop(c);
    assert!(
        retired,
        "actual original custodian cleanup did not retire its exact incarnation"
    );
    assert!(present,"pending original replacement must preserve current participant; do not repair it in fixture");
    assert_eq!(
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
}

#[test]
fn known_commit_after_real_old_retirement_preserves_fresh_healthy_audio_epoch_without_old_io() {
    let (m, lease, original) = movement();
    let captured = capture(&m);
    retire_original(&m, &lease);
    let identity = m.f.store.revalidate(&m.f.keys[0]).unwrap().identity_id;
    let c = m.f.store.connection().unwrap();
    original
        .continuation_authority(&m.f.store, &c, &identity)
        .expect(
            "original actor/member/endpoint authority must remain current after exact retirement",
        );
    drop(c);
    let retired_state = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let source = m.sources[1].clone();
    let old = m.sources[0].clone();
    let store = Arc::new(m.f.store);
    // This is a FRESH grant after the legitimate retirement Writer. An old
    // pre-retirement epoch must never be revived to make this assertion pass.
    let (snapshot, mut watcher) = publication(&store, &source);
    let io = admit(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let captured_result =
        record_same_capture(&store, &captured, || io.current() && epoch.current());
    let survived = io.current() && epoch.current();
    let same_stamp = snapshot.admit_original(stamp);
    let stamp_survived = same_stamp.as_ref().is_ok_and(|io| io.current());
    drop(same_stamp);
    drop(io);
    let queued_survived = epoch.current();
    watcher.shutdown().unwrap();
    let old_refused = store.try_native_media_read(&old, None).is_err();
    assert!(survived && stamp_survived && queued_survived,"known historical capture after actual old-native retirement revoked the NEW healthy unrelated audio epoch");
    assert!(old_refused, "historical metadata revived old source IO");
    let first = captured_result.unwrap();
    assert_eq!(first.state, OperationState::Preparing);
    let after = Store::inspect_snapshot(store.path()).unwrap();
    assert_ne!(
        after, retired_state,
        "same original known observation was not recorded"
    );
    let c = store.connection().unwrap();
    let old_retired:bool=c.query_row("SELECT state='retired' AND generation=?2+1 AND observed_runtime_id IS NULL AND readiness_json IS NULL FROM connections WHERE id=?1",params![lease.connection_id(),lease.generation()],|r|r.get(0)).unwrap();
    drop(c);
    assert!(old_retired);
    assert_eq!(
        record_same_capture(&store, &captured, || true).unwrap(),
        first
    );
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), after);
    assert!(store
        .try_native_media_read(captured.activation().origin(), None)
        .is_err());
}

#[test]
fn retired_original_capture_requires_current_original_authority_and_exact_retirement_tuple() {
    for change in [
        "operator-revoked",
        "source-revoked",
        "old-device-revoked",
        "old-generation",
        "retained-retired",
        "cancel",
        "unknown",
    ] {
        let (m, lease, _) = movement();
        let captured = capture(&m);
        retire_original(&m, &lease);
        {
            let c = m.f.store.connection().unwrap();
            let decision = crate::authority_writer_boundary::phase_ledger::commit::decision(
                &m.f.store, &c, &captured,
            )
            .unwrap();
            assert!(
                decision.reader,
                "exact retired setup did not enter Reader: {}",
                decision.reason
            );
            assert_eq!(decision.reason, "exact-retired-original");
        }
        let writer = m.f.controlled_conversation_writer(0);
        {
            let c = m.f.store.connection_with_authority(&writer).unwrap();
            match change {
                "operator-revoked" => {
                    c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&m.key])
                        .unwrap();
                }
                "source-revoked" => {
                    c.execute(
                        "UPDATE credentials SET revoked=1 WHERE id=?1",
                        [&m.f.keys[0]],
                    )
                    .unwrap();
                }
                "old-device-revoked" => {
                    c.execute("UPDATE devices SET revoked=1 WHERE id=(SELECT device_id FROM endpoints WHERE id=(SELECT endpoint_id FROM connections WHERE id=?1))",[lease.connection_id()]).unwrap();
                }
                "old-generation" => {
                    c.execute(
                        "UPDATE connections SET generation=generation+1 WHERE id=?1",
                        [lease.connection_id()],
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
                "cancel" => {
                    c.execute(
                        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                        [&captured.commit_ledger_key().record],
                    )
                    .unwrap();
                }
                "unknown" => {
                    c.execute(
                        "UPDATE operations SET state='unknown' WHERE record_id=?1",
                        [&captured.commit_ledger_key().record],
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
            "{change} entered retired Reader path"
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
        let refusal = m.f.store.record_native_replacement_committed(&captured);
        let unchanged = Store::inspect_snapshot(m.f.store.path()).unwrap() == before;
        drop(held);
        assert_eq!(refusal.unwrap_err().code, "authority-busy", "{change}");
        assert!(unchanged, "{change} changed metadata without Writer");
        assert!(
            m.f.store
                .try_native_media_read(&m.sources[0], None)
                .is_err(),
            "{change} revived old IO"
        );
    }
}

#[test]
fn retired_original_mapping_and_lifetime_cannot_be_forged_for_positive_capture() {
    let (m, lease, _) = movement();
    let captured = capture(&m);
    retire_original(&m, &lease);
    let writer = m.f.controlled_conversation_writer(0);
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    {
        let c = m.f.store.connection_with_authority(&writer).unwrap();
        assert!(c.execute("UPDATE connection_offers SET runtime_connection_id='foreign-retired-core' WHERE connection_id=?1",[lease.connection_id()]).is_err());
        assert!(c
            .execute(
                "UPDATE native_connection_lifetimes SET retired=0 WHERE operation_record_id=?1",
                [lease.record_id()]
            )
            .is_err());
        assert!(c.execute("UPDATE native_connection_lifetimes SET runtime_epoch='foreign-runtime' WHERE operation_record_id=?1",[lease.record_id()]).is_err());
        assert!(c.execute("UPDATE native_connection_lifetimes SET generation=generation+1 WHERE operation_record_id=?1",[lease.record_id()]).is_err());
    }
    drop(writer);
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    let c = m.f.store.connection().unwrap();
    let decision =
        crate::authority_writer_boundary::phase_ledger::commit::decision(&m.f.store, &c, &captured)
            .unwrap();
    assert!(
        decision.reader,
        "immutable foreign tuple attempts damaged valid exact receipt: {}",
        decision.reason
    );
    assert_eq!(decision.reason, "exact-retired-original");
    drop(c);
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[0], None)
        .is_err());
}

#[test]
fn known_commit_after_real_replacement_origin_retirement_preserves_new_healthy_epoch() {
    use crate::connection_control::tests::replacement_sources as fixtures;
    let (first, _, _) = movement();
    let initial_capture = capture(&first);
    record_same_capture(&first.f.store, &initial_capture, || true).unwrap();
    let replaced = first
        .f
        .store
        .project_native_replacement_committed(&initial_capture, &MetadataOnlyInstalled)
        .unwrap();
    assert_eq!(replaced.generation(), 2);
    let retained = first.sources[1].clone();
    let f = first.f;
    let endpoint = fixtures::endpoint(&f, vec!["audio".into()], "return-target");
    let key = fixtures::key(&f);
    let topology = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "move-established-replacement-again".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(replaced.connection_id().into()),
        expected_revision: topology,
        intent: connect_protocol::transitions::TransitionIntent::Replace {
            endpoint_id: Some(endpoint),
            candidate_connid: None,
            retained_connid: retained.connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let original = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let prepared = fixtures::allocation::prepared_target_for_scene_named(
        &[],
        (f, key, command, original, vec![replaced.clone(), retained]),
        "replacement-return",
    );
    let m = fixture_prepared(true, prepared);
    let captured = capture(&m);
    let absent = fixtures::candidate::Absent(
        replaced.runtime_connection_id().into(),
        replaced.generation(),
    );
    m.f.store
        .settle_native_replacement_cleanup(&replaced, &absent)
        .unwrap();
    let c = m.f.store.connection().unwrap();
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_admission_sources n JOIN native_replacement_lifetimes established ON established.operation_record_id=n.operation_record_id JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id JOIN connections v ON v.id=l.connection_id WHERE n.operation_record_id=?1 AND n.connection_id=?2 AND n.generation=2 AND l.retired=1 AND v.state='retired' AND v.generation=3)",params![replaced.offer_record_id(),replaced.connection_id()],|r|r.get(0)).unwrap();
    assert!(
        exact,
        "real replacement origin must use its actual UNION-view attribution"
    );
    let decision =
        crate::authority_writer_boundary::phase_ledger::commit::decision(&m.f.store, &c, &captured)
            .unwrap();
    assert!(
        decision.reader,
        "exact retired replacement origin refused Reader: {}",
        decision.reason
    );
    assert_eq!(decision.reason, "exact-retired-original");
    drop(c);
    assert!(m.f.store.try_native_media_read(&replaced, None).is_err());
    let healthy = m.sources[1].clone();
    let store = Arc::new(m.f.store);
    let (snapshot, mut watcher) = publication(&store, &healthy);
    let io = admit(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let result = record_same_capture(&store, &captured, || io.current() && epoch.current());
    let survived = io.current() && epoch.current();
    let original = snapshot.admit_original(stamp);
    let stamp_survived = original.as_ref().is_ok_and(|read| read.current());
    drop(original);
    drop(io);
    let queued_survived = epoch.current();
    watcher.shutdown().unwrap();
    assert!(
        survived && stamp_survived && queued_survived,
        "historical capture of second movement revoked fresh unrelated audio epoch"
    );
    result.unwrap();
    assert!(
        store.try_native_media_read(&replaced, None).is_err(),
        "second capture revived retired replacement IO"
    );
    let before = Store::inspect_snapshot(store.path()).unwrap();
    record_same_capture(&store, &captured, || true).unwrap();
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}
