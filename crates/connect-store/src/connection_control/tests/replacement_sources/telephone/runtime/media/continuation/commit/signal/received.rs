//! Actual committed-origin authority/custody fixtures; no physical IO claim.
use super::*;
use crate::{NativeDtmfRecipient, ReceivedDtmfBatch, ReceivedDtmfPreparation, SipSignalOrigin};
use connect_protocol::{
    native_dtmf::SubmissionDisposition as D,
    received_dtmf::{SourceKind, SourceObservation},
    transitions::OperationAction,
};
struct Signal {
    f: Fixture,
    phone: Arc<crate::TelephoneMediaOrigin>,
    native: Arc<crate::NativeMediaOrigin>,
    route: String,
    reader: String,
}
fn fixture() -> Signal {
    let (f, phone, sources, route) = established();
    permissions(&f, &route, &["sip_dial", "dtmf.send"]);
    permissions(
        &f,
        &f.keys[1],
        &["conversation.get", "subscribe", "dtmf.receive"],
    );
    let reader = f
        .store
        .issue(
            &phone.snapshot().identity_id,
            Scope::operator(),
            crate::now() + 600,
        )
        .unwrap()
        .id;
    Signal {
        f,
        phone,
        native: sources[1].clone(),
        route,
        reader,
    }
}
fn observation() -> SourceObservation {
    SourceObservation {
        kind: SourceKind::SipInfo,
        observation_id: "actual-established-received-input".into(),
        received_at: chrono::Utc::now(),
        digits: "5".into(),
        duration_ms: 100,
        rtp: None,
    }
}
fn candidates(s: &Signal) -> Vec<NativeDtmfRecipient> {
    vec![NativeDtmfRecipient {
        origin: s.native.clone(),
        registration_id: "actual-established-native-registration".into(),
    }]
}
fn prepare(s: &Signal, o: SourceObservation) -> connect_protocol::Result<ReceivedDtmfPreparation> {
    s.f.store.prepare_received_dtmf(
        SipSignalOrigin::Established(s.phone.clone()),
        o,
        candidates(s),
    )
}
fn freeze(s: &Signal, o: SourceObservation) -> ReceivedDtmfBatch {
    match prepare(s, o).unwrap() {
        ReceivedDtmfPreparation::New(batch) => batch,
        _ => panic!("expected new input"),
    }
}
fn get(
    s: &Signal,
    b: &ReceivedDtmfBatch,
) -> connect_protocol::Result<connect_protocol::transitions::OperationRecord> {
    s.f.store.execute_operation_action(
        &s.reader,
        OperationAction::Get {
            operation_id: b.operation_id().into(),
        },
    )
}

#[test]
fn established_received_dtmf_owns_logical_member_and_real_call_without_borrowing_retired_credentials(
) {
    let s = fixture();
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.f.keys[0]).unwrap();
    let observed = observation();
    let batch = freeze(&s, observed.clone());
    assert_eq!(batch.targets().len(), 1);
    assert_eq!(batch.observation(), &observed);
    let c = s.f.store.connection().unwrap();
    let (owner,parent,kind,source):(String,String,String,String)=c.query_row("SELECT o.owner_id,s.parent_record_id,s.parent_kind,s.source_json FROM received_dtmf_sources s JOIN operations o ON o.record_id=s.operation_record_id WHERE s.operation_record_id=?1",[batch.record_id()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(owner, s.phone.snapshot().identity_id);
    assert_ne!(owner, s.phone.snapshot().route_owner_id);
    assert_eq!(parent, s.phone.record_id());
    assert_eq!(kind, "sip-established");
    let source: serde_json::Value = serde_json::from_str(&source).unwrap();
    assert_eq!(source["generation"], 2);
    assert_eq!(source["sip_call_id"], s.phone.snapshot().media.call_id);
    for key in [
        "endpoint_credential_id",
        "endpoint_revision",
        "key_id",
        "signing_key_id",
    ] {
        assert!(source.get(key).is_none(), "{key}");
    }
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM initial_telephone_operations WHERE operation_record_id=?1",
            [s.phone.record_id()],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    assert!(c
        .execute(
            "UPDATE received_dtmf_sources SET parent_kind='sip-initial'",
            []
        )
        .is_err());
    drop(c);
    assert_eq!(get(&s, &batch).unwrap().state, OperationState::Preparing);
    assert!(s
        .f
        .store
        .execute_operation_action(
            &s.route,
            OperationAction::Get {
                operation_id: batch.operation_id().into()
            }
        )
        .is_err());
    let read =
        s.f.store
            .try_received_dtmf_read(&batch.targets()[0])
            .unwrap();
    assert!(read.current());
    s.f.store
        .try_start_received_dtmf_write(&read, "original-moved-received-frame")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_received_dtmf_outcome(&batch.targets()[0], D::LocalSubmitted)
        .unwrap();
    let done = s.f.store.finish_received_dtmf(&batch).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(done.result.as_ref().unwrap()["signal_source"], "sip-info");
    assert_eq!(
        done.result.as_ref().unwrap()["connection_id"],
        s.phone.snapshot().connection_id
    );
}

#[test]
fn established_received_dtmf_original_input_replay_rechecks_route_ceiling_without_recapturing_audience(
) {
    let s = fixture();
    let original = observation();
    let batch = freeze(&s, original.clone());
    let before = get(&s, &batch).unwrap();
    let mut repeated = original.clone();
    repeated.received_at -= chrono::Duration::seconds(1);
    let ReceivedDtmfPreparation::Replay(replay) = prepare(&s, repeated).unwrap() else {
        panic!()
    };
    assert_eq!(replay, before);
    let mut changed = original.clone();
    changed.digits = "9".into();
    assert_eq!(prepare(&s, changed).unwrap_err().code, "conflict");
    let c = s.f.store.connection().unwrap();
    c.execute(
        "UPDATE credentials SET recipients=?2 WHERE id=?1",
        params![s.route, json!([s.phone.snapshot().identity_id]).to_string()],
    )
    .unwrap();
    drop(c);
    assert!(prepare(&s, original.clone()).is_err());
    s.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients=?2 WHERE id=?1",
            params![s.route, json!(["*"]).to_string()],
        )
        .unwrap();
    let ReceivedDtmfPreparation::Replay(restored) = prepare(&s, original).unwrap() else {
        panic!()
    };
    assert_eq!(restored, before);
    assert_eq!(s.f.count("received_dtmf_sources"), 1);
    assert_eq!(s.f.count("received_dtmf_recipients"), 1);
}

#[test]
fn established_received_dtmf_directions_are_independent_and_known_output_survives_route_revocation()
{
    let s = fixture();
    let batch = freeze(&s, observation());
    let target = &batch.targets()[0];
    for (route, native, allowed) in [
        (
            vec!["sip_dial", "dtmf.receive"],
            vec!["conversation.get", "subscribe", "dtmf.receive"],
            false,
        ),
        (
            vec!["sip_dial", "dtmf.send"],
            vec!["conversation.get", "subscribe", "dtmf.send"],
            false,
        ),
        (
            vec!["sip_dial", "dtmf.send"],
            vec!["conversation.get", "subscribe", "dtmf.receive"],
            true,
        ),
    ] {
        permissions(&s.f, &s.route, &route);
        permissions(&s.f, &s.f.keys[1], &native);
        assert_eq!(s.f.store.try_received_dtmf_read(target).is_ok(), allowed);
    }
    let read = s.f.store.try_received_dtmf_read(target).unwrap();
    s.f.store
        .try_start_received_dtmf_write(&read, "original-moved-received-frame")
        .unwrap();
    assert_eq!(
        s.f.store
            .try_start_received_dtmf_write(&read, "other-frame")
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(read);
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.route).unwrap();
    assert!(s.f.store.try_received_dtmf_read(target).is_err());
    s.f.store
        .try_record_received_dtmf_outcome(target, D::LocalSubmitted)
        .unwrap();
    s.f.store
        .try_record_received_dtmf_outcome(target, D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_received_dtmf(&batch).unwrap().state,
        OperationState::Completed
    );
}

#[test]
fn established_received_dtmf_single_current_interval_blocks_revocation_without_queued_readers() {
    let s = fixture();
    let batch = freeze(&s, observation());
    let scope = crate::AuthorityScope::identity(
        &s.phone.snapshot().route_owner_id,
        crate::AuthorityMode::Write,
    )
    .unwrap();
    assert!(s.f.store.try_acquire_authority(&[scope.clone()]).is_ok());
    let read =
        s.f.store
            .try_received_dtmf_read(&batch.targets()[0])
            .unwrap();
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    std::thread::scope(|threads| {
        let writer = threads.spawn(|| s.f.store.revoke(&admin, &s.route));
        crate::authority_fences::test_wait_for_writer(&s.f.store, &scope);
        assert!(read.current());
        assert!(s
            .f
            .store
            .try_received_dtmf_read(&batch.targets()[0])
            .is_err());
        drop(read);
        writer.join().unwrap().unwrap();
    });
    assert!(s
        .f
        .store
        .try_received_dtmf_read(&batch.targets()[0])
        .is_err());
    s.f.store
        .try_record_received_dtmf_outcome(&batch.targets()[0], D::NotStarted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_received_dtmf(&batch).unwrap().state,
        OperationState::Failed
    );
}

#[test]
fn established_received_dtmf_successor_preserves_reserved_unknown_and_never_replays_input() {
    for attempted in [false, true] {
        let s = fixture();
        let batch = freeze(&s, observation());
        if attempted {
            let read =
                s.f.store
                    .try_received_dtmf_read(&batch.targets()[0])
                    .unwrap();
            s.f.store
                .try_start_received_dtmf_write(&read, "original-moved-received-frame")
                .unwrap();
        }
        let path = s.f.store.path.clone();
        let reader = s.reader.clone();
        drop(s.f.store);
        let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
        assert_eq!(successor.recover_received_dtmf(1).unwrap(), 1);
        assert_eq!(successor.recover_received_dtmf(1).unwrap(), 0);
        let status = successor
            .execute_operation_action(
                &reader,
                OperationAction::Get {
                    operation_id: batch.operation_id().into(),
                },
            )
            .unwrap();
        assert_eq!(
            status.state,
            if attempted {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        assert_eq!(
            status.problem.as_ref().unwrap()["signal"]["recipients"][0]["disposition"],
            if attempted {
                "outcome_unknown"
            } else {
                "not_started"
            }
        );
        assert!(successor
            .try_received_dtmf_read(&batch.targets()[0])
            .is_err());
    }
}

#[test]
fn established_received_dtmf_snapshot_detects_recipient_reservation_and_sql_origin_kind_is_immutable(
) {
    let s = fixture();
    let batch = freeze(&s, observation());
    let before = Store::inspect_snapshot(s.f.store.path()).unwrap();
    let read =
        s.f.store
            .try_received_dtmf_read(&batch.targets()[0])
            .unwrap();
    s.f.store
        .try_start_received_dtmf_write(&read, "original-moved-received-frame")
        .unwrap();
    drop(read);
    assert_ne!(
        before.digest,
        Store::inspect_snapshot(s.f.store.path()).unwrap().digest
    );
    let c = s.f.store.connection().unwrap();
    assert!(c.execute("UPDATE received_dtmf_sources SET parent_kind='sip-initial' WHERE operation_record_id=?1",[batch.record_id()]).is_err());
    assert!(c
        .execute(
            "UPDATE received_dtmf_sources SET parent_record_id=?2 WHERE operation_record_id=?1",
            params![batch.record_id(), s.native.offer_record_id()]
        )
        .is_err());
    assert!(c
        .execute(
            "DELETE FROM received_dtmf_sources WHERE operation_record_id=?1",
            [batch.record_id()]
        )
        .is_err());
}
