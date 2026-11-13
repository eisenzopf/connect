//! Actual signed native Root, acknowledged original watcher pause, explicit
//! one-slot cold-cache fixture, and first fresh marked RTP admission.
//! No claim about decoded quality, no-new-ingress or old-server continuity.
use super::*;
pub(in super::super) use connect_service::canonical_media_projection_fixture::RouteEnrollmentBoundary as Boundary;

#[tokio::test]
async fn production_pending_cold_route_first_fresh_packet_admits() {
    exercise(Boundary::Pending).await;
}
#[tokio::test]
async fn production_ordinary_cold_route_first_fresh_packet_admits() {
    exercise(Boundary::Ordinary).await;
}
async fn exercise(boundary: Boundary) {
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
        ActivationAudit::RouteEnrollment(boundary),
    )
    .await;
}

pub(in super::super) async fn paused(
    f: &Fixture,
    credential: &str,
    command: &TransitionCommand,
    peers: &mut [Peer],
    boundary: Boundary,
    pause: connect_service::canonical_media_projection_fixture::RouteEnrollmentPause,
) -> (
    String,
    usize,
    bool,
    Vec<std::sync::Weak<dyn std::any::Any + Send + Sync>>,
) {
    tokio::time::timeout(Duration::from_secs(5), pause.entered())
        .await
        .expect("actual Root did not reach watcher-paused route boundary");
    let current = status(f, credential, &command.operation_id).await;
    let roles_available = match tokio::time::timeout(
        Duration::from_millis(100),
        f.server.service.hold_canonical_roles_for_test(),
    )
    .await
    {
        Ok(Ok(roles)) => {
            drop(roles);
            true
        }
        _ => false,
    };
    let owners = f
        .server
        .service
        .canonical_replacement_weak_owners_for_test(&current.record_id)
        .unwrap();
    assert!(owners.iter().all(|owner| owner.strong_count() > 0));
    match boundary {
        Boundary::Pending => {
            assert_eq!(current.state, OperationState::Preparing);
            assert!(current.result.is_none());
        }
        Boundary::Ordinary => assert_eq!(current.state, OperationState::Completed),
    }
    let peer = &mut peers[2];
    peer.sequence = peer.sequence.wrapping_add(1);
    let timestamp = (peer.clock.elapsed().as_micros() * 8 / 1000) as u32;
    // Exact original wire identity, not timing or an assumed drained tail.
    pause.arm_first_original_publish(
        connect_service::canonical_media_projection_fixture::RouteEnrollmentMark {
            stream_local_id: peer.handle,
            sequence: peer.sequence,
            ssrc: 0x4736_8e01,
            timestamp,
        },
    );
    let paused_before = pause.watcher_is_paused();
    let marked = rvoip_uctp::substrate::pack_rtp_datagram(&rvoip_uctp::substrate::RtpDatagram {
        flags: 0,
        stream_local_id: peer.handle,
        seq: peer.sequence,
        rtp: rvoip_uctp::substrate::RtpMediaPayload {
            payload: (0..160)
                .map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
                .collect::<Vec<_>>()
                .into(),
            payload_type: 0,
            sequence_number: peer.sequence as u16,
            timestamp,
            ssrc: 0x4736_8e01,
        },
    })
    .unwrap();
    peer.client.send_native_media_datagram(&marked).unwrap();
    let (mut first, misses) =
        tokio::time::timeout(Duration::from_secs(2), pause.first_original_publish())
            .await
            .unwrap_or_else(|_| ("fixture-unqualified-no-original-policy".into(), 0));
    let paused_after = pause.watcher_is_paused();
    if !paused_before || !paused_after {
        first = "fixture-unqualified-owner-resumed".into();
    }
    eprintln!(
        "G6_ROUTE_ENROLLMENT_FIRST {}",
        json!({
            "boundary":format!("{boundary:?}"), "sequence":peer.sequence,
            "ssrc":0x4736_8e01u32, "first_policy_result":first,
            "actual_lazy_registration_misses":misses,
            "roles_available_at_paused_boundary":roles_available,
            "explicit_cold_unary_slot":true, "no_fake_grant":true,
            "watcher_paused_before":paused_before, "watcher_paused_after":paused_after,
        })
    );
    // Preserve the first verdict. Recovery is not credited to that packet.
    pause.release();
    tokio::time::timeout(Duration::from_secs(2), pause.left())
        .await
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let record = status(f, credential, &command.operation_id).await;
        if record.state == OperationState::Completed {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "original Root did not settle: {record:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(pause);
    (first, misses, roles_available, owners)
}
