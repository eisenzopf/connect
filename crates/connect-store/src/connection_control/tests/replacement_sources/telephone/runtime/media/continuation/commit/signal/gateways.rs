//! Controlled committed-SIP custody only; actual delivery is tested separately.
use super::*;
use crate::{
    NativeDtmfBatch, NativeDtmfGatewayRecipient, PeerAuthorityRequest, PeerCommand, SipSignalOrigin,
};
use connect_protocol::native_dtmf::{SourceObservation, SubmissionDisposition as D};

struct Signal {
    f: Fixture,
    phone: Arc<crate::TelephoneMediaOrigin>,
    native: Arc<crate::NativeMediaOrigin>,
    route: String,
    request: PeerAuthorityRequest,
    lease: crate::NativeStreamLease,
}
fn fixture() -> Signal {
    let (f, phone, sources, route) = established();
    permissions(&f, &route, &["sip_dial", "dtmf.receive"]);
    permissions(
        &f,
        &f.keys[1],
        &[
            "conversation.get",
            "subscribe",
            "dtmf.send",
            "operation.get",
        ],
    );
    let native = sources[1].clone();
    let mut request = native_selection(&f, &native, "established-sip-signal");
    let PeerCommand::NativeStream(intent) = &mut request.command else {
        panic!()
    };
    intent.command = connect_protocol::native_streams::Command::Dtmf;
    intent.expected_revision = None;
    intent.by_participant = Some(native.participant_id().into());
    intent.payload = json!({"by_participant":native.participant_id(),"digits":"42#","duration_ms":100,"method":"info"});
    let proof = f.admit(&request);
    f.store
        .accept_native_stream_with_peer(&f.keys[1], &proof)
        .unwrap();
    let lease = f
        .store
        .begin_native_stream_with_peer(&f.keys[1], &proof)
        .unwrap()
        .unwrap()
        .0;
    drop(proof);
    Signal {
        f,
        phone,
        native,
        route,
        request,
        lease,
    }
}
fn candidates(s: &Signal) -> Vec<NativeDtmfGatewayRecipient> {
    vec![NativeDtmfGatewayRecipient {
        origin: SipSignalOrigin::Established(s.phone.clone()),
        registration_id: "actual-established-sip-registration".into(),
    }]
}
fn observation(s: &Signal) -> SourceObservation {
    SourceObservation {
        envelope_id: "actual-native-to-established-observation".into(),
        received_at: chrono::Utc::now(),
        stream_revision: s.lease.expected_revision(),
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
            observation(s),
        )
        .unwrap()
}

#[test]
fn established_gateway_dtmf_freezes_actual_committed_generation_without_initial_or_native_surrogate(
) {
    let s = fixture();
    let batch = freeze(&s);
    assert!(batch.targets().is_empty());
    assert_eq!(batch.gateway_targets().len(), 1);
    let target = &batch.gateway_targets()[0];
    assert!(target.origin().initial().is_none());
    assert!(Arc::ptr_eq(
        target.origin().established().unwrap(),
        &s.phone
    ));
    assert_eq!(
        target.origin().sip_call_id(),
        s.phone.snapshot().media.call_id
    );
    let c = s.f.store.connection().unwrap();
    let (kind,json):(String,String) = c.query_row("SELECT target_kind,target_json FROM native_dtmf_deliveries WHERE operation_record_id=?1",[s.lease.record_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(kind, "sip-established");
    let snapshot: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(snapshot["generation"], 2);
    assert_eq!(snapshot["connection_id"], s.phone.snapshot().connection_id);
    for key in [
        "endpoint_credential_id",
        "endpoint_revision",
        "signing_key_id",
        "key_id",
    ] {
        assert!(snapshot.get(key).is_none(), "{key}");
    }
    for table in [
        "initial_telephone_operations",
        "native_connection_lifetimes",
        "native_dtmf_gateways",
    ] {
        let count: u64 = c
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE operation_record_id=?1"),
                [s.phone.record_id()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
    assert!(c
        .execute(
            "UPDATE native_dtmf_established_gateways SET target_json='{}'",
            []
        )
        .is_err());
    assert!(c
        .execute("DELETE FROM native_dtmf_established_gateways", [])
        .is_err());
    drop(c);
    let read = s.f.store.try_native_dtmf_gateway_read(target).unwrap();
    assert_eq!(
        read.phone_member().participant_id,
        s.phone.snapshot().participant_id
    );
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
        .unwrap();
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
        .unwrap();
    assert_eq!(
        s.f.store
            .try_start_native_dtmf_gateway_write(&read, "replacement-schedule")
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(read);
    s.f.store
        .try_record_native_dtmf_gateway_outcome(target, D::LocalSubmitted)
        .unwrap();
    let done = s.f.store.finish_native_dtmf(&batch).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(
        done.result.as_ref().unwrap()["recipients"][0]["generation"],
        2
    );
    assert_eq!(
        done.result.as_ref().unwrap()["recipients"][0]["frame_id"],
        "original-established-schedule"
    );
    assert!(s.f.store.try_native_dtmf_gateway_read(target).is_err());
}

#[test]
fn established_gateway_dtmf_current_rights_refuse_queued_work_but_captured_outcome_survives() {
    let s = fixture();
    let batch = freeze(&s);
    let target = &batch.gateway_targets()[0];
    // Queued metadata keeps no authority reader on either actual identity.
    for identity in [
        &s.phone.snapshot().identity_id,
        &s.phone.snapshot().route_owner_id,
    ] {
        assert!(s
            .f
            .store
            .try_acquire_authority(&[crate::AuthorityScope::identity(
                identity,
                crate::AuthorityMode::Write
            )
            .unwrap()])
            .is_ok());
    }
    permissions(&s.f, &s.route, &["sip_dial"]);
    assert!(s.f.store.try_native_dtmf_gateway_read(target).is_err());
    permissions(&s.f, &s.route, &["sip_dial", "dtmf.receive"]);
    let read = s.f.store.try_native_dtmf_gateway_read(target).unwrap();
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
        .unwrap();
    drop(read);
    s.f.store
        .connection()
        .unwrap()
        .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&s.route])
        .unwrap();
    assert!(s.f.store.try_native_dtmf_gateway_read(target).is_err());
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
    assert_eq!(
        s.f.store.finish_native_dtmf(&batch).unwrap().state,
        OperationState::Completed
    );
}

#[test]
fn established_gateway_dtmf_recovery_preserves_original_reserved_uncertainty_and_never_replays() {
    use connect_protocol::transitions::OperationAction;
    for attempted in [false, true] {
        let s = fixture();
        let batch = freeze(&s);
        if attempted {
            let read =
                s.f.store
                    .try_native_dtmf_gateway_read(&batch.gateway_targets()[0])
                    .unwrap();
            s.f.store
                .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
                .unwrap();
        }
        let path = s.f.store.path.clone();
        let key = s.f.keys[1].clone();
        drop(s.f.store);
        let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
        successor.recover_native_streams(256).unwrap();
        assert_eq!(successor.recover_native_dtmf(1).unwrap(), 1);
        assert_eq!(successor.recover_native_dtmf(1).unwrap(), 0);
        let result = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: "established-sip-signal".into(),
                },
            )
            .unwrap();
        assert_eq!(
            result.state,
            if attempted {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        let signal = &result.problem.as_ref().unwrap()["signal"];
        assert_eq!(
            signal["recipients"][0]["disposition"],
            if attempted {
                "outcome_unknown"
            } else {
                "not_started"
            }
        );
        assert_eq!(signal["recipients"][0]["generation"], 2);
        assert_eq!(
            signal["recipients"][0]["frame_id"],
            if attempted {
                json!("original-established-schedule")
            } else {
                serde_json::Value::Null
            }
        );
        assert!(successor
            .try_native_dtmf_gateway_read(&batch.gateway_targets()[0])
            .is_err());
    }
}

#[test]
fn established_gateway_dtmf_snapshot_digest_includes_recipient_only_reservation() {
    let s = fixture();
    let batch = freeze(&s);
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    let read =
        s.f.store
            .try_native_dtmf_gateway_read(&batch.gateway_targets()[0])
            .unwrap();
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
        .unwrap();
    drop(read);
    let after = Store::inspect_snapshot(s.f.store.path()).unwrap();
    assert_eq!(before.schema, crate::SCHEMA_VERSION);
    assert_ne!(before.digest, after.digest);
}

#[test]
fn established_gateway_dtmf_schema_refuses_wrong_call_generation_route_or_borrowed_native_facts() {
    let s = fixture();
    let c = s.f.store.connection().unwrap();
    c.execute("INSERT INTO native_dtmf_sources(operation_record_id,runtime_epoch,observation_json) VALUES(?1,?2,?3)",params![s.lease.record_id(),s.f.store.runtime_epoch(),serde_json::to_string(&observation(&s)).unwrap()]).unwrap();
    let p = s.phone.snapshot();
    let exact = json!({"connection_id":p.connection_id,"runtime_connection_id":p.runtime_connection_id,"sip_call_id":p.media.call_id,"participant_id":p.participant_id,"membership_epoch":p.membership_epoch,"generation":p.generation,"identity_id":p.identity_id,"endpoint_id":p.endpoint_id,"device_id":p.device_id,"route_credential_id":p.route_credential_id,"route_owner_id":p.route_owner_id});
    for field in [
        "connection_id",
        "runtime_connection_id",
        "sip_call_id",
        "participant_id",
        "membership_epoch",
        "generation",
        "identity_id",
        "endpoint_id",
        "device_id",
        "route_credential_id",
        "route_owner_id",
        "endpoint_credential_id",
        "key_id",
        "endpoint_revision",
    ] {
        let mut wrong = exact.clone();
        wrong[field] = if matches!(
            field,
            "generation" | "membership_epoch" | "endpoint_revision"
        ) {
            json!(999)
        } else {
            json!("borrowed-other-incarnation")
        };
        let result=c.execute("INSERT INTO native_dtmf_established_gateways(operation_record_id,target_record_id,registration_id,target_json) VALUES(?1,?2,'actual-established-sip-registration',?3)",params![s.lease.record_id(),s.phone.record_id(),wrong.to_string()]);
        assert!(result.is_err(), "{field}");
    }
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM native_dtmf_established_gateways",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert!(c.execute("INSERT INTO native_dtmf_established_gateways(operation_record_id,target_record_id,registration_id,target_json) VALUES(?1,?2,'actual-established-sip-registration',?3)",params![s.lease.record_id(),s.native.offer_record_id(),exact.to_string()]).is_err());
    c.execute("INSERT INTO native_dtmf_established_gateways(operation_record_id,target_record_id,registration_id,target_json) VALUES(?1,?2,'actual-established-sip-registration',?3)",params![s.lease.record_id(),s.phone.record_id(),exact.to_string()]).unwrap();
    c.execute(
        "UPDATE native_dtmf_sources SET sealed=1 WHERE operation_record_id=?1",
        [s.lease.record_id()],
    )
    .unwrap();
    assert!(c.execute("INSERT INTO native_dtmf_established_gateways(operation_record_id,target_record_id,registration_id,target_json) VALUES(?1,?2,'late-registration',?3)",params![s.lease.record_id(),s.phone.record_id(),exact.to_string()]).is_err());
}

#[test]
fn established_gateway_dtmf_failed_reservation_rolls_back_and_retries_original_schedule_only() {
    let s = fixture();
    let batch = freeze(&s);
    let target = &batch.gateway_targets()[0];
    let read = s.f.store.try_native_dtmf_gateway_read(target).unwrap();
    s.f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_established_schedule BEFORE UPDATE OF attempted ON native_dtmf_established_gateways BEGIN SELECT RAISE(ABORT,'controlled-failure'); END;").unwrap();
    assert!(s
        .f
        .store
        .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
        .is_err());
    let c = s.f.store.connection().unwrap();
    let state: (bool, Option<String>) = c
        .query_row(
            "SELECT attempted,frame_id FROM native_dtmf_established_gateways",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, (false, None));
    c.execute_batch("DROP TRIGGER fail_established_schedule")
        .unwrap();
    drop(c);
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-established-schedule")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_native_dtmf_gateway_outcome(target, D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&batch).unwrap().state,
        OperationState::Completed
    );
}
