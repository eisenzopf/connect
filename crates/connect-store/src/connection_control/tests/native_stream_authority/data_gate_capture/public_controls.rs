//! Public receipt/recorder controls. Controlled Store descriptors and genuine
//! reader/epoch custody only; no Core effect, decoded media or FPS claim.
use super::*;
use connect_protocol::transitions::{OperationAction, OperationRecord};

fn pending() -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    crate::NativeStreamLease,
    crate::PeerAuthorityGuard,
) {
    let (f, mut request, _, origin) =
        parent_permissions_origin_filtered_media(true, true, false, true, true, &[], &[], true);
    {
        let writer = f.controlled_conversation_writer(0);
        let c = f.store.connection_with_authority(&writer).unwrap();
        crate::native_stream_projection::complete(
            &c,
            origin.connection_id(),
            1,
            1,
            1,
            Some(&[binding(false), binding(true)]),
        )
        .unwrap();
    }
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Update;
    wire.operation_id = "captured-public-data-gate".into();
    wire.payload = json!({"action":"mute","streams":["application"]});
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, record) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert_eq!(record.effects.len(), 1);
    (f, origin, lease, proof)
}

fn gate(data: bool) -> Vec<GateStream> {
    vec![GateStream {
        binding: binding(data),
        state: "active".into(),
        source_muted: true,
        held_direction: None,
    }]
}

// The closure owns its proof/readers. Panic and success both drain the closure
// before joining the original watcher; intended assertions use returned facts.
fn with_publication<T>(
    store: &Arc<Store>,
    origin: &Arc<crate::NativeMediaOrigin>,
    work: impl FnOnce(&NativeMediaSnapshot, NativeMediaRead) -> T,
) -> T {
    let weak = Arc::downgrade(store);
    let source = origin.clone();
    let snapshot = store
        .register_native_media_snapshot_for(origin, None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&source, None)
        })
        .unwrap();
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| work(&snapshot, current(&snapshot))));
    let cleanup = watcher.shutdown();
    match result {
        Ok(value) => {
            cleanup.unwrap();
            value
        }
        Err(panic) => {
            let _ = cleanup;
            resume_unwind(panic)
        }
    }
}

// Same immutable receipt; no retry after any epoch cut, changed durable row,
// permanent refusal, or original cutoff. This is not a Core/physical retry.
fn record_before_cutoff(
    store: &Store,
    receipt: &crate::NativeStreamCommitted,
    snapshot: &NativeMediaSnapshot,
    io: &NativeMediaRead,
    proof: Option<&crate::PeerAuthorityGuard>,
    until: Instant,
) -> crate::Result<OperationRecord> {
    let before = database(store);
    let epoch = io.packet_epoch().unwrap();
    let stamp = io.snapshot_stamp().unwrap();
    loop {
        assert!(
            Instant::now() < until,
            "original public-recording cutoff expired"
        );
        assert!(
            proof.is_none_or(|p| p.current()),
            "original command interval expired"
        );
        let result = store.complete_native_stream(receipt);
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "authority-busy")
            && epoch.current()
            && io.current()
            && proof.is_none_or(|p| p.current())
            && database(store) == before
        {
            let same = snapshot.admit_original(stamp);
            if same.as_ref().is_ok_and(|v| v.current()) && Instant::now() < until {
                drop(same);
                std::thread::sleep(
                    Duration::from_millis(1).min(until.saturating_duration_since(Instant::now())),
                );
                continue;
            }
        }
        return result;
    }
}

#[test]
fn completed_data_gate_replay_is_read_only_under_newer_audio_state_and_new_original_epoch() {
    let (f, origin, data, proof) = prepared(Kind::DataGate);
    let mut next = proof.request.clone();
    drop(proof);
    let first = f.store.complete_native_stream(&data).unwrap();
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.operation_id = "newer-public-audio-gate".into();
    wire.expected_revision = Some(2);
    wire.payload = json!({"action":"mute","streams":["audio"]});
    let next_proof = admit(&f, &next).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &next_proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &next_proof)
        .unwrap()
        .unwrap();
    let audio = lease.gates_committed(3, gate(false)).unwrap();
    let store = Arc::new(f.store);
    // This completed Data record is unrelated to the newer Audio operation.
    // Admit its real Reader before the sole watcher starts; this avoids
    // turning background registry contention into a replay setup failure.
    let replay_record_read = store
        .try_acquire_authority(&[crate::AuthorityScope::record(
            &first.record_id,
            crate::AuthorityMode::Read,
        )
        .unwrap()])
        .unwrap();
    let (
        audio_refused,
        old_epoch_cut,
        updated,
        replay,
        current_same_epoch,
        unchanged,
        original_cutoff,
    ) = with_publication(&store, &origin, |snapshot, io| {
        let old_stamp = io.snapshot_stamp().unwrap();
        let old_epoch = io.packet_epoch().unwrap();
        assert!(old_epoch.current() && io.current() && next_proof.current());
        let before = database(&store);
        // One absolute budget covers both epochs and the historical replay.
        // This Store owns exactly ONE watcher, which is joined only after
        // every phase/IO below has drained through this catch boundary.
        let until = Instant::now() + Duration::from_secs(2);
        let result = record_before_cutoff(&store, &audio, snapshot, &io, Some(&next_proof), until);
        let refused = result
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "authority-busy");
        let cut = !old_epoch.current();
        assert!(Instant::now() < until && database(&store) == before);
        drop(io);
        drop(next_proof);

        // The genuine Audio control must finish only after its original
        // readers drain. The still-owned watcher can have a short refresh
        // in flight, so retry SAME metadata receipt only while all durable
        // rows remain unchanged. This repeats no Core/physical effect.
        let updated = loop {
            assert!(
                Instant::now() < until,
                "original public-recording cutoff expired"
            );
            let value = store.complete_native_stream(&audio);
            match value {
                Err(e) if e.code == "authority-busy" && database(&store) == before => {
                    assert!(!old_epoch.current(), "an old fenced epoch was restored");
                    std::thread::sleep(
                        Duration::from_millis(1)
                            .min(until.saturating_duration_since(Instant::now())),
                    );
                }
                other => break other,
            }
        }
        .unwrap();
        assert_eq!(updated.result.as_ref().unwrap()["stream_revision"], 3);
        assert!(!old_epoch.current());

        // The same snapshot's real owner watcher publishes a new epoch.
        // Never restart it: shutdown permanently stops Registry availability.
        let io = loop {
            assert!(
                Instant::now() < until,
                "fresh same-snapshot epoch missed original cutoff"
            );
            match snapshot.admit() {
                Ok(io) => break io,
                Err(e) if e.code == "authority-busy" => std::thread::sleep(
                    Duration::from_millis(1).min(until.saturating_duration_since(Instant::now())),
                ),
                Err(e) => panic!("new original publication refused: {}", e.code),
            }
        };
        let stamp = io.snapshot_stamp().unwrap();
        let epoch = io.packet_epoch().unwrap();
        assert!(
            stamp != old_stamp,
            "same snapshot must publish a NEW incarnation after the real AV fence"
        );
        assert!(epoch.current() && io.current() && !old_epoch.current());
        // The original Data RecordRead remains held through this replay,
        // so even its own RecordWrite must be unnecessary.
        let before_replay = database(&store);
        let audio_before = audio_metadata(&store);
        // Actual external write TX forbids a hidden replay WriteTX.
        let mut external = rusqlite::Connection::open(store.path()).unwrap();
        let writer = external
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        let replay = record_before_cutoff(&store, &data, snapshot, &io, None, until);
        let original = snapshot.admit_original(stamp);
        let healthy = epoch.current()
            && io.current()
            && original.as_ref().is_ok_and(|v| v.current())
            && !old_epoch.current();
        let unchanged = database(&store) == before_replay && audio_metadata(&store) == audio_before;
        let within = Instant::now() < until;
        writer.rollback().unwrap();
        drop(replay_record_read);
        drop(original);
        drop(io);
        (refused, cut, updated, replay, healthy, unchanged, within)
    });
    // ONE watcher has now joined after both real epochs and all SQL/Readers.
    assert!(
        audio_refused && old_epoch_cut,
        "real newer AV gate did not exercise its broad fence"
    );
    assert!(current_same_epoch && unchanged && original_cutoff);
    assert_eq!(replay.unwrap(), first);
    assert_eq!(store.complete_native_stream(&audio).unwrap(), updated);
}

#[test]
fn cancelled_and_unknown_data_gate_captures_keep_real_writer_fencing_before_historical_recording() {
    for cancel in [false, true] {
        let (f, origin, lease, proof) = pending();
        let request = proof.request.clone();
        let record = lease.record_id().to_owned();
        drop(proof);
        if cancel {
            f.store
                .execute_operation_action(
                    &f.keys[0],
                    OperationAction::Cancel {
                        operation_id: "cancel-public-data-gate".into(),
                        target_operation_id: "captured-public-data-gate".into(),
                        reason: None,
                    },
                )
                .unwrap();
        } else {
            assert_eq!(
                f.store
                    .native_stream_attempt_uncertain(&lease)
                    .unwrap()
                    .state,
                OperationState::Unknown
            );
        }
        let (state, cancelled, effect): (String, bool, String) = f.store.connection().unwrap().query_row(
            "SELECT o.state,o.cancel_requested,e.state FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id WHERE o.record_id=?1",[&record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).unwrap();
        assert_eq!(
            (state, cancelled, effect),
            ("unknown".into(), cancel, "unknown".into())
        );
        let proof = admit(&f, &request).unwrap();
        let receipt = lease.gates_committed(2, gate(true)).unwrap();
        let store = Arc::new(f.store);
        let (code, cut, unchanged, within) = with_publication(&store, &origin, |snapshot, io| {
            let epoch = io.packet_epoch().unwrap();
            assert!(io.current() && epoch.current() && proof.current());
            let before = database(&store);
            let until = Instant::now() + Duration::from_secs(2);
            let result = record_before_cutoff(&store, &receipt, snapshot, &io, Some(&proof), until);
            let facts = (
                result.err().map(|e| e.code),
                !epoch.current(),
                database(&store) == before,
                Instant::now() < until,
            );
            drop(io);
            drop(proof);
            facts
        });
        let completed = store.complete_native_stream(&receipt).unwrap();
        assert_eq!(completed.state, OperationState::Completed);
        assert_eq!(completed.cancel_requested, cancel);
        assert_eq!(store.complete_native_stream(&receipt).unwrap(), completed);
        assert_eq!(code.as_deref(), Some("authority-busy"));
        assert!(
            cut && unchanged && within,
            "nonpositive capture borrowed the Data-only positive permit"
        );
    }
}

#[test]
fn data_target_with_audio_observation_cannot_borrow_data_permit_or_change_durable_rows() {
    let (f, origin, lease, proof) = pending();
    let mut wrong = gate(true);
    // Generic gate syntax accepts this, but the actual Data target's saved
    // projection has medium=data. It must not receive a narrow Data permit.
    wrong[0].binding.medium = "audio".into();
    let receipt = lease.gates_committed(2, wrong).unwrap();
    let store = Arc::new(f.store);
    let (code, cut, unchanged, within, before) =
        with_publication(&store, &origin, |snapshot, io| {
            let epoch = io.packet_epoch().unwrap();
            assert!(io.current() && epoch.current() && proof.current());
            let before = database(&store);
            let until = Instant::now() + Duration::from_secs(2);
            let result = record_before_cutoff(&store, &receipt, snapshot, &io, Some(&proof), until);
            let facts = (
                result.err().map(|e| e.code),
                !epoch.current(),
                database(&store) == before,
                Instant::now() < until,
                before,
            );
            drop(io);
            drop(proof);
            facts
        });
    let rejected = store.complete_native_stream(&receipt).unwrap_err();
    assert_eq!(rejected.code, "invalid-observation");
    assert_eq!(
        database(&store),
        before,
        "invalid observed medium left partial durable attribution"
    );
    assert_eq!(code.as_deref(), Some("authority-busy"));
    assert!(cut && unchanged && within);
}

#[test]
fn immutable_data_gate_attempt_forgeries_are_refused_and_original_receipt_still_records_under_readers(
) {
    let (f, origin, lease, proof) = pending();
    let request = proof.request.clone();
    let record = lease.record_id().to_owned();
    drop(proof);
    let before = database(&f.store);
    {
        let c = f.change_fixture();
        for sql in [
            "UPDATE native_stream_operations SET parent_record_id='foreign-parent' WHERE operation_record_id=?1",
            "UPDATE native_stream_operations SET generation=generation+1 WHERE operation_record_id=?1",
            "UPDATE native_stream_operations SET source_expires_at=source_expires_at+1 WHERE operation_record_id=?1",
            "UPDATE native_stream_operations SET attempt_id='foreign-attempt' WHERE operation_record_id=?1",
            "UPDATE native_stream_operations SET attempt_runtime='foreign-runtime' WHERE operation_record_id=?1",
        ] {
            let refused = c.execute(sql, [&record]).unwrap_err();
            assert!(refused.to_string().contains("native stream origin is immutable"));
        }
    }
    assert_eq!(database(&f.store), before);
    let proof = admit(&f, &request).unwrap();
    let receipt = lease.gates_committed(2, gate(true)).unwrap();
    let store = Arc::new(f.store);
    let (recorded, same_epoch, audio_unchanged, within) =
        with_publication(&store, &origin, |snapshot, io| {
            let stamp = io.snapshot_stamp().unwrap();
            let epoch = io.packet_epoch().unwrap();
            assert!(io.current() && epoch.current() && proof.current());
            let audio_before = audio_metadata(&store);
            let until = Instant::now() + Duration::from_secs(2);
            let result = record_before_cutoff(&store, &receipt, snapshot, &io, Some(&proof), until);
            let original = snapshot.admit_original(stamp);
            let facts = (
                result,
                epoch.current() && io.current() && original.as_ref().is_ok_and(|v| v.current()),
                audio_metadata(&store) == audio_before,
                Instant::now() < until,
            );
            drop(original);
            drop(io);
            drop(proof);
            facts
        });
    let recorded = recorded.unwrap();
    assert_eq!(recorded.state, OperationState::Completed);
    assert_eq!(recorded.effects.len(), 1);
    assert!(same_epoch && audio_unchanged && within);
    assert_eq!(store.complete_native_stream(&receipt).unwrap(), recorded);
}
