//! Controlled Store source/outcome metadata only, never actual RTC/media proof.
use super::*;
use crate::{NativeStreamLease, PeerCommand};
use connect_protocol::native_streams::{Command, Intent};

fn offer(version: u64, tag: &str) -> String {
    format!("v=0\r\no=- 111 {version} IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 0.0.0.0\r\na=ice-ufrag:{tag}U\r\na=ice-pwd:{tag}PRIVATEPASSWORD1234567890\r\na=fingerprint:sha-256 SAME-IDENTITY\r\na=setup:actpass\r\na=mid:0\r\na=sendrecv\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\n")
}
fn local(version: u64, tag: &str) -> String {
    offer(version, tag)
        .replace("o=- 111", "o=- 222")
        .replace("SAME-IDENTITY", "SAME-LOCAL-IDENTITY")
        .replace("setup:actpass", "setup:active")
}
fn fixture() -> (Fixture, PeerAuthorityRequest, String) {
    let mut f = Fixture::new();
    let identity = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    for operation in ["connection.update", "operation.cancel"] {
        if !scope.operations.iter().any(|v| v == operation) {
            scope.operations.push(operation.into());
        }
    }
    f.keys[0] = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
    let endpoint = extra_endpoint_for_transport(&f, EndpointTransport::Webrtc);
    verify_web_rtc(&f, &endpoint);
    let mut request = web_rtc_request(&f);
    let PeerCommand::NativeConnectionOffer(wire) = &mut request.command else {
        panic!()
    };
    wire.offer["substrate_setup"]["sdp"] = json!(offer(1, "old"));
    let proof = f.admit(&request);
    f.store
        .prepare_native_offer_with_peer(&f.keys[0], &proof, Some(EndpointTransport::Webrtc))
        .unwrap();
    let (parent, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    let record = parent.record_id().to_owned();
    // This is controlled source metadata, matching the existing native stream
    // custody fixtures. No runtime readiness or completion method is claimed.
    let mut c = f.store.connection().unwrap();
    let tx = c
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
        [parent.effect_id()],
    )
    .unwrap();
    tx.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1",params![parent.connection_id(),f.store.runtime_epoch(),crate::now(),json!({"streams":["audio"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
    tx.execute(
        "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
        [parent.record_id()],
    )
    .unwrap();
    tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![parent.record_id(),parent.connection_id(),f.store.runtime_epoch(),parent.generation()]).unwrap();
    tx.commit().unwrap();
    drop(c);
    drop(proof);
    let PeerCommand::NativeConnectionOffer(wire) = &request.command else {
        panic!()
    };
    request.command = PeerCommand::NativeStream(Intent {
        operation_id: "ice-first".into(),
        conversation_id: wire.conversation_id.clone(),
        session_id: wire.session_id.clone(),
        connection_id: wire.connection_id.clone(),
        expected_revision: Some(1),
        by_participant: None,
        command: Command::Update,
        payload: json!({"action":"ice-restart","details":{"expected_ice_generation":0,"setup":{"kind":"websocket+webrtc","sdp":offer(2,"new")}}}),
    });
    (f, request, record)
}
fn begin(f: &Fixture, request: &PeerAuthorityRequest) -> NativeStreamLease {
    let proof = f.admit(request);
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    f.store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap()
        .0
}
fn counter(f: &Fixture, parent: &str) -> u64 {
    f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT ice_generation FROM native_webrtc_ice_state WHERE parent_record_id=?1",
            [parent],
            |r| r.get(0),
        )
        .unwrap()
}
fn prepare(
    f: &Fixture,
    lease: &mut NativeStreamLease,
    old_local: &str,
    remote: &str,
    answer: &str,
) {
    f.store
        .capture_native_ice_baseline(lease, old_local, remote)
        .unwrap();
    f.store.capture_native_ice_answer(lease, answer).unwrap();
}
#[test]
fn answer_is_private_original_replay_metadata_and_does_not_complete_a_generation() {
    let (f, request, parent) = fixture();
    let mut lease = begin(&f, &request);
    let before = f.count("operation_effects");
    assert_eq!(counter(&f, &parent), 0);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .native_ice_answer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        Some(local(2, "next"))
    );
    let record = f
        .store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none());
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(f.count("operation_effects"), before);
    assert_eq!(counter(&f, &parent), 0);
    assert!(!format!("{lease:?}{:?}", lease.ice_restart()).contains("PRIVATEPASSWORD"));
    f.store
        .capture_native_ice_answer(&mut lease, &local(2, "next"))
        .unwrap();
    assert!(f
        .store
        .capture_native_ice_answer(&mut lease, &local(3, "other"))
        .is_err());
}
#[test]
fn captured_original_receipts_advance_only_ice_and_old_replay_cannot_reset_a_new_generation() {
    let (f, request, parent) = fixture();
    let read = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id='connection-native'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    let first = lease.ice_committed(1).unwrap(); // Controlled trusted-host outcome, not actual ICE evidence.
    let done = f.store.complete_native_stream(&first).unwrap();
    assert_eq!(counter(&f, &parent), 1);
    assert_eq!(done.result.as_ref().unwrap()["stream_revision"], 1);
    assert!(!serde_json::to_string(&done)
        .unwrap()
        .contains("PRIVATEPASSWORD"));
    let mut next = request.clone();
    let PeerCommand::NativeStream(wire) = &mut next.command else {
        panic!()
    };
    wire.operation_id = "ice-second".into();
    wire.payload["details"]["expected_ice_generation"] = json!(1);
    wire.payload["details"]["setup"]["sdp"] = json!(offer(3, "third"));
    let mut lease = begin(&f, &next);
    prepare(
        &f,
        &mut lease,
        &local(2, "next"),
        &offer(2, "new"),
        &local(3, "later"),
    );
    f.store
        .complete_native_stream(&lease.ice_committed(2).unwrap())
        .unwrap();
    assert_eq!(counter(&f, &parent), 2);
    assert_eq!(f.store.complete_native_stream(&first).unwrap(), done);
    assert_eq!(counter(&f, &parent), 2);
    let after = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT readiness_json FROM connections WHERE id='connection-native'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(read, after);
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .native_ice_answer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        Some(local(2, "next"))
    );
}
#[test]
fn competing_or_changed_original_intent_cannot_allocate_another_attempt() {
    let (f, request, parent) = fixture();
    let _lease = begin(&f, &request);
    let before = f.count("operation_effects");
    for id in ["ice-first", "competing"] {
        let mut changed = request.clone();
        let PeerCommand::NativeStream(wire) = &mut changed.command else {
            panic!()
        };
        wire.operation_id = id.into();
        wire.payload["details"]["setup"]["sdp"] = json!(offer(3, "other"));
        let proof = f.admit(&changed);
        assert!(f
            .store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .is_err());
    }
    assert_eq!(f.count("operation_effects"), before);
    assert_eq!(counter(&f, &parent), 0);
}
#[test]
fn an_uncertain_original_keeps_its_answer_and_cannot_be_begun_again() {
    let (f, request, parent) = fixture();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    f.store.native_stream_attempt_uncertain(&lease).unwrap();
    let proof = f.admit(&request);
    assert_eq!(
        f.store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .state,
        OperationState::Unknown
    );
    assert!(f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    assert_eq!(
        f.store
            .native_ice_answer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        Some(local(2, "next"))
    );
    assert_eq!(counter(&f, &parent), 0);
    f.store
        .complete_native_stream(&lease.ice_committed(1).unwrap())
        .unwrap();
    assert_eq!(counter(&f, &parent), 1);
}
#[test]
fn sql_cannot_replace_original_sdp_or_claim_an_unconfirmed_generation() {
    let (f, request, parent) = fixture();
    let lease = begin(&f, &request);
    let c = f.store.connection().unwrap();
    for sql in [
        "UPDATE native_webrtc_ice_operations SET proposed_remote_offer=previous_remote_offer",
        "UPDATE native_webrtc_ice_operations SET committed=1",
        "DELETE FROM native_webrtc_ice_operations",
        "DELETE FROM native_webrtc_ice_state",
        "UPDATE native_webrtc_ice_state SET ice_generation=1",
    ] {
        assert!(c.execute(sql, []).is_err(), "{sql}");
    }
    drop(c);
    assert_eq!(counter(&f, &parent), 0);
    assert!(lease.ice_committed(1).is_err());
}
#[test]
fn a_retained_answer_cannot_be_reclassified_as_no_effect() {
    let (f, request, parent) = fixture();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    let receipt = lease.without_effect(connect_protocol::Error::new(
        "fixture",
        "not a no-effect outcome",
    ));
    assert!(f
        .store
        .record_native_stream_without_effect(&receipt)
        .is_err());
    assert_eq!(counter(&f, &parent), 0);
}
#[test]
fn backup_digest_covers_private_original_negotiation_bytes() {
    let (f, request, _) = fixture();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    let directory = tempfile::tempdir().unwrap();
    let backup = directory.path().join("ice-backup.sqlite3");
    let saved = f.store.backup(&backup).unwrap();
    assert_eq!(saved.schema, 116);
    assert_eq!(saved, Store::inspect_snapshot(&backup).unwrap());
    let c = Connection::open(&backup).unwrap();
    c.execute("DROP TRIGGER native_webrtc_ice_operation_immutable", [])
        .unwrap();
    c.execute("UPDATE native_webrtc_ice_operations SET answer_sdp=replace(answer_sdp,'nextPRIVATEPASSWORD','changedPRIVATEPASSWORD')",[]).unwrap();
    drop(c);
    assert_ne!(
        saved.digest,
        Store::inspect_snapshot(&backup).unwrap().digest
    );
}
#[test]
fn current_membership_withdrawal_refuses_a_retained_answer_even_with_the_old_proof() {
    let (f, request, _) = fixture();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    let proof = f.admit(&request);
    assert!(f
        .store
        .native_ice_answer_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_some());
    // Controlled current-authority fault, not a claim about the membership API.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&f.pids[0]],
        )
        .unwrap();
    assert!(f
        .store
        .native_ice_answer_with_peer(&f.keys[0], &proof)
        .is_err());
    drop(proof);
    assert!(f
        .store
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(1)))
        .is_err());
}
#[test]
fn a_restored_runtime_recovers_original_unknown_metadata_without_a_new_attempt() {
    let (f, request, _) = fixture();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    let directory = tempfile::tempdir().unwrap();
    let backup = directory.path().join("restored.sqlite3");
    f.store.backup(&backup).unwrap();
    let restored = Store::open(&backup, Limits::default()).unwrap();
    assert_ne!(restored.runtime_epoch(), f.store.runtime_epoch());
    let proof = restored
        .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(1)))
        .unwrap();
    assert_eq!(
        restored
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .unwrap()
            .state,
        OperationState::Unknown
    );
    assert_eq!(
        restored
            .native_ice_answer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        Some(local(2, "next"))
    );
    assert!(restored
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .is_none());
    let c = restored.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM operation_effects WHERE provider='native-stream'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    drop(c);
    assert!(restored
        .capture_native_ice_answer(&mut lease, &local(2, "next"))
        .is_err());
}
#[test]
fn stale_counter_and_changed_media_refuse_without_leaving_new_operation_or_effect_rows() {
    let (f, request, parent) = fixture();
    let mut lease = begin(&f, &request);
    prepare(
        &f,
        &mut lease,
        &local(1, "first"),
        &offer(1, "old"),
        &local(2, "next"),
    );
    f.store
        .complete_native_stream(&lease.ice_committed(1).unwrap())
        .unwrap();
    let before = (f.count("operations"), f.count("operation_effects"));
    for changed_media in [false, true] {
        let mut next = request.clone();
        let PeerCommand::NativeStream(wire) = &mut next.command else {
            panic!()
        };
        wire.operation_id = "refused-new-restart".into();
        if changed_media {
            wire.payload["details"]["expected_ice_generation"] = json!(1);
            wire.payload["details"]["setup"]["sdp"] =
                json!(offer(3, "third").replace("sendrecv", "inactive"));
        }
        let proof = f.admit(&next);
        assert!(f
            .store
            .accept_native_stream_with_peer(&f.keys[0], &proof)
            .is_err());
        assert_eq!(
            (f.count("operations"), f.count("operation_effects")),
            before
        );
    }
    assert_eq!(counter(&f, &parent), 1);
}

#[test]
fn continuation_reacquires_only_the_original_attempt_and_retains_no_authority_reader() {
    let (f, request, _) = fixture();
    let lease = begin(&f, &request);
    let proof = f.admit(&request);
    let origin = f
        .store
        .native_ice_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    let expiry = proof.expires_at().min(lease.expires_at());
    drop(proof);
    let effects = f.count("operation_effects");
    for _ in 0..2 {
        let current = f
            .store
            .resume_native_ice_authority(
                &f.keys[0],
                &origin,
                &lease,
                &AuthorityWait::new(Duration::from_secs(1)),
            )
            .unwrap();
        assert!(current.current());
        assert!(current.expires_at() <= expiry);
        drop(current);
    }
    // The origin retains no scope reader: the same domain can acquire writers
    // while the custodian is idle between observations.
    let held = f
        .store
        .acquire_authority(
            &[crate::AuthorityScope::identity(
                &f.store.revalidate(&f.keys[0]).unwrap().identity_id,
                crate::AuthorityMode::Write,
            )
            .unwrap()],
            &AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    drop(held);
    assert_eq!(f.count("operation_effects"), effects);
    assert!(!format!("{origin:?}").contains("PRIVATEPASSWORD"));
    assert!(f
        .store
        .resume_native_ice_authority(
            &f.keys[1],
            &origin,
            &lease,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
}

#[test]
fn continuation_denies_membership_withdrawal_and_an_unknown_attempt() {
    let (f, request, _) = fixture();
    let lease = begin(&f, &request);
    let proof = f.admit(&request);
    let origin = f
        .store
        .native_ice_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    drop(proof);
    f.store.native_stream_attempt_uncertain(&lease).unwrap();
    assert!(f
        .store
        .resume_native_ice_authority(
            &f.keys[0],
            &origin,
            &lease,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());

    let (f, request, _) = fixture();
    let lease = begin(&f, &request);
    let proof = f.admit(&request);
    let origin = f
        .store
        .native_ice_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    drop(proof);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&f.pids[0]],
        )
        .unwrap();
    assert!(f
        .store
        .resume_native_ice_authority(
            &f.keys[0],
            &origin,
            &lease,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
}

#[test]
fn continuation_cannot_cross_original_effects_or_installation_runtimes() {
    let (f, request, _) = fixture();
    let lease = begin(&f, &request);
    let proof = f.admit(&request);
    let origin = f
        .store
        .native_ice_origin_with_peer(&f.keys[0], &proof, &lease)
        .unwrap();
    drop(proof);
    let (other, other_request, _) = fixture();
    let other_lease = begin(&other, &other_request);
    assert!(f
        .store
        .resume_native_ice_authority(
            &f.keys[0],
            &origin,
            &other_lease,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("restored.sqlite3");
    f.store.backup(&path).unwrap();
    let restored = Store::open(&path, Limits::default()).unwrap();
    assert!(restored
        .resume_native_ice_authority(
            &f.keys[0],
            &origin,
            &lease,
            &AuthorityWait::new(Duration::from_secs(1))
        )
        .is_err());
}
