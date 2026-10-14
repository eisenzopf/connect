//! Real QUIC negotiation/routing; this test deliberately makes no decoder claim.
#![allow(clippy::unwrap_used)]
#[path = "gate5_support/video.rs"]
mod video;
use connect_client::{NativeAvFrame, VideoFrameAssembler};
use std::time::Duration;

#[tokio::test]
async fn native_video_preserves_rtp_fragments_audio_and_session_history() {
    let mut f = video::Fixture::new().await;
    let mut left = f
        .left
        .open_native_video(&f.conversation, &f.session, &f.right_id)
        .await
        .unwrap();
    let mut right = f
        .right
        .open_native_video(&f.conversation, &f.session, &f.left_id)
        .await
        .unwrap();
    f.left
        .subscribe_native_video_participant(&left, &f.right_id)
        .await
        .unwrap();
    f.right
        .subscribe_native_video_participant(&right, &f.left_id)
        .await
        .unwrap();
    // First datagrams can precede the reliable subscriber-stream announcement.
    // Start each stream before asserting delivery of the later known frame.
    for _ in 0..2 {
        left.send_pcmu(&[0xff; 160]).await.unwrap();
        right.send_pcmu(&[0xff; 160]).await.unwrap();
        left.send_vp8(&[0; 200]).await.unwrap();
        right.send_vp8(&[0; 200]).await.unwrap();
        let _ = tokio::join!(
            left.receive(Duration::from_millis(50)),
            right.receive(Duration::from_millis(50))
        );
    }
    let encoded = vec![4u8; 3500];
    left.send_vp8(&encoded).await.unwrap();
    let mut assembler = VideoFrameAssembler::default();
    let frame = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let NativeAvFrame::Video(packet) =
                right.receive(Duration::from_secs(2)).await.unwrap()
            {
                if let Some(frame) = assembler.push(&packet).unwrap() {
                    if frame.encoded == encoded {
                        break frame;
                    }
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(frame.keyframe);
    right.send_pcmu(&[42; 160]).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let NativeAvFrame::Audio(frame) = left.receive(Duration::from_secs(2)).await.unwrap()
            {
                if frame.payload == vec![42; 160] {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    left.set_camera_muted(true);
    left.send_vp8(&encoded).await.unwrap();
    left.end(&mut f.left).await.unwrap();
    let session = f
        .left
        .session_get(&f.conversation, &f.session)
        .await
        .unwrap();
    assert_eq!(session.conversation_id, f.conversation);
    drop(left);
    drop(right);
    drop(f.left);
    drop(f.right);
    tokio::time::timeout(Duration::from_secs(5), f.server.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert!(f.temporary.path().exists());
}
