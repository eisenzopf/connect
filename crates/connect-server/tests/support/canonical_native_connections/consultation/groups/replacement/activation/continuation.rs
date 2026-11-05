//! Original signed replacement with a real Core commit paused before metadata.
//! These are behavior controls, not a 60-second quality qualification.
use super::*;
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
    let target_expiry = f
        .server
        .store
        .authenticate(target_credential)
        .unwrap()
        .expires_at;
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
    unpublished(f, candidate, &current.record_id);
    peers[1].live = false;
    // Qualified BEFORE witness: the original live generation moved while the
    // journal has no Ready. Exact marked PCM must reach both actual endpoints.
    probe(peers, 0, &[2], "paused-original-move-retained-to-target").await;
    probe(peers, 2, &[0], "paused-original-move-target-to-retained").await;
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
        assert_eq!(retired.id, candidate);
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
        let remaining = target_expiry.saturating_sub(chrono::Utc::now().timestamp());
        assert!(
            (0..=120).contains(&remaining),
            "fixture original deadline is not bounded"
        );
        tokio::time::sleep(Duration::from_secs(remaining.max(0) as u64 + 1)).await;
    }
    unpublished(f, candidate, &current.record_id);
    if !matches!(case, Case::Expire) {
        assert_eq!(
            status(f, credential, &command.operation_id).await.effects,
            effects,
            "paused metadata must not replay the original movement"
        );
    }
    pause.release();
    tokio::time::timeout(Duration::from_secs(2), pause.left())
        .await
        .unwrap();
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
