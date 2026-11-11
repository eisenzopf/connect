//! Real native PCM snapshot custody around controlled exact Data queue receipts.
//! No decoder, native queue invocation or remote delivery is inferred here.
use super::*;
use crate::{AuthorityMode, AuthorityScope, NativeMediaRead, NativeMediaSnapshot};
use std::{sync::Arc, time::Instant};

fn prepared() -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    crate::NativeStreamLease,
) {
    let (f, origin, lease, _) = prepared_with_request();
    (f, origin, lease)
}
fn prepared_with_request() -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    crate::NativeStreamLease,
    PeerAuthorityRequest,
) {
    let (f, mut request, _, origin) =
        parent_permissions_origin_filtered_media(false, true, false, true, true, &[], &[], true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.operation_id = "publish-application".into();
    wire.expected_revision = None;
    wire.by_participant = Some(f.pids[0].clone());
    wire.command = Command::Data;
    wire.payload = json!({"from":f.pids[0],"msg_id":"application-message","label":"application","to":[f.pids[1]],"content_type":"application/json","body":"{\"hello\":true}"});
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    drop(proof);
    (f, origin, lease, request)
}

fn publication(
    store: &Arc<Store>,
    origin: &Arc<crate::NativeMediaOrigin>,
) -> (
    NativeMediaSnapshot,
    crate::media_authorization::OwnedWatcher,
) {
    let weak = Arc::downgrade(store);
    let source = origin.clone();
    let snapshot = store
        .register_native_media_snapshot_for(origin, None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&source, None)
        })
        .unwrap();
    let watcher = store.start_media_authorization_watcher().unwrap();
    (snapshot, watcher)
}
fn current(snapshot: &NativeMediaSnapshot) -> NativeMediaRead {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match snapshot.admit() {
            Ok(io) => return io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("original PCM setup refused: {}", error.code),
        }
    }
}

#[test]
fn queued_data_completion_and_exact_replay_preserve_actual_pcm_io_and_queued_epoch() {
    let (f, origin, lease) = prepared();
    let receipt = lease.queued().unwrap();
    let store = Arc::new(f.store);
    let (snapshot, mut watcher) = publication(&store, &origin);
    let io = current(&snapshot);
    assert!(io.target().offer["streams_offered"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["kind"] == "audio"));
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let first = store.complete_native_stream(&receipt);
    let survived = epoch.current();
    let original_admission = snapshot.admit_original(stamp);
    drop(io);
    // A known replay must not even reserve SQLite's write transaction.
    let external = rusqlite::Connection::open(store.path()).unwrap();
    external.execute_batch("BEGIN IMMEDIATE").unwrap();
    let replay = store.complete_native_stream(&receipt);
    let original_still_current =
        original_admission.as_ref().is_ok_and(|io| io.current()) && epoch.current();
    external.execute_batch("ROLLBACK").unwrap();
    drop(original_admission);
    watcher.shutdown().unwrap();
    assert!(survived, "Data completion revoked the original queued PCM epoch despite changing only its own successful ledger");
    assert!(
        original_still_current,
        "exact recorded replay changed the original queued PCM grant"
    );
    let first = first.unwrap();
    assert_eq!(
        first.state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(replay.unwrap(), first);
    assert_eq!(first.effects.len(), 1);
}

#[test]
fn real_credential_writer_drains_pcm_io_and_cannot_revive_original_queued_epoch() {
    let (f, origin, lease) = prepared();
    let credential = f.keys[0].clone();
    let actor = f.store.revalidate(&credential).unwrap().identity_id;
    let store = Arc::new(f.store);
    store
        .complete_native_stream(&lease.queued().unwrap())
        .unwrap();
    let (snapshot, mut watcher) = publication(&store, &origin);
    let io = current(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let scope = AuthorityScope::identity(&actor, AuthorityMode::Write).unwrap();
    let refused = store.try_acquire_authority(&[scope.clone()]);
    let code = refused.err().unwrap().code;
    let invalidated = !epoch.current();
    drop(io);
    let writer = store
        .acquire_authority(&[scope], &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    store
        .connection_with_authority(&writer)
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&credential],
        )
        .unwrap();
    drop(writer);
    let old_refused = snapshot.admit_original(stamp).is_err();
    watcher.shutdown().unwrap();
    assert_eq!(code, "authority-busy");
    assert!(invalidated && old_refused && !epoch.current());
}

#[test]
fn uncertain_data_outcome_keeps_the_real_writer_boundary() {
    let (f, origin, lease) = prepared();
    let store = Arc::new(f.store);
    let (snapshot, mut watcher) = publication(&store, &origin);
    let io = current(&snapshot);
    let epoch = io.packet_epoch().unwrap();
    let refused = store.native_stream_attempt_uncertain(&lease);
    let invalidated = !epoch.current();
    drop(io);
    let recorded = store.native_stream_attempt_uncertain(&lease);
    watcher.shutdown().unwrap();
    assert_eq!(refused.unwrap_err().code, "authority-busy");
    assert!(invalidated);
    assert_eq!(
        recorded.unwrap().state,
        connect_protocol::transitions::OperationState::Unknown
    );
}

#[test]
fn queued_data_lifecycle_witness_refusal_uses_the_existing_historical_writer() {
    let (f, _origin, lease) = prepared();
    let receipt = lease.queued().unwrap();
    let record = receipt_record(&f);
    f.change_fixture().execute("UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=(SELECT parent_record_id FROM native_stream_operations WHERE operation_record_id=?1)", [&record]).unwrap();
    let required = {
        let c = f.store.connection().unwrap();
        crate::authority_admission::conversation_scopes(&c, &f.cid, AuthorityMode::Read).unwrap()
    };
    let read = f
        .store
        .acquire_authority(&required, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    let refused = f.store.complete_native_stream(&receipt);
    drop(read);
    assert_eq!(refused.unwrap_err().code, "authority-busy");
    let completed = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(
        completed.state,
        connect_protocol::transitions::OperationState::Completed
    );
    let c = f.store.connection().unwrap();
    assert!(c
        .query_row("SELECT retired FROM native_connection_lifetimes", [], |r| r
            .get::<_, bool>(0))
        .unwrap());
}

fn receipt_record(f: &Fixture) -> String {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT record_id FROM operations WHERE operation_id='publish-application'",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

fn ledger_key(f: &Fixture) -> crate::authority_writer_boundary::phase_ledger::Key {
    let c = f.store.connection().unwrap();
    let (record, parent, attempt, effect, runtime, digest, connection, message): (String,String,String,String,String,String,String,String) = c.query_row(
        "SELECT o.record_id,n.parent_record_id,n.attempt_id,n.effect_id,n.attempt_runtime,o.digest,o.connection_id,d.message_id FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN native_data_messages d ON d.operation_record_id=o.record_id WHERE o.operation_id='publish-application'", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)),
    ).unwrap();
    crate::authority_writer_boundary::phase_ledger::Key {
        record,
        phase: "data-queued",
        attempt,
        effect,
        runtime,
        reservation: parent,
        digest,
        expected:
            json!({"connection_id":connection,"message_id":message,"core_queue_admission":true})
                .to_string(),
        stage: crate::authority_writer_boundary::phase_ledger::Stage::Confirm,
    }
}
fn readers(f: &Fixture) -> Arc<crate::AuthorityGuardSet> {
    let scopes = {
        let c = f.store.connection().unwrap();
        crate::authority_admission::conversation_scopes(&c, &f.cid, AuthorityMode::Read).unwrap()
    };
    f.store
        .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap()
}

#[test]
fn queued_positive_permit_rejects_unrelated_sql_and_exact_outcome_rewrites() {
    let (f, _origin, _lease) = prepared();
    let key = ledger_key(&f);
    let record = key.record.clone();
    let effect = key.effect.clone();
    let proof = readers(&f);
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let permit = f
        .store
        .authority_writer_boundary
        .native_queued_capture_ledger(&f.store, &tx, &proof, key)
        .unwrap();
    // No naked SQL mutation receives authority from the presence of a real
    // Data receipt permit. Unique victims and other same-owner rows stay fenced.
    for sql in [
        "UPDATE credentials SET revoked=1",
        "UPDATE connections SET generation=generation+1",
        "UPDATE sessions SET state='ended'",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        "UPDATE operations SET expires_at=expires_at+1 WHERE record_id=?1",
        "UPDATE operations SET lease_owner='foreign-attempt' WHERE record_id=?1",
        "UPDATE operations SET owner_id='foreign-owner' WHERE record_id=?1",
        "UPDATE operations SET state='unknown' WHERE record_id=?1",
        "UPDATE operations SET state='completed',revision=revision+2,result_json='{}' WHERE record_id=?1",
        "UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1",
        "INSERT OR REPLACE INTO operation_effects SELECT * FROM operation_effects WHERE operation_record_id=?1",
        "INSERT OR REPLACE INTO operations SELECT * FROM operations WHERE record_id=?1",
    ] {
        let result = if sql.contains("?1") { tx.execute(sql, [&record]) } else { tx.execute(sql, []) };
        assert!(result.is_err(), "positive Data permit authorized unrelated SQL: {sql}");
    }
    assert_eq!(
        tx.query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [&record],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "preparing"
    );
    assert_eq!(
        tx.query_row(
            "SELECT state FROM operation_effects WHERE id=?1",
            [&effect],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "submitting"
    );
    drop(permit);
    tx.rollback().unwrap();
}

#[test]
fn queued_positive_permit_is_bound_to_actual_unit_proof_and_rollback_nonce() {
    let (f, _origin, _lease) = prepared();
    let (foreign, _, _) = prepared();
    let foreign_proof = readers(&foreign);
    let proof = readers(&f);
    let key = ledger_key(&f);
    let wrong_key = ledger_key(&f);
    let old_key = ledger_key(&f);
    let next_key = ledger_key(&f);
    let effect = key.effect.clone();
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    assert!(f
        .store
        .authority_writer_boundary
        .native_queued_capture_ledger(&f.store, &tx, &foreign_proof, key)
        .is_err());
    assert!(foreign
        .store
        .authority_writer_boundary
        .native_queued_capture_ledger(&foreign.store, &tx, &proof, wrong_key)
        .is_err());
    let old = f
        .store
        .authority_writer_boundary
        .native_queued_capture_ledger(&f.store, &tx, &proof, old_key)
        .unwrap();
    tx.execute_batch("ROLLBACK; BEGIN IMMEDIATE").unwrap();
    let next = f
        .store
        .authority_writer_boundary
        .native_queued_capture_ledger(&f.store, &tx, &proof, next_key)
        .unwrap();
    drop(old); // Its stale nonce cannot clear the new same-Unit permit.
    assert_eq!(
        tx.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [&effect]
        )
        .unwrap(),
        1
    );
    drop(next);
    tx.rollback().unwrap();
    drop(c);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT state FROM operation_effects WHERE id=?1",
                [&effect],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "submitting"
    );
}

#[test]
fn queued_completion_releases_only_its_slot_and_never_restarts_the_original_effect() {
    let (f, _, lease, original) = prepared_with_request();
    let receipt = lease.queued().unwrap();
    let completed = f.store.complete_native_stream(&receipt).unwrap();
    let mut next = original.clone();
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.operation_id = "publish-next-message".into();
    wire.payload["msg_id"] = json!("next-message");
    let proof = admit(&f, &next).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (next_attempt, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    drop(proof);
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), completed);
    let c = f.store.connection().unwrap();
    let slot: (bool, bool) = c.query_row(
        "SELECT old.closed,new.closed FROM native_stream_operations old JOIN operations o ON o.record_id=old.operation_record_id JOIN native_stream_operations new ON new.operation_record_id=?1 WHERE o.operation_id='publish-application'", [next_attempt.record_id()], |r|Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(
        slot,
        (true, false),
        "known replay must not release a newer operation's slot"
    );
    drop(c);
    let proof = admit(&f, &original).unwrap();
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    drop(proof);
    f.store
        .complete_native_stream(&next_attempt.queued().unwrap())
        .unwrap();
    assert_eq!(f.count("native_data_messages"), 2);
}

#[test]
fn queued_completion_unblocks_credential_rotation_only_under_its_real_current_writer() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use std::collections::BTreeMap;
    let (f, origin, lease, mut binding) = prepared_with_request();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let next = f
        .store
        .issue(
            &identity,
            Scope::collaboration(vec!["*".into()]),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    let key = f
        .store
        .resolve_peer_signing_key(&f.keys[0], "tnt_default", &binding.key_id, "EdDSA")
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-next-data-capture-credential".into(),
                identity_id: identity,
                credential_id: next.clone(),
                device_id: binding.device.id.clone(),
                key_id: binding.key_id.clone(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: URL_SAFE_NO_PAD.encode(&key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(0),
            },
        )
        .unwrap();
    binding.credentials.push(next);
    binding.command = PeerCommand::Binding;
    let store = Arc::new(f.store);
    let (snapshot, mut watcher) = publication(&store, &origin);
    let io = current(&snapshot);
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let expires = io.expires_at();
    let pending = store.admit_native_credential_refresh(
        &binding,
        &[origin.clone()],
        &AuthorityWait::new(Duration::from_millis(20)),
    );
    let pending_code = pending.err().unwrap().code;
    let pending_unchanged = epoch.current();
    let completed = store.complete_native_stream(&lease.queued().unwrap());
    let capture_unchanged = epoch.current() && io.current() && io.expires_at() == expires;
    let busy = store.admit_native_credential_refresh(
        &binding,
        &[origin.clone()],
        &AuthorityWait::new(Duration::from_millis(20)),
    );
    let busy_code = busy.err().unwrap().code;
    let writer_invalidated = !epoch.current();
    drop(io);
    let (writer, plan) = store
        .admit_native_credential_refresh(
            &binding,
            &[origin.clone()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    let attempt = store
        .prepare_native_credential_refresh(&plan, &writer, "original-data-rotation")
        .unwrap();
    store
        .begin_native_credential_refresh(&attempt, &plan, &writer)
        .unwrap();
    let next_origin = plan.targets()[0].next().clone();
    let receipt = attempt
        .applied(BTreeMap::from([(
            origin.offer_record_id().to_owned(),
            plan.targets()[0].expected_credential_revision() + 1,
        )]))
        .unwrap();
    store
        .record_native_credential_refresh_applied(&receipt)
        .unwrap();
    drop(receipt);
    drop(plan);
    drop(writer);
    let original_refused = snapshot.admit_original(stamp).is_err()
        && store.try_native_media_read(&origin, None).is_err();
    let next_admitted = store.try_native_media_read(&next_origin, None).is_ok();
    watcher.shutdown().unwrap();
    assert_eq!(pending_code, "authority-busy");
    assert!(
        pending_unchanged && capture_unchanged,
        "settling the pending Data slot must not revoke unchanged PCM authority"
    );
    assert_eq!(
        completed.unwrap().state,
        connect_protocol::transitions::OperationState::Completed
    );
    assert_eq!(busy_code, "authority-busy");
    assert!(writer_invalidated && original_refused && next_admitted);
}
