//! Actual opaque Data Gate construction and persistent SQL refusal controls.
use super::*;
use crate::authority_writer_boundary::phase_ledger::FUNCTION;
use crate::AuthorityScope;
use rusqlite::{Connection, TransactionBehavior};
fn active(c: &Connection) -> i64 {
    c.query_row(
        &format!("SELECT {FUNCTION}('data-control-active','',NULL,'data-control',NULL,NULL,NULL)"),
        [],
        |r| r.get(0),
    )
    .unwrap()
}
#[test]
fn data_gate_permit_requires_same_store_unit_complete_proof_and_drops_on_rollback() {
    let (f, _, receipt, original) = prepared(Kind::DataGate);
    let boundary = &f.store.authority_writer_boundary;
    let before = database(&f.store);
    let (required, record) = {
        let c = f.store.connection().unwrap();
        let plan = receipt.data_control_plan(&f.store, &c).unwrap().unwrap();
        (plan.required, plan.key.record)
    };
    let record_write = AuthorityScope::record(&record, crate::AuthorityMode::Write).unwrap();
    let only_read = f
        .store
        .try_acquire_authority(
            &required
                .iter()
                .filter(|scope| *scope != &record_write)
                .cloned()
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let proof = f.store.try_acquire_authority(&required).unwrap();
    let other = Fixture::new();
    {
        let mut foreign = Connection::open(f.store.path()).unwrap();
        let tx = foreign
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            boundary
                .native_data_control_ledger(&f.store, &tx, &proof, &receipt)
                .is_err(),
            "foreign SQL cannot borrow registered primary custody"
        );
        tx.rollback().unwrap();
    }
    {
        let mut c = f.store.connection().unwrap();
        // Constructor itself must pin full proof; no earlier bind supplies it.
        assert!(boundary
            .native_data_control_ledger(&f.store, &c, &proof, &receipt)
            .is_err());
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert!(
            boundary
                .native_data_control_ledger(&f.store, &tx, &only_read, &receipt)
                .is_err(),
            "Record Writer cannot be replaced by a Reader"
        );
        assert!(
            other
                .store
                .authority_writer_boundary
                .native_data_control_ledger(&f.store, &tx, &proof, &receipt)
                .is_err(),
            "foreign Store cannot install this receipt"
        );
        let (old, _) = boundary
            .native_data_control_ledger(&f.store, &tx, &proof, &receipt)
            .unwrap()
            .unwrap();
        assert_eq!(active(&tx), 1);
        assert!(boundary
            .native_data_control_ledger(&f.store, &tx, &proof, &receipt)
            .is_err());
        // Real rollback hook clears the old row program while old Rust Drop is
        // delayed; a new actual transaction receives a strictly new nonce.
        tx.execute_batch("ROLLBACK; BEGIN IMMEDIATE").unwrap();
        assert_eq!(active(&tx), 0);
        let (fresh, plan) = boundary
            .native_data_control_ledger(&f.store, &tx, &proof, &receipt)
            .unwrap()
            .unwrap();
        drop(old);
        assert_eq!(active(&tx), 1, "stale Drop cleared a new nonce");
        drop(proof);
        let record_read = AuthorityScope::record(&record, crate::AuthorityMode::Read).unwrap();
        assert!(
            f.store
                .try_acquire_authority(&[record_read.clone()])
                .is_err(),
            "caller Drop released a live Record Writer"
        );
        plan.rows[0].apply(&tx).unwrap();
        assert!(
            tx.execute_batch("COMMIT").is_err(),
            "live row permit must refuse COMMIT and roll back"
        );
        assert_eq!(active(&tx), 0);
        drop(fresh);
        assert!(
            f.store
                .try_acquire_authority(&[record_read.clone()])
                .is_err(),
            "rollback released Unit-pinned proof before Unit finish"
        );
        tx.execute_batch("BEGIN IMMEDIATE").unwrap();
        tx.commit().unwrap();
        assert!(
            f.store
                .try_acquire_authority(&[record_read.clone()])
                .is_err(),
            "COMMIT released proof before original Unit finished"
        );
    }
    let released = f
        .store
        .try_acquire_authority(&[
            AuthorityScope::record(&record, crate::AuthorityMode::Read).unwrap()
        ])
        .unwrap();
    drop(released);
    assert_eq!(database(&f.store), before);
    drop(only_read);
    drop(original);
    let complete = f.store.complete_native_stream(&receipt).unwrap();
    assert_eq!(complete.state, OperationState::Completed);
    assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), complete);
}
#[test]
fn data_gate_exact_rows_deny_authority_edits_wrong_types_deletes_and_replace_victims() {
    for attack in [
        "UPDATE connections SET identity_id=identity_id||'-foreign' WHERE id='connection-native'",
        "UPDATE connections SET readiness_json=json_set(readiness_json,'$.codec_bindings',json('[]')) WHERE id='connection-native'",
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        "UPDATE operation_effects SET state='failed' WHERE operation_record_id=?1 AND provider='native-stream'",
        "UPDATE native_stream_operations SET closed=CAST('1' AS BLOB) WHERE operation_record_id=?1",
        "DELETE FROM native_stream_snapshots WHERE connection_id='connection-native'",
        "INSERT OR REPLACE INTO native_stream_snapshots SELECT connection_id,generation,stream_revision,available,projection_json FROM native_stream_snapshots WHERE connection_id='connection-native'",
    ] {
        let (f,_,receipt,original)=prepared(Kind::DataGate);
        let before=database(&f.store);
        let plan={let c=f.store.connection().unwrap();receipt.data_control_plan(&f.store,&c).unwrap().unwrap()};
        let record=plan.key.record.clone();
        let proof=f.store.try_acquire_authority(&plan.required).unwrap();
        {
            let mut c=f.store.connection().unwrap();c.bind_available_mutation_authority(&f.store,&proof).unwrap();
            let tx=c.transaction_with_behavior(TransactionBehavior::Immediate).unwrap();
            let (permit,plan)=f.store.authority_writer_boundary.native_data_control_ledger(&f.store,&tx,&proof,&receipt).unwrap().unwrap();
            assert_eq!(active(&tx),1);
            plan.rows[0].apply(&tx).unwrap();
            let refused=if attack.contains("?1") {tx.execute(attack,[&record])} else {tx.execute(attack,[])};
            assert!(refused.is_err(),"closed Data capture admitted forbidden SQL: {attack}");
            drop(permit);tx.rollback().unwrap();
        }
        assert_eq!(database(&f.store),before);
        drop(proof);drop(original);
        assert_eq!(f.store.complete_native_stream(&receipt).unwrap().state,OperationState::Completed);
    }
}

#[test]
fn data_gate_same_receipt_completion_between_preview_and_admission_stays_read_only() {
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::atomic::{AtomicUsize, Ordering},
    };
    let (f, origin, receipt, original) = prepared(Kind::DataGate);
    let store = Arc::new(f.store);
    let receipt = Arc::new(receipt);
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = calls.clone();
    let other_store = store.clone();
    let same = receipt.clone();
    let external = Rc::new(RefCell::new(None::<Connection>));
    let external_for_hook = external.clone();
    let until = Instant::now() + Duration::from_secs(2);
    let hook = Store::data_control_before_admission_for_test(move || {
        entered.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            other_store.complete_native_stream(&same).unwrap().state,
            OperationState::Completed
        );
        let raw = Connection::open(other_store.path()).unwrap();
        raw.execute_batch("BEGIN IMMEDIATE").unwrap();
        *external_for_hook.borrow_mut() = Some(raw);
    });
    // Direct genuine IO avoids a companion watcher racing the targeted preview
    // boundary. Snapshot/queued-epoch survival is covered by OLD63 and the
    // separate public replay control; this control proves READ-ONLY SQL mode.
    let work = catch_unwind(AssertUnwindSafe(|| {
        let io = store.try_native_media_read(&origin, None).unwrap();
        let av_before = audio_metadata(&store);
        let first = store.complete_native_stream(&receipt);
        let io_current = io.current();
        let av_unchanged = audio_metadata(&store) == av_before;
        let within = Instant::now() < until;
        drop(io);
        (first, io_current, av_unchanged, within)
    }));
    drop(hook);
    let cleanup = external
        .borrow_mut()
        .take()
        .map(|c| c.execute_batch("ROLLBACK"))
        .unwrap_or(Ok(()));
    drop(original);
    let (first, io_current, av_unchanged, within) = match work {
        Ok(v) => v,
        Err(panic) => {
            let _ = cleanup;
            resume_unwind(panic)
        }
    };
    cleanup.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        io_current && av_unchanged && within,
        "race control changed original AV facts or cutoff"
    );
    let first = first.unwrap();
    assert_eq!(first.state, OperationState::Completed);
    assert_eq!(store.complete_native_stream(&receipt).unwrap(), first);
}

// Controlled trusted-host witness for Store tuple machinery only. These tests
// do not claim Core routed Data; the separate Core suite owns real token origin.
struct ControlledObservation {
    cid: String,
    sid: String,
    operation: String,
    record: String,
    parent: String,
    logical: String,
    runtime: String,
    generation: u64,
    expected: u64,
    revision: u64,
}
impl crate::NativeDataSubscriptionObservation for ControlledObservation {
    fn matches(&self, s: &crate::NativeDataSubscriptionScope<'_>) -> bool {
        self.cid == s.conversation_id
            && self.sid == s.session_id
            && self.operation == s.operation_id
            && self.record == s.record_id
            && self.parent == s.parent_record_id
            && self.logical == s.connection_id
            && self.runtime == s.runtime_connection_id
            && self.generation == s.generation
            && self.expected == s.expected_revision
            && self.revision == s.stream_revision
            && s.selected_stream_ids.len() == 1
            && s.selected_stream_ids[0] == "application"
    }
}
fn descriptor_fixture(
    mode: &str,
) -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    crate::NativeStreamCommitted,
    crate::PeerAuthorityGuard,
) {
    let (f, mut request, _, origin) =
        parent_permissions_origin_filtered_media(true, true, false, true, true, &[], &[], true);
    let expected = if mode == "stale" { 2 } else { 1 };
    if mode != "absent" {
        let writer = f.controlled_conversation_writer(0);
        let c = f.store.connection_with_authority(&writer).unwrap();
        let generation = if mode == "foreign-generation" { 2 } else { 1 };
        let revision = if mode == "newer" { 2 } else { 1 };
        let mut bindings = vec![binding(false), binding(true)];
        for b in &mut bindings {
            b.connection_generation = generation;
        }
        crate::native_stream_projection::complete(
            &c,
            origin.connection_id(),
            generation,
            1,
            revision,
            Some(&bindings),
        )
        .unwrap();
        if mode == "stale" {
            // Genuine registered broad Writer establishes controlled old active
            // observation, then revision drift makes it invisible. No IO/gate
            // execution or handoff is claimed by this metadata fixture.
            crate::native_stream_projection::gates(
                &c,
                origin.connection_id(),
                1,
                1,
                1,
                &[GateStream {
                    binding: binding(false),
                    state: "active".into(),
                    source_muted: true,
                    held_direction: None,
                }],
            )
            .unwrap();
            c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',2) WHERE id=?1",[origin.connection_id()]).unwrap();
        }
        if mode == "unavailable" {
            c.execute(
                "UPDATE native_stream_snapshots SET available=0 WHERE connection_id=?1",
                [origin.connection_id()],
            )
            .unwrap();
        }
        if mode == "malformed" {
            c.execute("UPDATE native_stream_snapshots SET projection_json=json_set(projection_json,'$.stream_metadata[0].unexpected',1) WHERE connection_id=?1",[origin.connection_id()]).unwrap();
        }
    }
    if matches!(
        mode,
        "missing-envelope" | "missing-metadata" | "missing-counter"
    ) {
        let writer = f.controlled_conversation_writer(0);
        let c = f.store.connection_with_authority(&writer).unwrap();
        let path = if mode == "missing-envelope" {
            "$.stream_metadata_generation"
        } else if mode == "missing-metadata" {
            "$.stream_metadata"
        } else {
            "$.stream_metadata_counter"
        };
        c.execute("UPDATE native_stream_snapshots SET projection_json=json_remove(projection_json,?2) WHERE connection_id=?1",params![origin.connection_id(),path]).unwrap();
    }
    let (operation, cid, sid) = {
        let PeerCommand::NativeStream(wire) = &mut request.command else {
            panic!()
        };
        wire.operation_id = "data-descriptor-capture".into();
        wire.command = Command::Subscribe;
        wire.expected_revision = Some(expected);
        wire.payload = json!({"subscriptions":[{"strm_id":"application"}]});
        (
            wire.operation_id.clone(),
            wire.conversation_id.clone(),
            wire.session_id.clone(),
        )
    };
    let original = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &original)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &original)
        .unwrap()
        .unwrap();
    let witness = Arc::new(ControlledObservation {
        cid,
        sid,
        operation,
        record: lease.record_id().into(),
        parent: lease.parent_record_id().into(),
        logical: lease.connection_id().into(),
        runtime: lease.runtime_connection_id().into(),
        generation: lease.generation(),
        expected: lease.expected_revision(),
        revision: expected + 1,
    });
    let mut observed_bindings = vec![binding(false), binding(true)];
    if mode == "current-av-change" {
        observed_bindings[0].stream_generation = 2;
    }
    let receipt = lease
        .committed_with_unchanged_av(expected + 1, false, Some(observed_bindings), witness)
        .unwrap();
    (f, origin, receipt, original)
}
// Use the same actual row fields and attributed readiness projector as the
// production event snapshot. PreparedConnection is not an event-state view.
fn recorded_connection_state(
    c: &Connection,
    projection: &crate::PeerConnectionProjection,
) -> connect_protocol::events::ConnectionState {
    let actual = c.query_row(
        "SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces,n.readiness_json,n.observed_runtime_id FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",
        [&projection.connection.connection_id],
        |r| Ok(connect_protocol::events::ConnectionState {
            connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,
            membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,
            generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?,
            stream_revision:crate::events::stream_revision(r.get(8)?,r.get(10)?,r.get(11)?)?,
        }),
    ).unwrap();
    assert_eq!(actual.connection_id, projection.connection.connection_id);
    assert_eq!(actual.session_id, projection.connection.session_id);
    assert_eq!(actual.participant_id, projection.connection.participant_id);
    assert_eq!(
        actual.membership_epoch,
        projection.connection.membership_epoch
    );
    assert_eq!(actual.endpoint_id, projection.connection.endpoint_id);
    assert_eq!(actual.device_id, projection.device_id);
    assert_eq!(actual.generation, projection.connection.generation);
    actual
}
#[test]
fn data_subscription_descriptor_insert_stale_recovery_and_victims_stay_exact_and_nongrant() {
    for mode in [
        "absent",
        "current",
        "stale",
        "newer",
        "foreign-generation",
        "unavailable",
        "malformed",
        "current-av-change",
        "missing-envelope",
        "missing-metadata",
        "missing-counter",
    ] {
        let (f, origin, receipt, original) = descriptor_fixture(mode);
        let io = f.store.try_native_media_read(&origin, None).unwrap();
        let before = database(&f.store);
        let observed = {
            let c = f.store.connection().unwrap();
            let parent = recorded_connection_state(&c, io.target());
            crate::native_stream_projection::snapshot(&c, &parent)
        };
        let planned = {
            let c = f.store.connection().unwrap();
            receipt.data_control_plan(&f.store, &c).unwrap()
        };
        if matches!(
            mode,
            "newer"
                | "foreign-generation"
                | "unavailable"
                | "malformed"
                | "current-av-change"
                | "missing-envelope"
                | "missing-metadata"
                | "missing-counter"
        ) {
            assert!(
                planned.is_none(),
                "unsupported descriptor gained a positive row permit: {mode}"
            );
            drop(io);
            drop(original);
            continue;
        }
        if mode != "current" {
            assert!(
                observed.unwrap().is_empty(),
                "old descriptor must really be invisible"
            );
        }
        let plan = planned.unwrap();
        assert_eq!(
            plan.descriptor,
            if mode == "absent" {
                "first"
            } else if mode == "stale" {
                "stale-invisible"
            } else {
                "current"
            }
        );
        let proof = f.store.try_acquire_authority(&plan.required).unwrap();
        if mode == "absent" {
            let mut c = f.store.connection().unwrap();
            c.bind_available_mutation_authority(&f.store, &proof)
                .unwrap();
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            tx.pragma_update(None, "recursive_triggers", false).unwrap();
            let (permit, plan) = f
                .store
                .authority_writer_boundary
                .native_data_control_ledger(&f.store, &tx, &proof, &receipt)
                .unwrap()
                .unwrap();
            let mut reached = false;
            for change in &plan.rows {
                change.apply(&tx).unwrap();
                if change.snapshot_insert() {
                    reached = true;
                    break;
                }
                assert!(!change.journal);
            }
            assert!(
                reached,
                "actual missing descriptor did not receive its single exact INSERT"
            );
            assert!(
                permit.require_data_control_complete().is_err(),
                "partial tuple use cannot authorize COMMIT"
            );
            let count: usize = tx
                .query_row(
                    "SELECT COUNT(*) FROM native_stream_snapshots WHERE connection_id=?1",
                    [origin.connection_id()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
            assert!(tx.execute("INSERT OR REPLACE INTO native_stream_snapshots SELECT connection_id,generation,stream_revision,available,projection_json FROM native_stream_snapshots WHERE connection_id=?1",[origin.connection_id()]).is_err(),"identical tuple replay deleted an already inserted victim");
            let count: usize = tx
                .query_row(
                    "SELECT COUNT(*) FROM native_stream_snapshots WHERE connection_id=?1",
                    [origin.connection_id()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
            drop(permit);
            tx.rollback().unwrap();
            drop(c);
            assert_eq!(database(&f.store), before);
        }
        drop(proof);
        let result = f.store.complete_native_stream(&receipt).unwrap();
        assert_eq!(result.state, OperationState::Completed);
        assert!(io.current());
        let after = {
            let c = f.store.connection().unwrap();
            let parent = recorded_connection_state(&c, io.target());
            crate::native_stream_projection::snapshot(&c, &parent).unwrap()
        };
        assert!(!after.is_empty());
        if mode == "stale" {
            assert!(
                after.iter().all(|s| s.state == "negotiated"),
                "invisible old active observation was improperly carried as current"
            );
        }
        drop(io);
        drop(original);
        assert_eq!(f.store.complete_native_stream(&receipt).unwrap(), result);
    }
}
