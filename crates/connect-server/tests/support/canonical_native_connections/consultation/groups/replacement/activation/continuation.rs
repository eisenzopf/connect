//! Original signed replacement with a real Core commit paused before metadata.
//! These are behavior controls, not a 60-second quality qualification.
use super::*;
// A diagnostic refusal must not replace or weaken the original probe verdict.
// Codes: 1 = retained-ID query; 2 = metadata busy; 3 = unavailable exact owner.
struct PausedAudioDiagnostic(
    std::result::Result<rvoip_core::staged_streams::audio_diagnostic::AudioDiagnostics, u8>,
);
impl Drop for PausedAudioDiagnostic {
    fn drop(&mut self) {
        let snapshot = match &self.0 {
            Ok(observation) => {
                serde_json::json!({"armed":true,"observation":observation.snapshot()})
            }
            Err(code) => serde_json::json!({"armed":false,"refusal_code":code}),
        };
        if let Ok(snapshot) = serde_json::to_string(&snapshot) {
            use std::io::Write;
            let _ = writeln!(std::io::stderr(), "G6_PAUSED_AUDIO_DIAGNOSTIC {snapshot}");
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(in super::super) enum Case {
    Duplex,
    Writer,
    Revoke,
    Retire,
    Expire,
    Video,
}
async fn exercise(case: Case) {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::Continuation(case),
    )
    .await;
}
#[tokio::test]
async fn production_unpaused_replacement_continuation_fixture_control() {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::None,
    )
    .await;
}
#[tokio::test]
async fn production_paused_replacement_projection_keeps_explicit_video_transfer_gates() {
    exercise_replacement_with_av_transfer(
        true,
        true,
        false,
        true,
        false,
        Some("EdDSA"),
        false,
        Some((false, false, false)),
        false,
        true,
        ActivationAudit::Continuation(Case::Video),
        true,
    )
    .await;
}
#[tokio::test]
async fn production_paused_replacement_projection_keeps_original_duplex() {
    exercise(Case::Duplex).await;
}
#[tokio::test]
async fn production_paused_replacement_projection_writer_drains_and_fresh_authority_recovers() {
    exercise(Case::Writer).await;
}
#[tokio::test]
async fn production_paused_replacement_projection_target_revocation_refuses_without_old_revival() {
    exercise(Case::Revoke).await;
}
#[tokio::test]
async fn production_paused_replacement_projection_actual_target_retirement_cannot_resume() {
    exercise(Case::Retire).await;
}
#[tokio::test]
async fn production_paused_replacement_projection_original_target_grant_expiry_stops_without_replay(
) {
    exercise(Case::Expire).await;
}

pub(in super::super) async fn paused(
    f: &Fixture,
    cid: &str,
    candidate: &str,
    credential: &str,
    target_credential: &str,
    command: &TransitionCommand,
    peers: &mut Vec<Peer>,
    case: Case,
    video: Option<(u16, u16, u16, u16)>,
    original_video_sequence: &mut u16,
    pause: connect_service::canonical_media_projection_fixture::ReplacementProjectionPause,
    mut preparation_video_tail: Option<&mut decoded_readiness::VideoTail>,
) -> (bool, Vec<std::sync::Weak<dyn std::any::Any + Send + Sync>>) {
    tokio::time::timeout(Duration::from_secs(5), pause.entered())
        .await
        .expect("original worker did not reach physical-commit/projection boundary");
    let current = status(f, credential, &command.operation_id).await;
    assert_eq!(current.state, OperationState::Preparing);
    assert!(current.result.is_none());
    let effects = current.effects.clone();
    let owners = f
        .server
        .service
        .canonical_replacement_weak_owners_for_test(&current.record_id)
        .unwrap();
    assert!(owners.len() >= 2 && owners.iter().all(|owner| owner.strong_count() > 0));
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let old:String=c.query_row("SELECT runtime_connection_id FROM connection_offers WHERE connection_id='codec-target'",[],|r|r.get(0)).unwrap();
    let target: String = c
        .query_row(
            "SELECT candidate_runtime_id FROM native_replacements WHERE operation_record_id=?1",
            [&current.record_id],
            |r| r.get(0),
        )
        .unwrap();
    let retained = if matches!(case, Case::Duplex) {
        Some(c.query_row::<String, _, _>(
            "SELECT runtime_connection_id FROM connection_offers WHERE connection_id='codec-source'",
            [], |row| row.get(0),
        ).map_err(|_| 1u8))
    } else {
        None
    };
    let target_expiry = f
        .server
        .store
        .authenticate(target_credential)
        .unwrap()
        .expires_at;
    let projection_expiry: i64 = c
        .query_row(
            "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1",
            [&current.record_id],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    let session = f
        .server
        .service
        .canonical_session_metadata_for_test("native-session")
        .unwrap();
    {
        let s = session.read().unwrap();
        assert!(
            !s.connections
                .contains_key(&rvoip_core::ids::ConnectionId::from_string(old)),
            "old Core route must actually be retired"
        );
        assert!(
            s.connections
                .contains_key(&rvoip_core::ids::ConnectionId::from_string(target.clone())),
            "target must be the actual Core owner"
        );
    }
    let audio_diagnostic = retained.map(|retained| {
        PausedAudioDiagnostic(retained.and_then(|retained| {
            f.server
                .service
                .canonical_paused_audio_diagnostic_for_test(&retained, &target)
                .map_err(|error| if error.code == "authority-busy" { 2 } else { 3 })
        }))
    });
    unpublished(f, candidate, &current.record_id);
    peers[1].live = false;
    // Qualified BEFORE witness: the original live generation moved while the
    // journal has no Ready. Exact marked PCM must reach both actual endpoints.
    let original_preparation = peers[2].client.native_replacement_readiness().unwrap();
    let public_before_projection = probe_sequences_with_video(
        peers,
        0,
        &[2],
        "paused-original-move-retained-to-target",
        true,
        preparation_video_tail.as_deref_mut().map(|tail| (2, tail)),
    )
    .await;
    assert!(
        public_before_projection
            .iter()
            .filter(|v| v.receiver == 2 && v.non_silent && v.decoded_samples == 160)
            .count()
            > 10
    );
    probe_sequences_with_video(
        peers,
        2,
        &[0],
        "paused-original-move-target-to-retained",
        false,
        preparation_video_tail.as_deref_mut().map(|tail| (2, tail)),
    )
    .await;
    if matches!(case, Case::Video) {
        let (original_publisher, replacement_publisher, retained_receiver, replacement_receiver) =
            video.expect("actual declared video transfer");
        let (retained, rest) = peers.split_at_mut(1);
        assert_vp8_routes_from(
            &rest[1].client,
            &mut retained[0].client,
            replacement_publisher,
            retained_receiver,
            20,
        )
        .await;
        assert_vp8_routes_from(
            &retained[0].client,
            &mut rest[1].client,
            original_publisher,
            replacement_receiver,
            *original_video_sequence,
        )
        .await;
        *original_video_sequence = original_video_sequence.wrapping_add(29);
        av::drain_exercised_video(&mut retained[0], retained_receiver).await;
        av::drain_exercised_video(&mut rest[1], replacement_receiver).await;
        // Encoded VP8 routing proves the policy/gate boundary only. Native
        // decoded-keyframe readiness is a separate SDK preparation assertion.
        unpublished(f, candidate, &current.record_id);
    }
    if matches!(case, Case::Writer) {
        let store = f.server.store.clone();
        let cid = cid.to_owned();
        let held = tokio::task::spawn_blocking(move || {
            store.acquire_authority(
                &[connect_store::AuthorityScope::conversation(
                    &cid,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap()],
                &connect_store::AuthorityWait::new(Duration::from_secs(2)),
            )
        })
        .await
        .unwrap()
        .unwrap();
        probe(
            peers,
            0,
            &[],
            "paused-current-writer-withdraws-all-media-readers",
        )
        .await;
        drop(held);
        probe(
            peers,
            0,
            &[2],
            "paused-current-writer-release-reacquires-target",
        )
        .await;
        probe(
            peers,
            2,
            &[0],
            "paused-current-writer-release-reacquires-source",
        )
        .await;
    }
    if matches!(case, Case::Revoke) {
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let key = f
            .server
            .store
            .authenticate(target_credential)
            .unwrap()
            .credential_id;
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &key))
            .await
            .unwrap()
            .unwrap();
        // The live sender has no new target authority. Both endpoints may close;
        // original generation retirement is independently verified after resume.
    } else if matches!(case, Case::Retire) {
        assert_eq!(peers.len(), 3);
        let retired = peers.pop().expect("original owned target");
        // The fixture label remains provisional until journal completion.
        // Bind this same physical reader to its already verified signed
        // preparation, then retire the actual owner while projection is paused.
        let readiness = retired.client.native_replacement_readiness().unwrap();
        assert_eq!(readiness.connection_id(), Some(candidate));
        assert_eq!(readiness.record_id(), current.record_id);
        assert_eq!(
            readiness.operation_id(),
            Some(command.operation_id.as_str())
        );
        retired.client.shutdown().await.unwrap();
        let until = tokio::time::Instant::now() + Duration::from_secs(3);
        let target = rvoip_core::ids::ConnectionId::from_string(target.clone());
        while session.read().unwrap().connections.contains_key(&target) {
            assert!(
                tokio::time::Instant::now() < until,
                "actual target Core retirement did not join"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    let stopped = matches!(case, Case::Revoke | Case::Retire | Case::Expire);
    if matches!(case, Case::Expire) {
        // Actual original target credential/key expiry, no clock injection or
        // renewed grant. The original movement was already completed in Core.
        assert!(
            target_expiry < projection_expiry,
            "the original target grant must expire inside the original preparation deadline"
        );
        assert!(
            futures_util::FutureExt::now_or_never(pause.left()).is_none(),
            "the original projection pause already ended before the expiry witness"
        );
        let remaining = target_expiry.saturating_sub(chrono::Utc::now().timestamp());
        assert!(
            (0..=120).contains(&remaining),
            "fixture original deadline is not bounded"
        );
        tokio::time::sleep(Duration::from_secs(remaining.max(0) as u64 + 1)).await;
        let now = chrono::Utc::now().timestamp();
        assert!(now >= target_expiry && now < projection_expiry);
        assert!(
            futures_util::FutureExt::now_or_never(pause.left()).is_none(),
            "the expiry witness must precede the unchanged projection-pause deadline"
        );
    }
    unpublished(f, candidate, &current.record_id);
    if !matches!(case, Case::Expire) {
        assert_eq!(
            status(f, credential, &command.operation_id).await.effects,
            effects,
            "paused metadata must not replay the original movement"
        );
    }
    drop(audio_diagnostic);
    pause.release();
    tokio::time::timeout(Duration::from_secs(2), pause.left())
        .await
        .unwrap();
    if !stopped {
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        let completed = loop {
            let record = status(f, credential, &command.operation_id).await;
            if record.state == OperationState::Completed {
                break record;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "same original projection did not complete"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let boundary = original_preparation
            .activation_from_completed(&serde_json::to_value(&completed).unwrap())
            .unwrap();
        if let Some(tail) = preparation_video_tail.as_deref_mut() {
            let (private, public) = tail.finish(&boundary).expect(
                "every buffered video fragment must match the final original activation cut",
            );
            eprintln!("original negotiated video demultiplexed: private_packets={private} public_membership_packets={public}; no audio or decoded public-video credit");
        }
        let connect_protocol::replacement_activation::NativeActivationAudience::Public { streams } =
            boundary.audience
        else {
            panic!("public original move has no public native cut")
        };
        let cut = streams
            .iter()
            .find(|v| v.stream_id == original_preparation.audio().stream_id)
            .unwrap();
        for packet in public_before_projection
            .iter()
            .filter(|v| v.receiver == 2 && v.non_silent)
        {
            assert_eq!(packet.handle, cut.stream_local_id);
            assert_eq!(cut.membership(packet.sequence).unwrap(),connect_protocol::replacement_activation::SequenceMembership::Public,
                "actual decoded public PCM before durable completed reply must fall at or after original Core cut");
        }
        let replay = status(f, credential, &command.operation_id).await;
        assert_eq!(
            replay.result, completed.result,
            "original completed cut is immutable without a new IO observation"
        );
    }
    if stopped {
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let record = status(f, credential, &command.operation_id).await;
            if record.state == OperationState::Completed {
                stopped_without_ready(f, candidate, &record);
                assert!(record
                    .effects
                    .iter()
                    .all(|e| e.attempt == 1 && e.submission_generation == 1));
                break;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "original stopped projection did not settle: {record:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    (stopped, owners)
}

pub(in super::super) fn owners_joined(owners: &[std::sync::Weak<dyn std::any::Any + Send + Sync>]) {
    assert!(
        owners.iter().all(|owner| owner.upgrade().is_none()),
        "joined original cleanup retained a replacement root/policy/continuation/entry owner"
    );
}
