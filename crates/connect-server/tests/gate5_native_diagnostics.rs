//! Public native handle diagnostics observe actual local media without payloads.
#[path = "gate5_support/video.rs"]
mod video;
use connect_client::NativeAvFrame;
use std::time::Duration;

#[tokio::test]
async fn public_native_snapshot_observes_real_streams_and_shared_byte_buffer() {
    let mut f = video::Fixture::new().await;
    let mut left = f
        .left
        .open_native_video(&f.conversation, &f.session, &f.right_id)
        .await
        .expect("left AV");
    let mut right = f
        .right
        .open_native_video(&f.conversation, &f.session, &f.left_id)
        .await
        .expect("right AV");
    f.right
        .subscribe_native_video_participant(&right, &f.left_id)
        .await
        .expect("authorized publisher subscriptions");
    // StreamOpened control and best-effort media have no cross-channel delivery
    // order. Keep a paced publisher alive until both actual receive paths work;
    // a fixed number of ignored warm receipts is not a readiness barrier.
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut tick = tokio::time::interval(Duration::from_millis(40));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let (mut audio, mut video) = (false, false);
        while !(audio && video) {
            tokio::select! {
                _ = tick.tick() => {
                    left.send_pcmu(&[0xff; 160]).await.expect("warm audio");
                    // VP8 partition-zero startup requires the encoded keyframe bit.
                    left.send_vp8(&[0x34; 100]).await.expect("warm key-start video");
                }
                frame = right.receive(Duration::from_millis(500)) => {
                    match frame.expect("warm media receipt") {
                        NativeAvFrame::Audio(_) => audio = true,
                        NativeAvFrame::Video(_) => video = true,
                    }
                }
            }
        }
    })
    .await
    .expect("both native streams ready");
    // Different fixture bytes require fresh post-readiness receipts, not a
    // delayed warm packet. These encoded-byte fixtures are not decoder proof.
    left.send_pcmu(&[0xfe; 160])
        .await
        .expect("audio local enqueue");
    left.send_vp8(&[0x35; 100])
        .await
        .expect("video local enqueue");
    let (mut audio, mut video) = (false, false);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !(audio && video) {
            match right
                .receive(Duration::from_millis(500))
                .await
                .expect("media receipt")
            {
                NativeAvFrame::Audio(frame) => audio |= frame.payload == [0xfe; 160],
                NativeAvFrame::Video(packet) => video |= packet.rtp.ends_with(&[0x35; 100]),
            }
        }
    })
    .await
    .expect("both received streams");
    let outgoing = left.media_diagnostics();
    let incoming = right.media_diagnostics();
    let quic = outgoing.quic.as_ref().expect("observed client QUIC sends");
    let buffer = quic
        .send_buffer
        .as_ref()
        .expect("configured sampled client buffer");
    assert_eq!(buffer.capacity_bytes, 76032);
    assert!(buffer.observations > 0);
    assert!(buffer.sampled_high_water_bytes <= buffer.capacity_bytes);
    assert_eq!(buffer.capacity_mismatch_observations, 0);
    assert!(quic.accepted_datagrams >= 2);
    let video_in = incoming
        .queues
        .iter()
        .find(|(stage, _)| {
            matches!(
                stage,
                rvoip_core::media_diagnostics::QueueStage::QuicVideoIn
            )
        })
        .expect("observed video receiver queue");
    assert!(video_in.1.capacity_packets <= 64);
    assert!(video_in.1.sampled_high_water_packets <= video_in.1.capacity_packets);
    assert!(
        video_in
            .1
            .max_forwarded_frame_age_ms
            .expect("actual receiver dequeue age")
            <= 200
    );
    let debug = format!("{outgoing:?} {incoming:?}");
    for private in [&f.conversation, &f.session, &f.left_id, &f.left_token] {
        assert!(!debug.contains(private), "private metadata omitted");
    }
    // Pause only the application receiver, leaving both actual transports and
    // their owned pumps active. An old queued packet must not become fresh media
    // merely because the application starts receiving again.
    left.send_vp8(&[0x42; 100]).await.expect("old queued video");
    tokio::time::sleep(Duration::from_millis(300)).await;
    left.send_vp8(&[0x65; 100])
        .await
        .expect("fresh video after pause");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let NativeAvFrame::Video(packet) = right
                .receive(Duration::from_millis(500))
                .await
                .expect("fresh receipt after stale discard")
            {
                assert!(packet.rtp.ends_with(&[0x65; 100]));
                break;
            }
        }
    })
    .await
    .expect("fresh frame deadline");
    let after_pause = right.media_diagnostics();
    let (_, video_queue) = after_pause
        .queues
        .iter()
        .find(|(stage, _)| {
            matches!(
                stage,
                rvoip_core::media_diagnostics::QueueStage::QuicVideoIn
            )
        })
        .expect("paused receiver observations");
    assert!(video_queue.stale_forwarding_discards > 0);
    assert!(
        video_queue
            .max_forwarded_frame_age_ms
            .expect("fresh forwarded packet age")
            <= 200
    );
    left.end(&mut f.left).await.expect("owned end");
    right.end(&mut f.right).await.expect("counterpart end");
    f.left.close().await;
    f.right.close().await;
    f.server.shutdown().await.expect("server shutdown");
}
