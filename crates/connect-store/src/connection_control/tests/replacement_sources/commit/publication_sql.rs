//! Actual public native VP8 entitlement/IO custody, not sent or decoded video.
use super::positive_capture::{admit, publication};
use super::*;
use std::time::{Duration, Instant};

fn video_sibling(f: &Fixture) -> Arc<NativeMediaOrigin> {
    // A different, ordinary public Session in the same CID. Create/accept the
    // full AV offer normally; the existing native ready helper supplies only
    // controlled Store observation, never decoded-readiness evidence.
    let session = "publication-video-session";
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "publication-video-invite".into(),
                    conversation_id: f.cid.clone(),
                    session_id: session.into(),
                    by_participant: None,
                    expected_revision: None,
                    action: Action::Invite {
                        participants: vec![f.pids[2].clone()],
                        medium: "voice+video".into(),
                        intent: "conference".into(),
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[2],
            &Operation::SessionControl {
                request: Request {
                    operation_id: "publication-video-accept".into(),
                    conversation_id: f.cid.clone(),
                    session_id: session.into(),
                    by_participant: None,
                    expected_revision: Some(0),
                    action: Action::Accept {
                        capabilities: json!({"audio":["opus"],"video":["vp8"]}),
                    },
                },
            },
        )
        .unwrap();
    let mut request = f.native_request_for(2, 600, "publication-video-sibling");
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.session_id = session.into();
    intent.offer["streams_offered"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"video","kind":"video","direction":"sendrecv","codec_preferences":["vp8"]
        }));
    ready_video_request(f, 2, request)
}
// Exact existing native ready fixture, with both actual offered stream IDs
// in its controlled Store observation. No physical/decoded readiness claim.
fn ready_video_request(
    f: &Fixture,
    actor: usize,
    request: crate::PeerAuthorityRequest,
) -> Arc<NativeMediaOrigin> {
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(
            &f.keys[actor],
            &proof,
            Some(connect_protocol::provisioning::EndpointTransport::Quic),
        )
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let origin = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &lease)
        .unwrap();
    drop(proof);
    let writer = f.controlled_conversation_writer(actor);
    let mut c = f.store.connection_with_authority(&writer).unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [lease.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![lease.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":["audio","video"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
        [lease.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    tx.commit().unwrap();
    origin
}
fn metadata<T>(mut call: impl FnMut() -> crate::Result<T>) -> crate::Result<T> {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match call() {
            Err(e) if e.code == "authority-busy" && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

#[test]
fn public_activation_completed_writer_keeps_primary_sql_through_publication() {
    let m = fixture(true);
    let capture = activation_barrier::captured(&m);
    let source = video_sibling(&m.f);
    let store = Arc::new(m.f.store);
    let (snapshot, mut watcher) = publication(&store, &source);
    let io = admit(&snapshot);
    let has_video = io.target().offer["streams_offered"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| {
            s["kind"] == "video"
                && s["codec_preferences"]
                    .as_array()
                    .is_some_and(|v| v.iter().any(|c| c == "vp8"))
        });
    let stamp = io.snapshot_stamp().unwrap();
    let epoch = io.packet_epoch().unwrap();
    let media = &io.target().session.session.media;
    let session_av = media.iter().any(|v| v == "audio") && media.iter().any(|v| v == "video");
    let ready_video: bool = store.connection().unwrap().query_row(
        "SELECT state='ready' AND observed_runtime_id=?2 AND EXISTS(SELECT 1 FROM json_each(readiness_json,'$.streams') WHERE value='video') FROM connections WHERE id=?1",
        params![source.connection_id(),store.runtime_epoch()], |r|r.get(0)).unwrap();
    let initially_current = io.current() && epoch.current();
    // A queued epoch never pins IO fences. Immediate failed Writer attempts may
    // already revoke it; fixed success below claims completed-Writer SQL custody
    // and fresh sibling authorization, NEVER survival across those prior cuts.
    drop(io);
    let before = Store::inspect_snapshot(store.path()).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    let (entered, ready) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel(1);
    let admission_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let pre_admission_refusals = std::sync::atomic::AtomicUsize::new(0);
    let (
        arrived,
        sql_fault,
        sql_error,
        observer_current,
        queued_current,
        stamp_current,
        unchanged,
        result,
    ) = std::thread::scope(|workers| {
        let worker = workers.spawn(|| {
            let probe_admission = admission_seen.clone();
            crate::authority_admission::with_passive_admission_probe(
                move |admitted| {
                    probe_admission.store(admitted, std::sync::atomic::Ordering::SeqCst);
                    let _ = entered.send(admitted);
                    let _ = resume.recv_timeout(until.saturating_duration_since(Instant::now()));
                },
                || {
                    let mut last_busy = None;
                    loop {
                    if Instant::now() >= until {
                        return Err(last_busy.expect("original counter cutoff elapsed before any Store attempt"));
                    }
                    let result = store.project_native_replacement_committed(&capture, &MetadataOnlyInstalled);
                    match result {
                        Err(e) if e.code == "authority-busy"
                            && !admission_seen.load(std::sync::atomic::Ordering::SeqCst)
                            && Instant::now() < until => {
                                let prior = pre_admission_refusals.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                                // No retry hides this first/prior failed attempt:
                                // it may be kernel Busy OR the post-file5mscheck.
                                eprintln!("public activation pre-complete attempt={prior} error_message={:?} activation_diagnostics={}", e.message, capture.projection_diagnostics()["activation"]);
                                // Returned Store call has dropped its entire
                                // Unit/partialproof; same receipt/cutoff only.
                                last_busy = Some(e);
                                std::thread::sleep(Duration::from_millis(20).min(until.saturating_duration_since(Instant::now())));
                            }
                        // Once ANY actual complete Writer checkpoint occurred,
                        // stop forever, even its SQL-fault Busy. No publication
                        // or Core effect is replayed after that checkpoint.
                        result => break result,
                    }
                    }
                },
            )
        });
        let arrived = ready.recv_timeout(Duration::from_secs(1)) == Ok(true);
        // OLD: primary Unit was released before this actual Writer. Inject real
        // SQL exclusion now. FIXED: the exact Unit is still owned, so this try
        // must refuse; releasing the rendezvous permits a legitimate commit.
        let sql_result = if arrived {
            Some(store.try_connection())
        } else {
            None
        };
        let sql_error = sql_result
            .as_ref()
            .and_then(|v| v.as_ref().err())
            .map(|e| e.code.clone());
        let sql = sql_result.and_then(Result::ok);
        let sql_fault = sql.is_some();
        let _ = release.send(());
        // OLD returns definite Busy while SQL remains held. FIXED cannot have
        // injected SQL here, and can finish the same exact publication.
        let result = worker.join();
        // The failed old call has dropped its full Writer. While primary SQL
        // remains excluded, the independent packet observer can freshly prove
        // the unchanged sibling; it cannot revive the captured original epoch.
        let observer = store.try_native_media_read(&source, None);
        let observer_current = observer.as_ref().is_ok_and(|v| v.current());
        drop(observer);
        let queued_current = epoch.current();
        let original = snapshot.admit_original(stamp);
        let stamp_current = original.as_ref().is_ok_and(|v| v.current());
        drop(original);
        let unchanged = Store::inspect_snapshot(store.path()).map(|v| v == before);
        drop(sql);
        (
            arrived,
            sql_fault,
            sql_error,
            observer_current,
            queued_current,
            stamp_current,
            unchanged,
            result,
        )
    });
    let original_after = store.try_native_media_read(&source, None);
    let sibling_still_current = original_after.as_ref().is_ok_and(|v| v.current());
    drop(original_after);
    watcher.shutdown().unwrap();
    let pre_admission_refusals = pre_admission_refusals.load(std::sync::atomic::Ordering::SeqCst);
    eprintln!("public activation SQL counter pre_admission_refusals={pre_admission_refusals} admitted={arrived} primary_fault_acquired={sql_fault} sql_error={sql_error:?} observer_current={observer_current} queued_current={queued_current} original_stamp_current={stamp_current} same_snapshot={unchanged:?} result_code={:?}", result.as_ref().ok().and_then(|r|r.as_ref().err()).map(|e|e.code.as_str()));
    // Every pre-complete refusal was printed at its actual boundary above.
    // This is the final joined outcome, never a claim those epochs survived.
    eprintln!(
        "public activation completed-writer outcome error_message={:?} activation_diagnostics={}",
        result
            .as_ref()
            .ok()
            .and_then(|r| r.as_ref().err())
            .map(|e| e.message.as_str()),
        capture.projection_diagnostics()["activation"]
    );
    assert!(
        has_video && session_av && ready_video && initially_current,
        "counter requires actual public VP8 Store IO/stamp/epoch before the fault"
    );
    assert!(
        arrived,
        "counter never reached an actual complete Writer admission"
    );
    let result = result.expect("original projection owner panicked");
    if sql_fault {
        assert_eq!(pre_admission_refusals, 0, "old SQL-fault causal oracle is confounded by an earlier Writer attempt; old38 singleattempt proof is retained separately");
        assert!(
            observer_current,
            "primary exclusion must leave the independent observer usable"
        );
        assert_eq!(
            result.as_ref().err().map(|e| e.code.as_str()),
            Some("authority-busy")
        );
        assert!(
            unchanged.unwrap(),
            "pre-TX SQL refusal changed durable state"
        );
        assert!(queued_current && stamp_current, "pre-TX final activation SQL reacquire revoked unaffected public video epoch without any publication");
    } else {
        assert_eq!(
            sql_error.as_deref(),
            Some("authority-busy"),
            "only exact retained SQL custody may prevent interleaving"
        );
        assert!(
            result.is_ok(),
            "same-Unit public activation failed after its original Writer admission: {:?}",
            result.err()
        );
        assert!(
            !unchanged.unwrap(),
            "legitimate public activation must actually publish metadata"
        );
        assert!(sibling_still_current, "unaffected public sibling cannot lose current authorization after legitimate publication");
    }
}

#[test]
fn public_activation_immediate_writer_releases_sql_on_real_reader_busy_and_progresses_after_drain()
{
    let m = fixture(true);
    let capture = activation_barrier::captured(&m);
    let held =
        m.f.store
            .try_native_media_read(&m.sources[1], None)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let result =
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled);
    let held_current = held.current();
    let sql_result = m.f.store.try_connection();
    let sql_released = sql_result.is_ok();
    drop(sql_result);
    let unchanged = Store::inspect_snapshot(m.f.store.path()).unwrap() == before;
    drop(held);
    // A fresh call retries this immutable metadata only. No Core, new operation
    // identity or physical effect is executed by this Store fixture.
    let projected = metadata(|| {
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
    });
    assert_eq!(
        result.as_ref().err().map(|e| e.code.as_str()),
        Some("authority-busy")
    );
    assert!(
        held_current && sql_released && unchanged,
        "reader refusal retained SQL, lost current authorization, or changed metadata"
    );
    let projected = projected.unwrap();
    assert!(m
        .f
        .store
        .try_native_media_read(&projected, None)
        .unwrap()
        .current());
    assert!(
        m.f.store
            .try_native_media_read(&m.sources[0], None)
            .is_err(),
        "real old-source retirement must remain effective"
    );
    let published = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let repeated = metadata(|| {
        m.f.store
            .project_native_replacement_committed(&capture, &MetadataOnlyInstalled)
    })
    .unwrap();
    assert_eq!(
        repeated.runtime_connection_id(),
        projected.runtime_connection_id()
    );
    assert_eq!(
        Store::inspect_snapshot(m.f.store.path()).unwrap(),
        published
    );
}
