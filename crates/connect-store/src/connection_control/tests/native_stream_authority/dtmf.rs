//! Controlled durable source custody. No physical DTMF IO or playback is claimed.
use super::*;
use connect_protocol::transitions::{OperationAction, OperationState};

fn fixture(granted: bool, completed: bool) -> (Fixture, PeerAuthorityRequest) {
    let extra = if granted { &["dtmf.send"][..] } else { &[] };
    let (f, mut request, _, _) =
        parent_permissions_origin_extra(true, completed, false, false, true, extra);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Dtmf;
    wire.operation_id = "native-dtmf".into();
    wire.expected_revision = None;
    wire.payload =
        json!({"digits":"12aB*#", "duration_ms":100, "method":"info", "future":{"keep":true}});
    (f, request)
}
fn accept(
    f: &Fixture,
    request: &PeerAuthorityRequest,
) -> connect_protocol::transitions::OperationRecord {
    let proof = admit(f, request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
}
fn begin(f: &Fixture, request: &PeerAuthorityRequest) -> crate::NativeStreamLease {
    let proof = admit(f, request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap()
        .0
}
fn cancel_dtmf(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-native-dtmf".into(),
                target_operation_id: "native-dtmf".into(),
                reason: None,
            },
        )
        .unwrap()
}
fn status(f: &Fixture) -> connect_protocol::transitions::OperationRecord {
    f.store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: "native-dtmf".into(),
            },
        )
        .unwrap()
}

#[test]
fn native_dtmf_needs_its_named_signal_permission_and_a_current_completed_physical_parent() {
    for (granted, completed) in [(false, true), (true, false)] {
        let (f, request) = fixture(granted, completed);
        let before = (
            f.count("operations"),
            f.count("journal"),
            f.count("operation_effects"),
        );
        assert_eq!(admit(&f, &request).unwrap_err().code, "forbidden");
        assert_eq!(
            (
                f.count("operations"),
                f.count("journal"),
                f.count("operation_effects")
            ),
            before
        );
    }
    let (f, request) = fixture(true, true);
    let proof = admit(&f, &request).unwrap();
    let source = f
        .store
        .project_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(source.connection.connection.participant_id, f.pids[0]);
    assert_eq!(source.stream_revision, 1);
    assert_eq!(f.count("operation_effects"), 1);
}

#[test]
fn native_dtmf_preserves_exact_intent_and_replay_never_creates_another_attempt() {
    let (f, request) = fixture(true, true);
    let accepted = accept(&f, &request);
    assert_eq!(accepted.state, OperationState::Accepted);
    assert_eq!(accepted.command_type, "dtmf.send");
    assert_eq!(accepted.expected_revision, None);
    assert!(accepted.effects.is_empty());
    assert_eq!(accept(&f, &request), accepted);
    let original: serde_json::Value = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT intent_json FROM operations WHERE record_id=?1",
            [&accepted.record_id],
            |r| r.get::<_, String>(0),
        )
        .map(|s| serde_json::from_str(&s).unwrap())
        .unwrap();
    let PeerCommand::NativeStream(wire) = &request.command else {
        panic!()
    };
    assert_eq!(original, serde_json::to_value(wire).unwrap());
    for (field, value) in [
        ("digits", json!("12AB*#")),
        ("duration_ms", json!(101)),
        ("method", json!("rfc4733")),
        ("future", json!({"keep":false})),
    ] {
        let mut changed = request.clone();
        let PeerCommand::NativeStream(wire) = &mut changed.command else {
            panic!()
        };
        wire.payload[field] = value;
        let proof = admit(&f, &changed).unwrap();
        assert_eq!(
            f.store
                .accept_native_stream_with_peer(&f.keys[0], &proof)
                .unwrap_err()
                .code,
            "conflict"
        );
    }
    let mut changed = request.clone();
    let PeerCommand::NativeStream(wire) = &mut changed.command else {
        panic!()
    };
    wire.expected_revision = Some(1);
    let proof = admit(&f, &changed).unwrap();
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(proof);
    let lease = begin(&f, &request);
    let started = status(&f);
    assert_eq!(started.state, OperationState::Preparing);
    assert_eq!(started.effects.len(), 1);
    assert_eq!(started.effects[0].id, lease.effect_id());
    assert_eq!(accept(&f, &request), started);
    let proof = admit(&f, &request).unwrap();
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), 2);
    assert_eq!(f.count("messages"), 0);
}

#[test]
fn native_dtmf_stale_revision_fails_durably_before_any_signal_attempt() {
    let (f, mut request) = fixture(true, true);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.expected_revision = Some(2);
    let failed = accept(&f, &request);
    assert_eq!(failed.state, OperationState::Failed);
    assert_eq!(failed.problem.as_ref().unwrap()["code"], "conflict");
    assert!(failed.effects.is_empty());
    assert_eq!(accept(&f, &request), failed);
    assert_eq!(f.count("operation_effects"), 1);
}

#[test]
fn native_dtmf_cancellation_preserves_the_actual_attempt_cutoff_and_reserved_identity() {
    for started in [false, true] {
        let (f, request) = fixture(true, true);
        accept(&f, &request);
        let lease = started.then(|| begin(&f, &request));
        let decision = cancel_dtmf(&f);
        assert_eq!(cancel_dtmf(&f), decision);
        let saved = status(&f);
        assert_eq!(
            saved.state,
            if started {
                OperationState::Unknown
            } else {
                OperationState::Cancelled
            }
        );
        assert!(saved.cancel_requested);
        if let Some(lease) = lease {
            assert_eq!(saved.effects[0].id, lease.effect_id());
            assert_eq!(
                f.store.native_stream_attempt_uncertain(&lease).unwrap(),
                saved
            );
        } else {
            assert!(saved.effects.is_empty());
        }
        assert_eq!(accept(&f, &request), saved);
        let proof = admit(&f, &request).unwrap();
        assert!(f
            .store
            .begin_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .is_none());
        drop(proof);
        let mut next = request.clone();
        let PeerCommand::NativeStream(wire) = &mut next.command else {
            panic!()
        };
        wire.operation_id = "native-dtmf-next".into();
        let proof = admit(&f, &next).unwrap();
        let next = f.store.accept_native_stream_with_peer(&f.keys[0], &proof);
        if started {
            assert_eq!(next.unwrap_err().code, "conflict");
        } else {
            assert_eq!(next.unwrap().state, OperationState::Accepted);
        }
    }
}

#[test]
fn native_dtmf_successor_recovers_original_uncertainty_without_automatic_resubmission() {
    let (f, request) = fixture(true, true);
    let lease = begin(&f, &request);
    let started = status(&f);
    let old_epoch = f.store.runtime_epoch().to_owned();
    let path = f.store.path.clone();
    let credential = f.keys[0].clone();
    drop(f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert_ne!(old_epoch, successor.runtime_epoch());
    assert_eq!(successor.recover_native_streams(256).unwrap(), 1);
    assert_eq!(successor.recover_native_streams(256).unwrap(), 0);
    let saved = successor
        .execute_operation_action(
            &credential,
            OperationAction::Get {
                operation_id: "native-dtmf".into(),
            },
        )
        .unwrap();
    assert_eq!(saved.state, OperationState::Unknown);
    assert_eq!(saved.record_id, started.record_id);
    assert_eq!(saved.expires_at, started.expires_at);
    assert_eq!(saved.effects[0].id, lease.effect_id());
    let proof = successor
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(
        successor
            .accept_native_stream_with_peer(&credential, &proof)
            .unwrap(),
        saved
    );
    assert!(successor
        .begin_native_stream_with_peer(&credential, &proof)
        .unwrap()
        .is_none());
    assert!(successor
        .validate_native_stream_attempt(&credential, &proof, &lease)
        .is_err());
}

#[test]
fn native_dtmf_definite_no_effect_failure_releases_control_without_a_delivery_claim() {
    let (f, request) = fixture(true, true);
    let lease = begin(&f, &request);
    let failed = f
        .store
        .fail_native_stream_without_effect(
            lease,
            Error::new(
                "unsupported-media-profile",
                "No authorized signal recipient",
            ),
        )
        .unwrap();
    assert_eq!(failed.state, OperationState::Failed);
    assert!(failed.result.is_none());
    assert_eq!(accept(&f, &request), failed);
    let mut next = request.clone();
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.operation_id = "native-dtmf-replacement".into();
    assert_eq!(accept(&f, &next).state, OperationState::Accepted);
}

#[test]
fn native_dtmf_attempt_cannot_be_completed_with_a_media_or_descriptor_receipt() {
    for kind in ["streams", "quality", "descriptor", "data"] {
        let (f, request) = fixture(true, true);
        let lease = begin(&f, &request);
        let receipt = match kind {
            "streams" => lease.committed(2, true),
            "quality" => lease.quality_configured(400),
            "descriptor" => lease.advertisement_received(),
            _ => lease.queued(),
        };
        assert!(receipt.is_err());
        assert_eq!(status(&f).state, OperationState::Preparing);
        assert!(status(&f).result.is_none());
    }
}

#[test]
fn native_dtmf_acceptance_cannot_lend_old_authority_to_a_later_signal_attempt() {
    for loss in [
        "permission",
        "credential",
        "key",
        "device",
        "endpoint",
        "member",
        "lifetime",
    ] {
        let (f, request) = fixture(true, true);
        let accepted = accept(&f, &request);
        let proof = admit(&f, &request).unwrap();
        // Controlled current Store facts. Actual transport/key-loss IO belongs
        // to the production integration tests, not this source-custody fixture.
        let c = f.store.connection().unwrap();
        match loss {
            "permission" => c.execute("UPDATE credentials SET operations='[\"connect:control\",\"connection.offer\"]' WHERE id=?1", [&f.keys[0]]),
            "credential" => c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]]),
            "key" => c.execute("UPDATE peer_signing_keys SET revoked=1 WHERE key_id=?1", [&request.key_id]),
            "device" => c.execute("UPDATE devices SET revoked=1 WHERE id=?1", [&request.device.id]),
            "endpoint" => c.execute("UPDATE endpoints SET verification_state='revoked' WHERE id=?1", [&f.endpoints[0]]),
            "member" => c.execute("UPDATE participants SET active=0 WHERE participant_id=?1", [&f.pids[0]]),
            _ => c.execute("UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id='connection-native'", []),
        }.unwrap();
        drop(c);
        assert!(
            f.store
                .begin_native_stream_with_peer(&f.keys[0], &proof)
                .is_err(),
            "{loss}"
        );
        let c = f.store.connection().unwrap();
        let state: String = c
            .query_row(
                "SELECT state FROM operations WHERE record_id=?1",
                [&accepted.record_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "accepted");
        drop(c);
        assert_eq!(f.count("operation_effects"), 1);
    }
}

#[test]
fn native_dtmf_source_schema_rejects_malformed_sequences_even_without_rust_validation() {
    let (f, request) = fixture(true, true);
    let original = accept(&f, &request);
    let proof = admit(&f, &request).unwrap();
    f.store
        .refuse_native_stream_before_effect_with_peer(
            &f.keys[0],
            &proof,
            Error::new("unsupported-media-profile", "Controlled no-effect refusal"),
        )
        .unwrap();
    drop(proof);
    let PeerCommand::NativeStream(wire) = &request.command else {
        panic!()
    };
    let mut c = f.store.connection().unwrap();
    for payload in [
        json!({"digits":"1x", "duration_ms":100, "method":"info"}),
        json!({"digits":"1\u{0}x", "duration_ms":100, "method":"info"}),
        json!({"digits":"１２", "duration_ms":100, "method":"info"}),
        json!({"digits":"", "duration_ms":100, "method":"info"}),
        json!({"digits":"1".repeat(33), "duration_ms":100, "method":"info"}),
        json!({"digits":"1", "duration_ms":"100", "method":"info"}),
        json!({"digits":"1", "duration_ms":99, "method":"info"}),
        json!({"digits":"1", "duration_ms":100, "method":"inband"}),
        json!({"digits":"12345", "duration_ms":6000, "method":"rfc4733"}),
        json!({"digits":"1", "duration_ms":100, "method":"info", "future":"x".repeat(32768)}),
        wire.payload.clone(),
    ] {
        let valid = payload == wire.payload;
        let mut forged = wire.clone();
        forged.operation_id = "forged-dtmf".into();
        forged.payload = payload;
        let tx = c.transaction().unwrap();
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at) SELECT 'forged-dtmf-record',tenant_id,owner_id,'forged-dtmf',digest,command_type,conversation_id,session_id,connection_id,expected_revision,?2,'accepted',created_at,updated_at,expires_at FROM operations WHERE record_id=?1", params![original.record_id,serde_json::to_string(&forged).unwrap()]).unwrap();
        let inserted = tx.execute("INSERT INTO native_stream_operations(operation_record_id,parent_record_id,credential_id,key_id,device_id,participant_id,membership_epoch,generation,resources_json,source_expires_at) SELECT 'forged-dtmf-record',parent_record_id,credential_id,key_id,device_id,participant_id,membership_epoch,generation,resources_json,source_expires_at FROM native_stream_operations WHERE operation_record_id=?1", [&original.record_id]);
        if valid {
            assert_eq!(inserted.unwrap(), 1);
        } else {
            let error = inserted.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("native stream source binding mismatch"),
                "{error}"
            );
        }
        tx.rollback().unwrap();
    }
}
