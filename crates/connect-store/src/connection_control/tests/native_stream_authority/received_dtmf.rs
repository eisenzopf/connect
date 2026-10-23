//! Controlled journal/authorization facts only; no physical SIP/native IO claim.
use super::*;
use crate::{NativeDtmfRecipient, ReceivedDtmfBatch, ReceivedDtmfPreparation};
use connect_protocol::{
    native_dtmf::SubmissionDisposition as D,
    received_dtmf::{SourceKind, SourceObservation},
    transitions::{OperationAction, OperationState},
};
use std::sync::Arc;
fn observed() -> SourceObservation {
    SourceObservation {
        kind: SourceKind::SipInfo,
        observation_id: "actual-sip-observation".into(),
        received_at: chrono::Utc::now(),
        digits: "5".into(),
        duration_ms: 237,
        rtp: None,
    }
}
fn freeze(
    f: &Fixture,
    native: &Arc<crate::NativeMediaOrigin>,
    phone: &Arc<crate::InitialTelephoneMediaOrigin>,
    o: SourceObservation,
) -> ReceivedDtmfBatch {
    match f
        .store
        .prepare_received_dtmf(
            phone.clone(),
            o,
            vec![NativeDtmfRecipient {
                origin: native.clone(),
                registration_id: "original-registration".into(),
            }],
        )
        .unwrap()
    {
        ReceivedDtmfPreparation::New(b) => b,
        _ => panic!("expected original source"),
    }
}
fn get(
    f: &Fixture,
    b: &ReceivedDtmfBatch,
) -> connect_protocol::Result<connect_protocol::transitions::OperationRecord> {
    f.store.execute_operation_action(
        &f.keys[1],
        OperationAction::Get {
            operation_id: b.operation_id().into(),
        },
    )
}
fn export(record: &connect_protocol::transitions::OperationRecord) {
    if let Some(path) = std::env::var_os("CONNECT_RECEIVED_DTMF_STORE_RESULTS") {
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
fn received_dtmf_freezes_real_sip_facts_and_original_expiry_without_a_native_source() {
    let (f, n, p) = sip_signal::fixture();
    let o = observed();
    let b = freeze(&f, &n, &p, o.clone());
    assert_eq!(b.targets().len(), 1);
    assert_eq!(b.observation(), &o);
    let before = get(&f, &b).unwrap();
    assert_eq!(before.state, OperationState::Preparing);
    assert!(before.result.is_none());
    assert!(before.expires_at.unwrap() <= o.received_at.timestamp() + 40);
    let mut later = o.clone();
    later.received_at += chrono::Duration::seconds(1);
    // Use an earlier timestamp rather than waiting: replay ignores only the
    // delivery time, never extends the first receive window.
    later.received_at -= chrono::Duration::seconds(2);
    let ReceivedDtmfPreparation::Replay(replay) =
        f.store.prepare_received_dtmf(p, later, vec![]).unwrap()
    else {
        panic!()
    };
    assert_eq!(before, replay);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM native_dtmf_sources", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT attempted FROM received_dtmf_recipients", [], |r| {
            r.get::<_, bool>(0)
        })
        .unwrap(),
        false
    );
}
#[test]
fn received_dtmf_reserves_one_actual_frame_before_local_submission_and_cannot_rewrite_it() {
    let (f, n, p) = sip_signal::fixture();
    let b = freeze(&f, &n, &p, observed());
    let t = &b.targets()[0];
    assert!(f
        .store
        .try_record_received_dtmf_outcome(t, D::LocalSubmitted)
        .is_err());
    let read = f.store.try_received_dtmf_read(t).unwrap();
    f.store
        .try_start_received_dtmf_write(&read, "actual-signed-frame")
        .unwrap();
    f.store
        .try_start_received_dtmf_write(&read, "actual-signed-frame")
        .unwrap();
    assert!(f
        .store
        .try_start_received_dtmf_write(&read, "different-frame")
        .is_err());
    drop(read);
    f.store
        .try_record_received_dtmf_outcome(t, D::LocalSubmitted)
        .unwrap();
    f.store
        .try_record_received_dtmf_outcome(t, D::LocalSubmitted)
        .unwrap();
    assert!(f
        .store
        .try_record_received_dtmf_outcome(t, D::OutcomeUnknown)
        .is_err());
    let done = f.store.finish_received_dtmf(&b).unwrap();
    assert_eq!(done.state, OperationState::Completed);
    assert_eq!(f.store.finish_received_dtmf(&b).unwrap(), done);
    assert!(f.store.try_received_dtmf_read(t).is_err());
    let public = get(&f, &b).unwrap();
    export(&public);
    let result = public.result.unwrap();
    assert_eq!(result["signal_source"], "sip-info");
    assert_eq!(
        result["source_observation"],
        serde_json::to_value(b.observation()).unwrap()
    );
    assert_eq!(result["recipients"][0]["frame_id"], "actual-signed-frame");
    for key in [
        "stream_revision",
        "signing_key_id",
        "signing_algorithm",
        "envelope_id",
    ] {
        assert!(result.get(key).is_none());
    }
}
#[test]
fn received_dtmf_current_source_and_recipient_named_permissions_are_independent_of_media() {
    for fault in ["source", "recipient"] {
        let (f, n, p) = sip_signal::fixture();
        let b = freeze(&f, &n, &p, observed());
        let actor = if fault == "source" { 1 } else { 0 };
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![f.keys[actor], "[\"stream.subscribe\",\"connection.offer\"]"],
            )
            .unwrap();
        assert!(
            f.store.try_received_dtmf_read(&b.targets()[0]).is_err(),
            "{fault}"
        );
        // Captured no-start facts remain recordable after rights withdrawal.
        f.store
            .try_record_received_dtmf_outcome(&b.targets()[0], D::NotStarted)
            .unwrap();
        assert_eq!(
            f.store.finish_received_dtmf(&b).unwrap().state,
            OperationState::Failed
        );
    }
    let (f, n, p) = sip_signal::fixture();
    for (i, names) in [(0, "[\"dtmf.receive\"]"), (1, "[\"dtmf.send\"]")] {
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![f.keys[i], names],
            )
            .unwrap();
    }
    let b = freeze(&f, &n, &p, observed());
    assert!(f.store.try_received_dtmf_read(&b.targets()[0]).is_ok());
    assert!(f.store.try_native_media_read(&n, None).is_err());
    assert!(f.store.try_initial_telephone_media_read(p).is_err());
}
#[test]
fn received_dtmf_conflicting_physical_id_or_expired_input_cannot_reserve_another_effect() {
    let (f, n, p) = sip_signal::fixture();
    let o = observed();
    let _b = freeze(&f, &n, &p, o.clone());
    let mut changed = o.clone();
    changed.digits = "6".into();
    assert_eq!(
        f.store
            .prepare_received_dtmf(p.clone(), changed, vec![])
            .unwrap_err()
            .code,
        "conflict"
    );
    for delta in [-41, 5] {
        let mut o = o.clone();
        o.observation_id = format!("time-{delta}");
        o.received_at += chrono::Duration::seconds(delta);
        assert!(f.store.prepare_received_dtmf(p.clone(), o, vec![]).is_err());
    }
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT count(*) FROM received_dtmf_sources", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        1
    );
}
#[test]
fn received_dtmf_empty_frozen_audience_is_a_truthful_no_submission_failure() {
    let (f, _n, p) = sip_signal::fixture();
    let ReceivedDtmfPreparation::New(b) = f
        .store
        .prepare_received_dtmf(p, observed(), vec![])
        .unwrap()
    else {
        panic!()
    };
    let r = f.store.finish_received_dtmf(&b).unwrap();
    export(&r);
    assert_eq!(r.state, OperationState::Failed);
    assert_eq!(r.problem.unwrap()["signal"]["recipients"], json!([]));
}
#[test]
fn received_dtmf_cancel_waits_for_owned_read_and_prevents_new_physical_attempts() {
    let (f, n, p) = sip_signal::fixture();
    let b = freeze(&f, &n, &p, observed());
    let read = f.store.try_received_dtmf_read(&b.targets()[0]).unwrap();
    let writes = [crate::AuthorityScope::identity(
        &f.store.revalidate(&f.keys[1]).unwrap().identity_id,
        crate::AuthorityMode::Write,
    )
    .unwrap()];
    assert!(f.store.try_acquire_authority(&writes).is_err());
    drop(read);
    f.store
        .execute_operation_action(
            &f.keys[1],
            OperationAction::Cancel {
                operation_id: "cancel-received".into(),
                target_operation_id: b.operation_id().into(),
                reason: None,
            },
        )
        .unwrap();
    assert!(f.store.try_received_dtmf_read(&b.targets()[0]).is_err());
    f.store
        .try_record_received_dtmf_outcome(&b.targets()[0], D::NotStarted)
        .unwrap();
    let cancelled = f.store.finish_received_dtmf(&b).unwrap();
    export(&cancelled);
    assert_eq!(cancelled.state, OperationState::Cancelled);
}
#[test]
fn received_dtmf_successor_records_reserved_uncertainty_without_replaying_input() {
    for attempted in [false, true] {
        let (f, n, p) = sip_signal::fixture();
        let b = freeze(&f, &n, &p, observed());
        if attempted {
            let r = f.store.try_received_dtmf_read(&b.targets()[0]).unwrap();
            f.store
                .try_start_received_dtmf_write(&r, "original-frame")
                .unwrap();
        }
        let path = f.store.path.clone();
        let key = f.keys[1].clone();
        drop(f.store);
        let successor = Store::open(&path, Limits::default()).unwrap();
        assert_eq!(successor.recover_received_dtmf(1).unwrap(), 1);
        assert_eq!(successor.recover_received_dtmf(1).unwrap(), 0);
        assert!(successor.try_received_dtmf_read(&b.targets()[0]).is_err());
        let r = successor
            .execute_operation_action(
                &key,
                OperationAction::Get {
                    operation_id: b.operation_id().into(),
                },
            )
            .unwrap();
        export(&r);
        assert_eq!(
            r.state,
            if attempted {
                OperationState::Unknown
            } else {
                OperationState::Failed
            }
        );
        assert_eq!(
            r.problem.unwrap()["signal"]["recipients"][0]["disposition"],
            if attempted {
                "outcome_unknown"
            } else {
                "not_started"
            }
        );
    }
}
#[test]
fn received_dtmf_recovery_waits_for_physical_read_release_before_settling_expiry() {
    let (f, n, p) = sip_signal::fixture();
    let b = freeze(&f, &n, &p, observed());
    let r = f.store.try_received_dtmf_read(&b.targets()[0]).unwrap();
    f.store
        .try_start_received_dtmf_write(&r, "original-frame")
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE operations SET expires_at=?2 WHERE record_id=?1",
            params![b.record_id(), crate::now() - 1],
        )
        .unwrap();
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let s = &f.store;
        let worker = scope.spawn(move || {
            let result = s.recover_received_dtmf(1);
            tx.send(result).unwrap();
        });
        assert!(rx.recv_timeout(Duration::from_millis(60)).is_err());
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row("SELECT closed FROM received_dtmf_sources", [], |r| r
                    .get::<_, bool>(0))
                .unwrap(),
            false
        );
        drop(r);
        assert_eq!(rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap(), 1);
        worker.join().unwrap();
    });
}
#[test]
fn received_dtmf_metadata_owner_and_sql_seals_refuse_forged_provenance() {
    let (f, n, p) = sip_signal::fixture();
    let b = freeze(&f, &n, &p, observed());
    assert!(f
        .store
        .execute_operation_action(
            &f.keys[0],
            OperationAction::Get {
                operation_id: b.operation_id().into()
            }
        )
        .is_err());
    let c = f.store.connection().unwrap();
    for sql in ["UPDATE received_dtmf_sources SET observation_json=json_set(observation_json,'$.digits','9')", "UPDATE received_dtmf_recipients SET registration_id='replacement'", "DELETE FROM received_dtmf_sources", "DELETE FROM received_dtmf_recipients"] {assert!(c.execute(sql,[]).is_err(),"{sql}");}
}

#[test]
fn received_dtmf_snapshot_digest_covers_original_source_observation() {
    let (f, n, p) = sip_signal::fixture();
    let _batch = freeze(&f, &n, &p, observed());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("received-source.sqlite3");
    let original = f.store.backup(&path).unwrap();
    assert_eq!(crate::Store::inspect_snapshot(&path).unwrap(), original);
    let c = rusqlite::Connection::open(&path).unwrap();
    // Adversarial offline copy only: live source custody remains immutable.
    // Logical row digests must detect changed contents even in such a copy.
    c.execute_batch("DROP TRIGGER received_dtmf_source_immutable;
        UPDATE received_dtmf_sources SET observation_json=json_set(observation_json,'$.digits','9');")
        .unwrap();
    drop(c);
    let changed = crate::Store::inspect_snapshot(&path).unwrap();
    assert_eq!(changed.schema, original.schema);
    assert_eq!(changed.installation_id, original.installation_id);
    assert_ne!(changed.digest, original.digest);
}

#[test]
fn received_dtmf_snapshot_digest_covers_recipient_write_reservation() {
    let (f, n, p) = sip_signal::fixture();
    let batch = freeze(&f, &n, &p, observed());
    let dir = tempfile::tempdir().unwrap();
    let before = f.store.backup(&dir.path().join("before.sqlite3")).unwrap();
    let operation = get(&f, &batch).unwrap();
    let read = f.store.try_received_dtmf_read(&batch.targets()[0]).unwrap();
    f.store
        .try_start_received_dtmf_write(&read, "reserved-original-frame")
        .unwrap();
    drop(read);
    let after = f.store.backup(&dir.path().join("after.sqlite3")).unwrap();
    assert_eq!(get(&f, &batch).unwrap(), operation);
    assert_eq!(before.schema, after.schema);
    assert_eq!(before.installation_id, after.installation_id);
    assert_ne!(before.digest, after.digest);
}
