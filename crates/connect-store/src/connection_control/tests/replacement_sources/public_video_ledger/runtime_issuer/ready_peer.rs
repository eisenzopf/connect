//! Actual Store PeerAuthorityGuard, original capture/start and metadata APIs.
//! Provisioned key/device/credential binding is real; WSS signature verification,
//! native/Core/F/B decode and Host whole-marker custody remain external tests.
use super::*;
struct ReadyFixture {
    scene: Started,
    started: Arc<crate::NativeReplacementStarted>,
    prepared: Arc<crate::NativeReplacementPrepared>,
    request: crate::PeerAuthorityRequest,
}
fn ready_fixture() -> ReadyFixture {
    let (f, key, command, prepared, mut request, sources) = setup(1, None);
    let reservation = retry_effect_free_metadata(&f, || {
        f.store
            .reserve_native_replacement_public_video_intent(prepared.clone())
    })
    .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["webrtc_video_receive_reservation"] =
        serde_json::to_value(reservation.intent()).unwrap();
    let proof = f.admit(&request);
    let (attempt, read) = f
        .store
        .begin_native_replacement_allocation_with_peer(&prepared, &request.credentials[0], &proof)
        .unwrap();
    let expected = attempt.expected_allocation().clone();
    let allocated = Arc::new(attempt.allocated(expected).unwrap());
    drop(read);
    drop(proof);
    f.store
        .record_native_replacement_allocated(&allocated)
        .unwrap();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"]["profile"] = json!("connect-replacement-media-v1");
    let proof = f.admit(&request);
    let read = f
        .store
        .native_replacement_start_read_with_peer(&allocated, &request.credentials[0], &proof)
        .unwrap();
    let (attempt, read, _) = f
        .store
        .begin_native_replacement_start_with_read(allocated, &request.credentials[0], &proof, read)
        .unwrap();
    let expected = attempt.expected_resources().clone();
    let started = Arc::new(attempt.started(expected).unwrap());
    drop(read);
    drop(proof);
    f.store.record_native_replacement_started(&started).unwrap();
    let origin = f
        .store
        .native_replacement_media_origin(started.clone())
        .unwrap();
    let scene = Started {
        f,
        key,
        command,
        reservation,
        origin,
        sources,
    };
    // Controlled Store NNF only; no native reservation/negotiation is inferred.
    issue(
        &scene.f,
        &scene.reservation,
        &scene.command,
        Stage::NegotiatedNeverFed,
    );
    for phase in [
        crate::NativeReplacementMediaPhase::Attach,
        crate::NativeReplacementMediaPhase::Activate,
        crate::NativeReplacementMediaPhase::Readiness,
    ] {
        let read = scene
            .f
            .store
            .try_native_replacement_scene_write(&scene.origin, &scene.sources)
            .unwrap();
        let attempt = scene
            .f
            .store
            .begin_native_replacement_media_phase(scene.origin.clone(), phase, read.media())
            .unwrap();
        let expected = attempt.expected().unwrap();
        let captured = attempt.applied(expected).unwrap();
        drop(read);
        scene
            .f
            .store
            .record_native_replacement_media_phase(&captured)
            .unwrap();
    }
    let fixture = ReadyFixture {
        scene,
        started,
        prepared,
        request,
    };
    assert_eq!(
        readiness_rows(&fixture),
        1,
        "the original private A/audio readiness was actually recorded once"
    );
    fixture
}
fn public_request(m: &ReadyFixture) -> crate::PeerAuthorityRequest {
    let mut request = m.request.clone();
    let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
        panic!()
    };
    intent.offer["replacement_preparation"] = json!({"profile":wire::READY_PROFILE,
        "operation_record_id":m.scene.reservation.operation_record_id(),"public_video_readiness":{
            "reservation_id":m.scene.reservation.intent().reservation_id,"source_cut_id":"controlled-original-cut",
            "nonce":"original-public-challenge","binding":{"stream_id":"public-b","preview_stream_id":"camera-a-0","mid":"2",
                "stream_generation":3,"physical_writer_incarnation":"controlled-store-writer-not-native-proof",
                "connection_generation":7,"primary_ssrc":0,"payload_type":96,"clock_rate_hz":90000,"codec":"vp8"},
            "decoded_frames":1,"decoded_keyframes":null,"decoder_startup":wire::DECODER_STARTUP}});
    request
}
fn readiness_rows(m: &ReadyFixture) -> u64 {
    m.scene.f.store.try_connection().unwrap().query_row(
        "SELECT count(*) FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase='readiness' AND started=1",
        [m.scene.reservation.operation_record_id()], |row| row.get(0),
    ).unwrap()
}
#[test]
fn public_ready_peer_uses_same_complete_original_proof_without_nested_reader_record_scope_or_new_phase(
) {
    let m = ready_fixture();
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    // Real allocation stores Prepared by value. A distinct Arc wrapper of its
    // legitimate value clone must retain SAME original capture provenance.
    let before_clone = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let value_clone = Arc::new(m.prepared.as_ref().clone());
    assert!(!Arc::ptr_eq(&m.prepared, &value_clone));
    let same = retry_effect_free_metadata(&m.scene.f, || {
        m.scene
            .f
            .store
            .reserve_native_replacement_public_video_intent(value_clone.clone())
    })
    .unwrap();
    assert!(m.scene.reservation.same_original(&same));
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before_clone
    );
    let request = public_request(&m);
    let proof = m.scene.f.admit(&request);
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let admissions = m.scene.f.store.authority.test_admission_attempts();
    let read = m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    assert!(proof.current() && read.current());
    assert!(Arc::ptr_eq(
        read.source().retained().interval(),
        Store::peer_scopes(&proof)
    ));
    assert!(read.expires_at() <= m.scene.reservation.expires_at());
    assert!(read.expires_at() <= proof.expires_at());
    assert_eq!(
        read.target().connection.connection_id,
        m.scene.reservation.candidate_connection_id()
    );
    assert_eq!(
        read.target().connection.generation,
        m.scene.reservation.target_generation()
    );
    assert!(
        !m.scene.f.store.authority_covers(
            Store::peer_scopes(&proof),
            &[crate::AuthorityScope::record(
                m.scene.reservation.operation_record_id(),
                crate::AuthorityMode::Read,
            )
            .unwrap()]
        ),
        "media snapshot must not gain a long-lived RecordRead"
    );
    assert_eq!(
        m.scene.f.store.authority.test_admission_attempts(),
        admissions
    );
    drop(read);
    let record = m
        .scene
        .f
        .store
        .replay_native_replacement_public_video_ready_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none() && record.problem.is_none() && record.effects.is_empty());
    assert_eq!(
        m.scene.f.store.authority.test_admission_attempts(),
        admissions
    );
    assert_eq!(
        readiness_rows(&m),
        1,
        "public B does not rerun private A/audio readiness"
    );
    assert_eq!(m.scene.f.count("native_replacement_public_video_phases"), 2);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    // Actual primary SQL owner exclusion is immediate, without nested admission.
    let sql = m.scene.f.store.try_connection().unwrap();
    assert_eq!(
        m.scene
            .f
            .store
            .native_replacement_public_video_ready_read_with_peer(
                &m.started,
                &request.credentials[0],
                &proof
            )
            .err()
            .unwrap()
            .code,
        "authority-busy"
    );
    drop(sql);
    drop(proof);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
}
#[test]
fn public_ready_peer_rejects_original_offer_root_cut_binding_and_marker_contradictions_without_metadata_change(
) {
    let m = ready_fixture();
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    for fault in 0..13 {
        let mut request = public_request(&m);
        let crate::PeerCommand::NativeConnectionOffer(intent) = &mut request.command else {
            panic!()
        };
        match fault {
            0 => intent.operation_id = "another-original-operation".into(),
            1 => intent.connection_id = "another-original-target".into(),
            2 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]
                    ["reservation_id"] = json!("different-reservation")
            }
            3 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["source_cut_id"] =
                    json!("different-cut")
            }
            4 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["nonce"] =
                    json!("different-challenge")
            }
            5 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["binding"]
                    ["mid"] = json!("3")
            }
            6 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["binding"]
                    ["connection_generation"] = json!(8)
            }
            7 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["binding"]
                    ["primary_ssrc"] = json!(1)
            }
            8 => {
                intent.offer["substrate_setup"]["sdp"] = json!("v=0\r\ns=changed-original-sdp\r\n")
            }
            9 => intent.expected_revision = Some(1),
            10 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["binding"]
                    ["physical_writer_incarnation"] = json!("different-original-writer")
            }
            11 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["nonce"] =
                    json!(m.scene.reservation.intent().nonce)
            }
            12 => {
                intent.offer["replacement_preparation"]["public_video_readiness"]["nonce"] =
                    json!("original-private-challenge")
            }
            _ => unreachable!(),
        }
        let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
        let proof = m.scene.f.admit(&request);
        assert!(
            m.scene
                .f
                .store
                .native_replacement_public_video_ready_read_with_peer(
                    &m.started,
                    &request.credentials[0],
                    &proof
                )
                .is_err(),
            "fault={fault}"
        );
        assert!(
            m.scene
                .f
                .store
                .replay_native_replacement_public_video_ready_with_peer(
                    &m.started,
                    &request.credentials[0],
                    &proof
                )
                .is_err(),
            "fault={fault}"
        );
        drop(proof);
        assert_eq!(
            Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
            before
        );
    }
    for fault in 0..4 {
        let mut request = public_request(&m);
        match fault {
            0 => request.credentials = vec![m.scene.f.keys[1].clone()],
            1 => request.device.id = "different-original-device".into(),
            2 => request.key_id = "different-original-key".into(),
            3 => request.algorithm = "different-original-algorithm".into(),
            _ => unreachable!(),
        }
        let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
        let error = retry_effect_free_metadata(&m.scene.f, || {
            m.scene.f.store.admit_peer_authority(
                &request,
                &crate::AuthorityWait::new(std::time::Duration::from_millis(100)),
            )
        })
        .err()
        .expect("wrong original signer/device must not create a usable proof");
        assert_ne!(
            error.code, "authority-busy",
            "transient refusal is not a signer negative"
        );
        assert_eq!(
            Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
            before
        );
    }
    let foreign = ready_fixture();
    assert!(!m
        .scene
        .reservation
        .same_original(&foreign.scene.reservation));
    let request = public_request(&m);
    let proof = m.scene.f.admit(&request);
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    assert!(m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &foreign.started,
            &request.credentials[0],
            &proof
        )
        .is_err());
    assert!(m
        .scene
        .f
        .store
        .replay_native_replacement_public_video_ready_with_peer(
            &foreign.started,
            &request.credentials[0],
            &proof
        )
        .is_err());
    drop(proof);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
}
#[test]
fn public_ready_peer_requires_original_cut_and_same_expiry_and_never_uses_negotiated_or_expired_readiness(
) {
    let m = ready_fixture();
    let request = public_request(&m);
    let proof = m.scene.f.admit(&request);
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    assert!(m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof
        )
        .is_err());
    assert!(m
        .scene
        .f
        .store
        .replay_native_replacement_public_video_ready_with_peer(
            &m.started,
            &request.credentials[0],
            &proof
        )
        .is_err());
    drop(proof);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    let proof = m.scene.f.admit(&request);
    let read = m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    let snapshot_until = read.expires_at();
    let original_until = m.scene.reservation.expires_at();
    assert!(
        snapshot_until < original_until,
        "the five-second source snapshot is shorter than the original operation"
    );
    let proof_until = snapshot_until.checked_add(1).unwrap();
    assert!(proof_until < original_until);
    // Narrow this SAME admitted proof: a fresh source snapshot may not extend
    // either its physical proof or the original reservation/credential bounds.
    let mut proof = proof;
    proof.limit_expiry(proof_until).unwrap();
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    {
        let clock = crate::test_clock::Clock::at(snapshot_until);
        assert!(
            !read.current(),
            "an already-admitted snapshot never renews itself"
        );
        assert!(proof.current());
        // This is a NEW current source authorization, not another private
        // readiness phase or an identical captured browser-marker replay.
        let fresh = m
            .scene
            .f
            .store
            .native_replacement_public_video_ready_read_with_peer(
                &m.started,
                &request.credentials[0],
                &proof,
            )
            .unwrap();
        assert!(fresh.current() && fresh.expires_at() > snapshot_until);
        assert!(fresh.expires_at() <= original_until && fresh.expires_at() <= proof.expires_at());
        for credential in [
            &m.scene.key,
            &request.credentials[0],
            &m.scene.f.keys[0],
            &m.scene.f.keys[1],
        ] {
            assert!(
                fresh.expires_at() <= m.scene.f.store.revalidate(credential).unwrap().expires_at
            );
        }
        assert_eq!(
            fresh.expires_at(),
            proof_until,
            "fresh source facts retain the shorter SAME proof cap"
        );
        clock.set(proof_until);
        assert!(!fresh.current() && !proof.current());
        assert!(
            m.scene
                .f
                .store
                .native_replacement_public_video_ready_read_with_peer(
                    &m.started,
                    &request.credentials[0],
                    &proof,
                )
                .is_err(),
            "an expired original proof cannot authorize a fresh source snapshot"
        );
        drop(fresh);
    }
    drop(read);
    drop(proof);
    let proof = m.scene.f.admit(&request);
    assert!(
        proof.expires_at() > original_until,
        "isolate original operation expiry from target proof expiry"
    );
    {
        let _clock = crate::test_clock::Clock::at(original_until);
        assert!(proof.current());
        assert!(
            m.scene
                .f
                .store
                .native_replacement_public_video_ready_read_with_peer(
                    &m.started,
                    &request.credentials[0],
                    &proof,
                )
                .is_err(),
            "fresh current target/source authority cannot renew the original operation cutoff"
        );
    }
    drop(proof);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    assert_eq!(readiness_rows(&m), 1);
}
#[test]
fn public_ready_peer_fresh_read_rechecks_withdrawn_source_while_same_marker_replay_is_only_masked_current_target_metadata(
) {
    let m = ready_fixture();
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    let request = public_request(&m);
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    // Controlled Host precondition ONLY: retain exact immutable whole marker.
    // This fixture does not claim actual decoder/lease or Host integration.
    let captured_marker = intent.offer["replacement_preparation"].clone();
    let proof = m.scene.f.admit(&request);
    let read = m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    assert!(read.current());
    drop(read);
    drop(proof);
    let admin = m.scene.f.store.revalidate(&m.scene.f.admin).unwrap();
    m.scene.f.store.revoke(&admin, &m.scene.f.keys[0]).unwrap();
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let proof = m.scene.f.admit(&request);
    assert!(m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof
        )
        .is_err());
    let crate::PeerCommand::NativeConnectionOffer(intent) = &request.command else {
        panic!()
    };
    assert_eq!(intent.offer["replacement_preparation"], captured_marker);
    let record = m
        .scene
        .f
        .store
        .replay_native_replacement_public_video_ready_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    assert_eq!(record.record_id, m.scene.reservation.operation_record_id());
    assert_eq!(record.state, OperationState::Preparing);
    assert!(record.result.is_none() && record.problem.is_none() && record.effects.is_empty());
    drop(proof);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    assert_eq!(readiness_rows(&m), 1);
    assert_eq!(m.scene.f.count("native_replacement_public_video_phases"), 2);
}
#[test]
fn public_ready_peer_completed_original_replay_never_recreates_private_readiness_or_public_decode_after_adoption(
) {
    let m = ready_fixture();
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::CutBoundPostF,
    );
    let request = public_request(&m);
    let proof = m.scene.f.admit(&request);
    let read = m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    drop(read);
    drop(proof);
    issue(
        &m.scene.f,
        &m.scene.reservation,
        &m.scene.command,
        Stage::DecodedReady,
    );
    let captured = capture(&m.scene);
    let installed = Installation::new(&m.scene);
    let established = m
        .scene
        .f
        .store
        .project_native_replacement_committed(&captured, &installed)
        .unwrap();
    assert!(m
        .scene
        .f
        .store
        .try_native_media_read(&established, None)
        .unwrap()
        .current());
    let completed = snapshot(&m.scene.f, &m.scene.key, &m.scene.command);
    assert_eq!(completed["phase"]["phase"], "activated");
    let before = Store::inspect_snapshot(m.scene.f.store.path()).unwrap();
    let proof = m.scene.f.admit(&request);
    assert!(m
        .scene
        .f
        .store
        .native_replacement_public_video_ready_read_with_peer(
            &m.started,
            &request.credentials[0],
            &proof
        )
        .is_err());
    let replay = m
        .scene
        .f
        .store
        .replay_native_replacement_public_video_ready_with_peer(
            &m.started,
            &request.credentials[0],
            &proof,
        )
        .unwrap();
    assert_eq!(replay.state, OperationState::Completed);
    assert!(replay.result.is_none() && replay.problem.is_none() && replay.effects.is_empty());
    drop(proof);
    assert_eq!(
        Store::inspect_snapshot(m.scene.f.store.path()).unwrap(),
        before
    );
    assert_eq!(readiness_rows(&m), 1);
    assert_eq!(m.scene.f.count("native_replacement_public_video_phases"), 4);
    assert_eq!(
        snapshot(&m.scene.f, &m.scene.key, &m.scene.command),
        completed
    );
}

#[path = "ready_peer/not_submitted.rs"]
mod not_submitted;
