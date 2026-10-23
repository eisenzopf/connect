//! Controlled Store custody, not a claim of SIP/native physical delivery.
use super::*;
use crate::{NativeDtmfBatch, NativeDtmfGatewayRecipient, NativeDtmfRecipient};
use connect_protocol::{
    native_dtmf::{SourceObservation, SubmissionDisposition as D},
    transitions::{OperationAction, OperationState},
};
use std::sync::Arc;

struct Signal {
    f: Fixture,
    native: Arc<crate::NativeMediaOrigin>,
    phone: Arc<crate::InitialTelephoneMediaOrigin>,
    request: PeerAuthorityRequest,
    lease: crate::NativeStreamLease,
}
fn fixture() -> Signal {
    let (f, native, phone) = super::sip_signal::fixture();
    let request = PeerAuthorityRequest {
        credentials: vec![f.keys[0].clone()],
        device: crate::PeerDeviceClaim {
            id: "device-0".into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: "native-key-0".into(),
        algorithm: "EdDSA".into(),
        command: PeerCommand::NativeStream(Intent {
            operation_id: "gateway-signal".into(),
            conversation_id: f.cid.clone(),
            session_id: "session".into(),
            connection_id: native.connection_id().into(),
            expected_revision: None,
            by_participant: Some(f.pids[0].clone()),
            command: Command::Dtmf,
            payload: json!({"by_participant":f.pids[0],"digits":"12#","duration_ms":100,"method":"info","source":"sip-info"}),
        }),
    };
    let proof = f.admit(&request);
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let lease = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap()
        .0;
    drop(proof);
    Signal {
        f,
        native,
        phone,
        request,
        lease,
    }
}
fn candidates(s: &Signal) -> Vec<NativeDtmfGatewayRecipient> {
    vec![NativeDtmfGatewayRecipient {
        origin: crate::NativeDtmfGatewayOrigin::Initial(s.phone.clone()),
        registration_id: "original-sip-registration".into(),
    }]
}
fn observed(s: &Signal) -> SourceObservation {
    SourceObservation {
        envelope_id: "actual-native-observation".into(),
        received_at: chrono::Utc::now(),
        stream_revision: 1,
        credential_revision: s.native.credential_epoch() + 1,
        signing_key_id: s.request.key_id.clone(),
        signing_algorithm: s.request.algorithm.clone(),
    }
}
fn freeze(s: &Signal) -> NativeDtmfBatch {
    let proof = s.f.admit(&s.request);
    let selected = s
        .f
        .store
        .select_native_dtmf_gateways_with_peer(&s.lease, s.native.clone(), candidates(s), &proof)
        .unwrap();
    assert_eq!(selected.len(), 1);
    s.f.store
        .prepare_native_mixed_dtmf_with_peer(
            &s.lease,
            s.native.clone(),
            vec![],
            selected,
            &proof,
            observed(s),
        )
        .unwrap()
}
fn reserve(s: &Signal, b: &NativeDtmfBatch) {
    let read =
        s.f.store
            .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
            .unwrap();
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-sip-schedule")
        .unwrap();
}
fn observe(record: &connect_protocol::transitions::OperationRecord) {
    if let Some(path) = std::env::var_os("CONNECT_DTMF_GATEWAY_OBSERVATIONS") {
        use std::io::Write;
        let path = std::path::PathBuf::from(path);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6");
        assert!(path.is_absolute() && path.starts_with(root));
        let mut bytes = serde_json::to_vec(record).unwrap();
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
    }
}

#[test]
fn native_gateway_dtmf_freezes_actual_call_without_native_lifetime_or_idle_permission() {
    let s = fixture();
    let b = freeze(&s);
    assert!(b.targets().is_empty());
    assert_eq!(b.gateway_targets().len(), 1);
    let target = &b.gateway_targets()[0];
    assert_eq!(target.origin().sip_call_id(), s.phone.sip_call_id());
    assert_eq!(
        target.observation().envelope_id,
        "actual-native-observation"
    );
    assert_eq!(target.intent().payload["source"], "sip-info");
    let c = s.f.store.connection().unwrap();
    let (calls, native_lifetimes):(u64,u64) = c.query_row("SELECT (SELECT count(*) FROM native_dtmf_gateways),(SELECT count(*) FROM native_connection_lifetimes WHERE operation_record_id=?1)",[s.phone.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((calls, native_lifetimes), (1, 0));
    let saved: String = c
        .query_row(
            "SELECT json_extract(target_json,'$.sip_call_id') FROM native_dtmf_gateways",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved, s.phone.sip_call_id());
    assert!(c
        .execute("UPDATE native_dtmf_gateways SET target_json='{}'", [])
        .is_err());
    assert!(c.execute("DELETE FROM native_dtmf_gateways", []).is_err());
    drop(c);
    let identity = s.f.store.revalidate(&s.f.keys[1]).unwrap().identity_id;
    assert!(s
        .f
        .store
        .try_acquire_authority(&[crate::AuthorityScope::identity(
            &identity,
            crate::AuthorityMode::Write
        )
        .unwrap()])
        .is_ok());
    let proof = s.f.admit(&s.request);
    assert_eq!(
        s.f.store
            .prepare_native_mixed_dtmf_with_peer(
                &s.lease,
                s.native.clone(),
                vec![],
                candidates(&s),
                &proof,
                observed(&s)
            )
            .unwrap_err()
            .code,
        "conflict"
    );
}

#[test]
fn native_gateway_dtmf_reserves_one_schedule_and_reports_only_local_submission() {
    let s = fixture();
    let b = freeze(&s);
    let target = &b.gateway_targets()[0];
    assert!(s
        .f
        .store
        .try_record_native_dtmf_gateway_outcome(target, D::LocalSubmitted)
        .is_err());
    let read = s.f.store.try_native_dtmf_gateway_read(target).unwrap();
    assert_eq!(read.phone_member().participant_id, s.f.pids[1]);
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-sip-schedule")
        .unwrap();
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-sip-schedule")
        .unwrap();
    assert_eq!(
        s.f.store
            .try_start_native_dtmf_gateway_write(&read, "another-schedule")
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(read);
    s.f.store
        .try_record_native_dtmf_gateway_outcome(target, D::LocalSubmitted)
        .unwrap();
    s.f.store
        .try_record_native_dtmf_gateway_outcome(target, D::LocalSubmitted)
        .unwrap();
    assert!(s
        .f
        .store
        .try_record_native_dtmf_gateway_outcome(target, D::OutcomeUnknown)
        .is_err());
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    observe(&done);
    assert_eq!(done.state, OperationState::Completed);
    let result = done.result.unwrap();
    assert_eq!(result["signal_source"], "uctp-native");
    assert_eq!(result["local_submission_only"], true);
    assert_eq!(
        result["recipients"][0]["connection_id"],
        s.phone.connection_id()
    );
    assert_eq!(result["recipients"][0]["frame_id"], "original-sip-schedule");
    assert!(s.f.store.try_native_dtmf_gateway_read(target).is_err());
}

#[test]
fn native_gateway_dtmf_mixed_batch_aggregates_every_native_and_sip_recipient() {
    let mut s = fixture();
    let third = super::dtmf_recipients::target(&mut s.f, 2, true);
    let proof = s.f.admit(&s.request);
    let b =
        s.f.store
            .prepare_native_mixed_dtmf_with_peer(
                &s.lease,
                s.native.clone(),
                vec![NativeDtmfRecipient {
                    origin: third,
                    registration_id: "actual-native-target".into(),
                }],
                candidates(&s),
                &proof,
                observed(&s),
            )
            .unwrap();
    drop(proof);
    assert_eq!(b.targets().len(), 1);
    assert_eq!(b.gateway_targets().len(), 1);
    let read = s.f.store.try_native_dtmf_read(&b.targets()[0]).unwrap();
    s.f.store
        .try_start_native_dtmf_write(&read, "original-native-frame")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_native_dtmf_outcome(&b.targets()[0], D::LocalSubmitted)
        .unwrap();
    assert!(s.f.store.finish_native_dtmf(&b).is_err());
    s.f.store
        .try_record_native_dtmf_gateway_outcome(&b.gateway_targets()[0], D::NotStarted)
        .unwrap();
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    observe(&done);
    assert_eq!(done.state, OperationState::Failed);
    let result = &done.problem.unwrap()["signal"];
    assert_eq!(result["recipients"].as_array().unwrap().len(), 2);
    assert_eq!(done.result, None);
}

#[test]
fn native_gateway_dtmf_queue_rechecks_independent_source_recipient_and_route_authority() {
    for fault in [
        "source-scope",
        "recipient-scope",
        "device",
        "endpoint",
        "route",
        "member",
        "source-key",
    ] {
        let s = fixture();
        let b = freeze(&s);
        let c = s.f.store.connection().unwrap();
        match fault {
            "source-scope" => c.execute(
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                [&s.f.keys[0]],
            ),
            "recipient-scope" => c.execute(
                "UPDATE credentials SET operations='[]' WHERE id=?1",
                [&s.f.keys[1]],
            ),
            "device" => c.execute(
                "UPDATE devices SET revoked=1 WHERE id='signal-phone-device'",
                [],
            ),
            "endpoint" => c.execute(
                "UPDATE endpoints SET revision=revision+1 WHERE id='signal-phone'",
                [],
            ),
            "route" => c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&s.f.admin]),
            "member" => c.execute(
                "UPDATE participants SET active=0 WHERE participant_id=?1",
                [&s.f.pids[1]],
            ),
            "source-key" => c.execute(
                "UPDATE peer_signing_keys SET revoked=1 WHERE key_id='native-key-0'",
                [],
            ),
            _ => unreachable!(),
        }
        .unwrap();
        drop(c);
        assert!(
            s.f.store
                .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
                .is_err(),
            "{fault}"
        );
    }
}

#[test]
fn native_gateway_dtmf_reservation_storage_failure_retries_metadata_without_another_attempt() {
    let s = fixture();
    let b = freeze(&s);
    let read =
        s.f.store
            .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
            .unwrap();
    s.f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_gateway_attempt BEFORE UPDATE OF attempted ON native_dtmf_gateways BEGIN SELECT RAISE(ABORT,'storage fixture'); END;").unwrap();
    assert!(s
        .f
        .store
        .try_start_native_dtmf_gateway_write(&read, "original-sip-schedule")
        .is_err());
    let c = s.f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT attempted FROM native_dtmf_gateways", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER fail_gateway_attempt")
        .unwrap();
    drop(c);
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-sip-schedule")
        .unwrap();
}

#[test]
fn native_gateway_dtmf_captured_outcome_survives_revocation_without_new_signal_authority() {
    let s = fixture();
    let b = freeze(&s);
    reserve(&s, &b);
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET revoked=1 WHERE id=?1",
            [&s.f.keys[1]],
        )
        .unwrap();
    assert!(s
        .f
        .store
        .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
        .is_err());
    s.f.store
        .try_record_native_dtmf_gateway_outcome(&b.gateway_targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&b).unwrap().state,
        OperationState::Completed
    );
}

#[test]
fn native_gateway_dtmf_restart_recovers_reserved_uncertainty_and_never_replays_a_schedule() {
    for attempted in [false, true] {
        let s = fixture();
        let b = freeze(&s);
        if attempted {
            reserve(&s, &b);
        }
        let path = s.f.store.path.clone();
        let key = s.f.keys[0].clone();
        drop(s.f.store);
        let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
        successor.recover_native_streams(256).unwrap();
        assert_eq!(successor.recover_native_dtmf(1).unwrap(), 1);
        assert_eq!(successor.recover_native_dtmf(1).unwrap(), 0);
        let status = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: "gateway-signal".into(),
                },
            )
            .unwrap();
        observe(&status);
        assert_eq!(
            status.state,
            if attempted {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        assert_eq!(
            status.problem.unwrap()["signal"]["recipients"][0]["disposition"],
            if attempted {
                "outcome_unknown"
            } else {
                "not_started"
            }
        );
        assert!(successor
            .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
            .is_err());
    }
}

#[test]
fn native_gateway_dtmf_selected_pair_does_not_require_unrelated_session_recipients() {
    let s = fixture();
    let source = s.f.store.revalidate(&s.f.keys[0]).unwrap().identity_id;
    let phone = s.f.store.revalidate(&s.f.keys[1]).unwrap().identity_id;
    let c = s.f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET recipients=?2 WHERE id=?1",
        params![s.f.keys[0], json!([source, phone]).to_string()],
    )
    .unwrap();
    c.execute(
        "UPDATE credentials SET recipients=?2 WHERE id=?1",
        params![s.f.keys[1], json!([source]).to_string()],
    )
    .unwrap();
    c.execute(
        "UPDATE credentials SET recipients=?2 WHERE id=?1",
        params![s.f.admin, json!([source, phone]).to_string()],
    )
    .unwrap();
    drop(c);
    let b = freeze(&s);
    assert_eq!(b.gateway_targets().len(), 1);
    assert!(s
        .f
        .store
        .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
        .is_ok());
}

#[test]
fn native_gateway_dtmf_cancellation_closes_unstarted_work_without_a_schedule() {
    let s = fixture();
    let b = freeze(&s);
    s.f.store
        .execute_operation_action(
            &s.f.keys[0],
            OperationAction::Cancel {
                operation_id: "cancel-gateway-signal".into(),
                target_operation_id: "gateway-signal".into(),
                reason: None,
            },
        )
        .unwrap();
    assert!(s
        .f
        .store
        .try_native_dtmf_gateway_read(&b.gateway_targets()[0])
        .is_err());
    s.f.store
        .try_record_native_dtmf_gateway_outcome(&b.gateway_targets()[0], D::NotStarted)
        .unwrap();
    let done = s.f.store.finish_native_dtmf(&b).unwrap();
    observe(&done);
    assert_eq!(done.state, OperationState::Cancelled);
    assert!(done.problem.unwrap()["signal"]["recipients"][0]["frame_id"].is_null());
}

#[test]
fn native_gateway_dtmf_freeze_and_outcomes_preserve_the_shared_journal_reservation() {
    let mut s = fixture();
    let saved = s.f.store.limits.journal_events;
    s.f.store.limits.journal_events = 1;
    let proof = s.f.admit(&s.request);
    assert_eq!(
        s.f.store
            .prepare_native_mixed_dtmf_with_peer(
                &s.lease,
                s.native.clone(),
                vec![],
                candidates(&s),
                &proof,
                observed(&s)
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    drop(proof);
    assert_eq!(s.f.count("native_dtmf_sources"), 0);
    assert_eq!(s.f.count("native_dtmf_gateways"), 0);
    s.f.store.limits.journal_events = saved;
    let b = freeze(&s);
    let consumed = s.f.count("journal");
    let mut found = false;
    for limit in consumed..consumed + 128 {
        s.f.store.limits.journal_events = limit;
        let c = s.f.store.connection().unwrap();
        if crate::conversations::capacity(&c, 0, 0, &s.f.store).unwrap() {
            found = true;
            break;
        }
    }
    assert!(found);
    let c = s.f.store.connection().unwrap();
    assert!(!crate::conversations::capacity(&c, 1, 0, &s.f.store).unwrap());
    drop(c);
    s.f.store
        .try_record_native_dtmf_gateway_outcome(&b.gateway_targets()[0], D::NotStarted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&b).unwrap().state,
        OperationState::Failed
    );
}

#[test]
fn native_gateway_dtmf_duplicate_registration_across_families_rolls_back_the_whole_set() {
    let mut s = fixture();
    let native = super::dtmf_recipients::target(&mut s.f, 2, true);
    let proof = s.f.admit(&s.request);
    assert_eq!(
        s.f.store
            .prepare_native_mixed_dtmf_with_peer(
                &s.lease,
                s.native.clone(),
                vec![NativeDtmfRecipient {
                    origin: native,
                    registration_id: "original-sip-registration".into()
                }],
                candidates(&s),
                &proof,
                observed(&s)
            )
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(proof);
    assert_eq!(s.f.count("native_dtmf_sources"), 0);
    assert_eq!(s.f.count("native_dtmf_recipients"), 0);
    assert_eq!(s.f.count("native_dtmf_gateways"), 0);
    assert_eq!(freeze(&s).gateway_targets().len(), 1);
}
