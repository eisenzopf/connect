use super::*;
use crate::authority_writer_boundary::phase_ledger::{Key, Stage};
use rusqlite::TransactionBehavior;

fn key_and_target(
    store: &Store,
    read: &crate::NativeReplacementAllocationRead,
    proof: &crate::PeerAuthorityGuard,
    request: &PeerAuthorityRequest,
) -> (Key, String) {
    let intent = read.target_intent().unwrap();
    let record = intent.offer["replacement_preparation"]["operation_record_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (runtime,reservation,digest):(String,String,String)=store.connection().unwrap().query_row(
        "SELECT runtime_epoch,reservation_id,plan_digest FROM native_replacements WHERE operation_record_id=?1",
        [&record],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    let context = read.source().context();
    let descriptor = json!({"intent":intent,"credential_id":request.credentials.last().unwrap(),"key_id":request.key_id,"algorithm":request.algorithm,"public_key":proof.key.public_key,"device_id":proof.key.device_id,"endpoint_id":context.target_endpoint_id,"endpoint_revision":context.target_endpoint_revision});
    let target = crate::provisioning::canonical_json(&descriptor).unwrap();
    (
        Key {
            record,
            phase: "allocate",
            attempt: format!("nra_{}", uuid::Uuid::new_v4()),
            effect: format!("nre_{}", uuid::Uuid::new_v4()),
            expected: target.clone(),
            runtime,
            reservation,
            digest,
            stage: Stage::Begin,
        },
        target,
    )
}
fn copy_key(key: &Key) -> Key {
    Key {
        record: key.record.clone(),
        phase: key.phase,
        attempt: key.attempt.clone(),
        effect: key.effect.clone(),
        expected: key.expected.clone(),
        runtime: key.runtime.clone(),
        reservation: key.reservation.clone(),
        digest: key.digest.clone(),
        stage: key.stage,
    }
}
fn birth(c: &rusqlite::Connection, key: &Key) -> rusqlite::Result<usize> {
    c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-allocate',?1,'none','prepared',?3)",params![key.effect,key.record,key.expected])
}

#[test]
fn resource_begin_nonce_rejects_partial_proof_descriptor_collision_and_unrelated_dml() {
    let (f, _, command, prepared, request, _) = prepared_target_scene();
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_allocation_read_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let (key, target) = key_and_target(&f.store, &read, &proof, &request);
    let mut binding = request.clone();
    binding.command = crate::PeerCommand::Binding;
    let partial = f.admit(&binding);
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert!(birth(&tx, &key).is_err());
    let boundary = &f.store.authority_writer_boundary;
    assert!(boundary
        .native_resource_begin_ledger(
            &f.store,
            &tx,
            &partial,
            read.source(),
            copy_key(&key),
            &target
        )
        .is_err());
    assert!(boundary
        .native_resource_begin_ledger(&f.store, &tx, &proof, read.source(), copy_key(&key), "{}")
        .is_err());
    let mut wrong_descriptor: serde_json::Value = serde_json::from_str(&target).unwrap();
    wrong_descriptor["device_id"] = json!("another-device");
    let wrong_target = crate::provisioning::canonical_json(&wrong_descriptor).unwrap();
    let mut wrong_key = copy_key(&key);
    wrong_key.expected = wrong_target.clone();
    assert!(boundary
        .native_resource_begin_ledger(
            &f.store,
            &tx,
            &proof,
            read.source(),
            wrong_key,
            &wrong_target
        )
        .is_err());
    let mut wrong = copy_key(&key);
    wrong.runtime.push_str("-foreign");
    assert!(boundary
        .native_resource_begin_ledger(&f.store, &tx, &proof, read.source(), wrong, &target)
        .is_err());
    let mut collision = copy_key(&key);
    collision.attempt = tx.query_row("SELECT attempt_id FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='prepare'",[&key.record],|row|row.get(0)).unwrap();
    assert!(boundary
        .native_resource_begin_ledger(&f.store, &tx, &proof, read.source(), collision, &target)
        .is_err());
    let permit = boundary
        .native_resource_begin_ledger(
            &f.store,
            &tx,
            &proof,
            read.source(),
            copy_key(&key),
            &target,
        )
        .unwrap();
    for sql in [
        "UPDATE operations SET state='unknown' WHERE record_id=?1",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        "UPDATE operations SET lease_expires_at=lease_expires_at+1 WHERE record_id=?1",
        "UPDATE operations SET expires_at=expires_at+1 WHERE record_id=?1",
    ] {
        assert!(tx.execute(sql, [&key.record]).is_err());
    }
    assert!(tx
        .execute(
            "UPDATE connections SET revision=revision+1 WHERE conversation_id=?1",
            [&command.conversation_id]
        )
        .is_err());
    birth(&tx, &key).unwrap();
    assert!(tx.execute("INSERT OR REPLACE INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-allocate',?1,'none','prepared',?3)",params![key.effect,key.record,key.expected]).is_err());
    assert!(tx.execute("UPDATE operation_effects SET state='confirmed',attempt=1,submission_generation=1 WHERE id=?1",[&key.effect]).is_err());
    tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'allocate',?2,?3)",params![key.record,key.attempt,key.effect]).unwrap();
    assert!(tx.execute("INSERT OR REPLACE INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'allocate',?2,?3)",params![key.record,key.attempt,key.effect]).is_err());
    tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='allocate'",[&key.record]).unwrap();
    tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&key.effect]).unwrap();
    tx.execute(
        "UPDATE operations SET revision=revision+1 WHERE record_id=?1",
        [&key.record],
    )
    .unwrap();
    assert!(tx
        .execute(
            "UPDATE operations SET revision=revision+1 WHERE record_id=?1",
            [&key.record]
        )
        .is_err());
    drop(permit);
    tx.rollback().unwrap();
    drop(c);
    drop(partial);
    drop(read);
    drop(proof);
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn resource_begin_enrollment_dependency_keeps_actual_subject_writer_admission_and_begin() {
    let (f, _, command, prepared, request, _) = prepared_target_scene();
    let subject = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    let record = match &request.command {
        crate::PeerCommand::NativeConnectionOffer(intent) => intent.offer
            ["replacement_preparation"]["operation_record_id"]
            .as_str()
            .unwrap(),
        _ => panic!(),
    };
    // A persisted transitive-link edge exercises the exact SQL footprint. It
    // issues no credential and cannot complete as enrollment.complete: this is
    // the defensive historical-row fallback, not an enrollment issuance path.
    let writer = f
        .store
        .try_acquire_authority(&[AuthorityScope::identity(&subject, AuthorityMode::Write).unwrap()])
        .unwrap();
    let c = f.store.connection_with_authority(&writer).unwrap();
    c.execute("INSERT INTO client_enrollments(id,tenant_id,issuer_identity_id,issuer_credential_id,identity_id,device_id,key_id,endpoint_id,approve_operation_id,complete_operation_id,request_json,intent_digest,challenge_hash,challenge_expires_at,credential_expires_at) SELECT 'historical-resource-dependency',o.tenant_id,o.owner_id,n.credential_id,?2,'historical-device','historical-key','historical-endpoint','historical-approval',o.operation_id,'{}',?3,?3,?4,?4 FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id WHERE o.record_id=?1",params![record,subject,"0".repeat(64),crate::now()+600]).unwrap();
    drop(c);
    drop(writer);
    let held = f
        .store
        .try_acquire_authority(&[AuthorityScope::identity(&subject, AuthorityMode::Read).unwrap()])
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let denied = f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_millis(20)));
    assert_eq!(denied.err().unwrap().code, "authority-busy");
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
    drop(held);
    let proof = f.admit(&request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[
            AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Write).unwrap(),
            AuthorityScope::identity(&subject, AuthorityMode::Write).unwrap()
        ]
    ));
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    drop(read);
    let allocated = Arc::new(attempt.allocated(expected).unwrap());
    let record = f
        .store
        .record_native_replacement_allocated_with_peer(&allocated, &proof)
        .unwrap();
    assert_eq!(
        record
            .effects
            .iter()
            .filter(|effect| effect.provider == "native-replacement-allocate")
            .count(),
        1
    );
    drop(proof);
    let mut start_request = request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut start_request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    let proof = f.admit(&start_request);
    assert!(f.store.authority_covers(
        Store::peer_scopes(&proof),
        &[
            AuthorityScope::conversation(&command.conversation_id, AuthorityMode::Write).unwrap(),
            AuthorityScope::identity(&subject, AuthorityMode::Write).unwrap()
        ]
    ));
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &start_request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(
            allocated,
            &start_request.credentials[0],
            &proof,
            read,
        )
        .unwrap();
    let expected = attempt.expected_resources().clone();
    drop(read);
    let record = f
        .store
        .record_native_replacement_started_with_peer(&attempt.started(expected).unwrap(), &proof)
        .unwrap();
    assert_eq!(
        record
            .effects
            .iter()
            .filter(|effect| effect.provider == "native-replacement-start")
            .count(),
        1
    );
}

#[test]
fn resource_begin_rechecks_original_peer_expiry_before_effect() {
    let (f, _, _, prepared, request, _) = prepared_target_scene();
    let mut proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_allocation_read_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let until = crate::now() + 1;
    proof.limit_expiry(until).unwrap();
    let stop = Instant::now() + Duration::from_secs(3);
    while crate::now() < until && Instant::now() < stop {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(crate::now() >= until);
    assert_eq!(
        f.store
            .begin_native_replacement_allocation_with_read(
                &prepared,
                &request.credentials[0],
                &proof,
                read
            )
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), before);
}

#[test]
fn immutable_allocation_unknown_receipt_drains_real_io_and_replays_without_new_mutation() {
    let (f, _, _, prepared, request, sources) = prepared_target_scene();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let capture = Arc::new(attempt.uncertain());
    drop(read);
    drop(proof);
    let store = Arc::new(f.store);
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let weak = Arc::downgrade(&store);
    let source = sources[0].clone();
    let snapshot = store
        .register_native_media_snapshot_for(&sources[0], None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&source, None)
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let io = loop {
        match snapshot.admit() {
            Ok(io) => break io,
            Err(error) if error.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(error) => panic!("actual original snapshot refused: {}", error.code),
        }
    };
    let epoch = io.packet_epoch().unwrap();
    let (entered, waited) = std::sync::mpsc::sync_channel(1);
    let task_store = store.clone();
    let task_capture = capture.clone();
    let worker = std::thread::spawn(move || {
        crate::authority_fences::with_waited_lock_probe(
            move || {
                let _ = entered.send(());
            },
            || task_store.native_replacement_allocation_capture_uncertain(&task_capture),
        )
    });
    let observed = waited.recv_timeout(Duration::from_secs(2));
    let revoked = !epoch.current();
    drop(io);
    drop(snapshot);
    let result = worker.join();
    watcher.shutdown().unwrap();
    assert!(
        observed.is_ok(),
        "actual Unknown Writer never reached retained IO exclusion"
    );
    assert!(revoked);
    result.unwrap().unwrap();
    let before = Store::inspect_snapshot(store.path()).unwrap();
    store
        .native_replacement_allocation_capture_uncertain(&capture)
        .unwrap();
    assert_eq!(Store::inspect_snapshot(store.path()).unwrap(), before);
}
