//! Controlled writer callbacks, not positive physical IO evidence.
use super::*;
use connect_protocol::events::{EventQuery, JournalData, SnapshotQuery};
use connect_protocol::native_data::WriteDisposition as D;

fn fixture_for(
    actor: usize,
) -> (
    Fixture,
    crate::NativeDataWriter,
    crate::NativeStreamLease,
    String,
) {
    let (f, request, _) = data_request(true);
    let mut target = f.native_request_for(actor, 600, "writer-target");
    let PeerCommand::NativeConnectionOffer(offer) = &mut target.command else {
        panic!()
    };
    offer.offer = json!({"substrate":"quic","capabilities":{"data_protocols":["json"],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"application-in","kind":"data","direction":"recvonly","codec_preferences":["json"]}]});
    let proof = f.admit(&target);
    f.store
        .prepare_native_offer_with_peer(
            &f.keys[actor],
            &proof,
            Some(connect_protocol::provisioning::EndpointTransport::Quic),
        )
        .unwrap();
    let (target_lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &target_lease)
        .unwrap();
    let writer = f.store.native_data_writer(&origin).unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [target_lease.effect_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
            [target_lease.record_id()],
        )
        .unwrap();
        c.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![origin.connection_id(),f.store.runtime_epoch(),json!({"streams":["application-in"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
        c.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![target_lease.record_id(),origin.connection_id(),f.store.runtime_epoch(),origin.generation()]).unwrap();
    }
    drop(proof);
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
    (f, writer, lease, origin.runtime_connection_id().into())
}
fn fixture() -> (
    Fixture,
    crate::NativeDataWriter,
    crate::NativeStreamLease,
    String,
) {
    fixture_for(1)
}
fn reserve(f: &Fixture, writer: &crate::NativeDataWriter, lease: &crate::NativeStreamLease) {
    f.store
        .try_reserve_native_data_write(
            writer,
            "publish-application",
            "application-message",
            lease.runtime_connection_id(),
        )
        .unwrap();
}
fn record(
    f: &Fixture,
    writer: &crate::NativeDataWriter,
    lease: &crate::NativeStreamLease,
    target: &str,
    d: D,
) -> crate::Result<()> {
    f.store.try_record_native_data_write(
        writer,
        "publish-application",
        "application-message",
        lease.runtime_connection_id(),
        target,
        d,
    )
}
#[test]
fn native_data_writer_reserves_before_io_and_records_one_typed_local_fact_without_completing_queue_work(
) {
    let (f, w, lease, target) = fixture();
    let cursor = f
        .store
        .conversation_snapshot(&f.keys[0], &f.cid, SnapshotQuery::default())
        .unwrap()
        .cursor;
    assert_eq!(
        record(&f, &w, &lease, &target, D::LocalWritten)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let before = f.count("journal");
    let receipts = f.count("message_receipts");
    reserve(&f, &w, &lease);
    reserve(&f, &w, &lease);
    assert_eq!(f.count("native_data_writes"), 1);
    assert_eq!(f.count("journal"), before);
    record(&f, &w, &lease, &target, D::LocalWritten).unwrap();
    record(&f, &w, &lease, &target, D::LocalWritten).unwrap();
    assert_eq!(f.count("journal"), before + 1);
    assert_eq!(
        record(&f, &w, &lease, &target, D::SubmissionUnknown)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(f
        .store
        .try_reserve_native_data_write(
            &w,
            "publish-application",
            "application-message",
            lease.runtime_connection_id()
        )
        .is_err());
    let page = f
        .store
        .conversation_events(
            &f.keys[0],
            &f.cid,
            EventQuery {
                after: Some(cursor),
                ..Default::default()
            },
        )
        .unwrap();
    let facts: Vec<_> = page
        .events
        .iter()
        .filter(|e| e.kind == "native_data_write")
        .collect();
    assert_eq!(facts.len(), 1);
    let JournalData::NativeDataWrite(fact) = &facts[0].data else {
        panic!()
    };
    assert_eq!(fact.disposition, D::LocalWritten);
    assert_eq!(fact.message_id, "application-message");
    assert_eq!(fact.participant_id, f.pids[1]);
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [lease.record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "preparing"
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM message_receipts", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        receipts
    );
}
#[test]
fn native_data_writer_historical_fact_survives_cancel_revocation_and_retired_target_without_revival(
) {
    let (f, w, lease, target) = fixture();
    reserve(&f, &w, &lease);
    f.store
        .execute_operation_action(
            &f.admin,
            connect_protocol::transitions::OperationAction::Cancel {
                operation_id: "cancel-writer-source".into(),
                target_operation_id: "publish-application".into(),
                reason: None,
            },
        )
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[1]])
            .unwrap();
        c.execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&f.pids[1]],
        )
        .unwrap();
        c.execute("UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=(SELECT connection_id FROM connection_offers WHERE runtime_connection_id=?1)",[&target]).unwrap();
        c.execute("UPDATE connections SET state='retired',generation=generation+1,observed_runtime_id=NULL,readiness_json=NULL WHERE id=(SELECT connection_id FROM connection_offers WHERE runtime_connection_id=?1)",[&target]).unwrap();
        c.execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&f.pids[0]],
        )
        .unwrap();
        c.execute(
            "UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",
            [&f.pids[0]],
        )
        .unwrap();
    }
    record(&f, &w, &lease, &target, D::SubmissionUnknown).unwrap();
    let c = f.store.connection().unwrap();
    assert_eq!(c.query_row("SELECT recipient_participant_id,recipient_epoch FROM journal WHERE kind='native_data_write'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?))).unwrap(),(f.pids[0].clone(),1));
    assert_eq!(
        c.query_row(
            "SELECT state FROM operations WHERE record_id=?1",
            [lease.record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "unknown"
    );
    assert_eq!(c.query_row("SELECT state FROM connections WHERE id=(SELECT connection_id FROM connection_offers WHERE runtime_connection_id=?1)",[&target],|r|r.get::<_,String>(0)).unwrap(),"retired");
}
#[test]
fn native_data_writer_storage_failure_retains_original_reservation_and_same_callback_recovers() {
    let (f, w, lease, target) = fixture();
    reserve(&f, &w, &lease);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER writer_fault BEFORE INSERT ON journal WHEN NEW.kind='native_data_write' BEGIN SELECT RAISE(ABORT,'controlled writer fault'); END;").unwrap();
    assert_eq!(
        record(&f, &w, &lease, &target, D::LocalWritten)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert!(f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT disposition IS NULL FROM native_data_writes",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER writer_fault;")
        .unwrap();
    record(&f, &w, &lease, &target, D::LocalWritten).unwrap();
    assert_eq!(f.count("native_data_writes"), 1);
}
#[test]
fn native_data_writer_capacity_is_reserved_before_write_and_exchanged_for_exactly_one_fact() {
    let (mut f, w, lease, target) = fixture();
    let c = f.store.connection().unwrap();
    let used: f64 = c
        .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
        .unwrap();
    drop(c);
    f.store.limits.journal_events = used as u64;
    assert_eq!(
        f.store
            .try_reserve_native_data_write(
                &w,
                "publish-application",
                "application-message",
                lease.runtime_connection_id()
            )
            .unwrap_err()
            .code,
        "capacity-exceeded"
    );
    assert_eq!(f.count("native_data_writes"), 0);
    // Find the exact shared-budget boundary rather than assume other native
    // lifetime/operation reservations do not exist in this realistic fixture.
    while f
        .store
        .try_reserve_native_data_write(
            &w,
            "publish-application",
            "application-message",
            lease.runtime_connection_id(),
        )
        .is_err()
    {
        f.store.limits.journal_events += 1;
    }
    assert!(
        !crate::conversations::capacity(&f.store.connection().unwrap(), 1, 0, &f.store).unwrap()
    );
    record(&f, &w, &lease, &target, D::LocalWritten).unwrap();
    assert!(
        !crate::conversations::capacity(&f.store.connection().unwrap(), 1, 0, &f.store).unwrap()
    );
}
#[test]
fn native_data_writer_refusal_before_admission_is_recorded_but_cannot_become_written() {
    let (f, w, lease, target) = fixture();
    record(&f, &w, &lease, &target, D::NotStarted).unwrap();
    record(&f, &w, &lease, &target, D::NotStarted).unwrap();
    assert_eq!(
        record(&f, &w, &lease, &target, D::LocalWritten)
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(f.count("native_data_writes"), 1);
}
#[test]
fn native_data_writer_refuses_wrong_source_message_operation_target_and_store_without_facts() {
    let (f, w, lease, target) = fixture();
    let before = f.count("journal");
    for (op, msg, source, to) in [
        (
            "different",
            "application-message",
            lease.runtime_connection_id(),
            target.as_str(),
        ),
        (
            "publish-application",
            "different",
            lease.runtime_connection_id(),
            target.as_str(),
        ),
        (
            "publish-application",
            "application-message",
            "different",
            target.as_str(),
        ),
        (
            "publish-application",
            "application-message",
            lease.runtime_connection_id(),
            "different",
        ),
    ] {
        assert_eq!(
            f.store
                .try_record_native_data_write(&w, op, msg, source, to, D::NotStarted)
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    let other = Fixture::new();
    assert_eq!(
        other
            .store
            .try_record_native_data_write(
                &w,
                "publish-application",
                "application-message",
                lease.runtime_connection_id(),
                &target,
                D::NotStarted
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(f.count("journal"), before);
    assert_eq!(f.count("native_data_writes"), 0);
}
#[test]
fn native_data_writer_callback_refuses_store_contention_immediately_and_retries_same_identity() {
    let (f, w, lease, target) = fixture();
    reserve(&f, &w, &lease);
    let held = f.store.connection().unwrap();
    let start = std::time::Instant::now();
    assert_eq!(
        record(&f, &w, &lease, &target, D::LocalWritten)
            .unwrap_err()
            .code,
        "authority-busy"
    );
    assert!(start.elapsed() < Duration::from_millis(100));
    drop(held);
    record(&f, &w, &lease, &target, D::LocalWritten).unwrap();
}
#[test]
fn native_data_writer_successor_marks_only_pending_original_writes_unknown_and_rejects_old_tokens()
{
    let (f, w, lease, target) = fixture();
    reserve(&f, &w, &lease);
    let path = f.store.path.clone();
    let before = f.count("journal");
    assert_eq!(f.store.recover_native_data_writes(256).unwrap(), 0);
    drop(f.store);
    let successor = Store::open(&path, Limits::default()).unwrap();
    assert_eq!(
        successor
            .try_record_native_data_write(
                &w,
                "publish-application",
                "application-message",
                lease.runtime_connection_id(),
                &target,
                D::LocalWritten
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(successor.recover_native_data_writes(1).unwrap(), 1);
    assert_eq!(successor.recover_native_data_writes(1).unwrap(), 0);
    let c = successor.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT disposition,provenance FROM native_data_writes",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        )
        .unwrap(),
        ("submission_unknown".into(), "runtime_recovery".into())
    );
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM journal", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        before + 1
    );
}

#[test]
fn native_data_writer_genuine_unaddressed_target_cannot_reserve_or_record_a_write_and_refusal_releases_without_facts(
) {
    let (f, w, lease, target) = fixture_for(2);
    assert_eq!(
        f.store
            .try_reserve_native_data_write(
                &w,
                "publish-application",
                "application-message",
                lease.runtime_connection_id()
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        record(&f, &w, &lease, &target, D::LocalWritten)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let before = f.count("journal");
    record(&f, &w, &lease, &target, D::NotStarted).unwrap();
    assert_eq!(f.count("journal"), before);
    assert_eq!(f.count("native_data_writes"), 0);
}
#[test]
fn native_data_writer_sqlite_writer_contention_never_waits_and_preserves_pending_identity() {
    let (f, w, lease, target) = fixture();
    reserve(&f, &w, &lease);
    let other = rusqlite::Connection::open(&f.store.path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = std::time::Instant::now();
    assert_eq!(
        record(&f, &w, &lease, &target, D::LocalWritten)
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert!(start.elapsed() < Duration::from_millis(100));
    other.execute_batch("ROLLBACK").unwrap();
    assert!(f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT disposition IS NULL FROM native_data_writes",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    record(&f, &w, &lease, &target, D::LocalWritten).unwrap();
}
