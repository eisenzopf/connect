//! Signed replacement transfers existing video selections at activation.
use super::*;
use connect_protocol::replacement_video::{ReplacementVideo, ReplacementVideoStream};

#[tokio::test]
async fn production_native_video_transfer_activates_both_directions_without_post_commit_controls() {
    assert!(!super::super::super::super::sdk_controls::NodeMovementProbe::enabled());
    assert!(!crate::external_sdk_consumer::enabled());
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
        ActivationAudit::None,
        true,
    )
    .await;
}

pub(in super::super) fn intent() -> ReplacementVideo {
    ReplacementVideo {
        publications: vec![ReplacementVideoStream {
            previous: "codec-camera-target".into(),
            replacement: "replacement-camera".into(),
        }],
        receivers: vec![ReplacementVideoStream {
            previous: "codec-camera-source".into(),
            replacement: "replacement-incoming-camera".into(),
        }],
    }
}

pub(in super::super) async fn select_original(
    f: &Fixture,
    clients: &mut [CanonicalClient],
    cid: &str,
    publisher: u16,
) -> u16 {
    let (_, target) = control(
        &mut clients[1],
        cid,
        "codec-target",
        "video-transfer-original-incoming",
        stream_revision(f, "codec-target"),
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-voice"},{"strm_id":"codec-camera-source"}]}),
    )
    .await;
    let old_receiver = stream_handle(&target.unwrap(), "codec-camera-source");
    let (_, retained) = control(
        &mut clients[0],
        cid,
        "codec-source",
        "video-transfer-original-retained",
        stream_revision(f, "codec-source"),
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-monitor"},{"strm_id":"codec-camera-target"}]}),
    )
    .await;
    let retained = stream_handle(&retained.unwrap(), "codec-camera-target");
    let (source, target) = clients.split_at_mut(1);
    assert_vp8_routes_from(&source[0], &mut target[0], publisher, old_receiver, 10).await;
    retained
}

pub(in super::super) async fn after_commit(
    f: &Fixture,
    peers: &mut [Peer],
    cid: &str,
    candidate: &str,
    original_publisher: u16,
    replacement_publisher: u16,
    retained_receiver: u16,
    replacement_receiver: u16,
) {
    // No subscribe/unmute call is allowed before either actual packet proof.
    let db = rusqlite::Connection::open(&f.config.database).unwrap();
    let text: String = db
        .query_row(
            "SELECT readiness_json FROM connections WHERE id=?1",
            [candidate],
            |r| r.get(0),
        )
        .unwrap();
    let ready: Value = serde_json::from_str(&text).unwrap();
    let mut streams = ready["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    streams.sort();
    assert_eq!(
        streams,
        vec![
            "replacement-camera",
            "replacement-incoming-camera",
            "replacement-voice"
        ]
    );
    drop(db);
    let revisions = [
        stream_revision(f, "codec-source"),
        stream_revision(f, candidate),
    ];
    let (retained, rest) = peers.split_at_mut(1);
    assert_vp8_routes_from(
        &rest[1].client,
        &mut retained[0].client,
        replacement_publisher,
        retained_receiver,
        50,
    )
    .await;
    assert_vp8_routes_from(
        &retained[0].client,
        &mut rest[1].client,
        original_publisher,
        replacement_receiver,
        100,
    )
    .await;
    assert_eq!(
        [
            stream_revision(f, "codec-source"),
            stream_revision(f, candidate)
        ],
        revisions
    );
    // Afterwards restore the shared fixture's audio-only probes and drain only
    // the two already identified video hops. This cannot establish activation.
    control(
        &mut peers[0].client,
        cid,
        "codec-source",
        "video-transfer-proof-done-retained",
        revisions[0],
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"replacement-voice"}]}),
    )
    .await;
    control(
        &mut peers[2].client,
        cid,
        candidate,
        "video-transfer-proof-done-target",
        revisions[1],
        "stream.subscribe",
        json!({"subscriptions":[{"strm_id":"codec-voice"}]}),
    )
    .await;
    drain_exercised_video(&mut peers[0], retained_receiver).await;
    drain_exercised_video(&mut peers[2], replacement_receiver).await;
    eprintln!("signed video transfer: actual VP8 in both directions on original negotiated receiver handles, no post-commit subscribe/unmute or resource revision change; source/target authority and original recovery audited by shared fixture; not decoded quality");
}
