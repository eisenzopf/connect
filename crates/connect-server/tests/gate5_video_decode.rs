//! Independent FFmpeg decoding is an explicit external-tool qualification lane.
#![allow(clippy::unwrap_used)]
#[path = "gate5_support/video_codec.rs"]
mod codec;
#[path = "gate5_support/video.rs"]
mod video;
use std::time::Duration;

#[tokio::test]
#[ignore = "requires explicit local FFmpeg; 60-second decoded-video operating profile"]
async fn native_to_native_decodes_duplex_moving_video_and_distinct_audio_for_sixty_seconds() {
    let encoded = codec::generate(60).await;
    let mut f = video::Fixture::new().await;
    let left = f
        .left
        .open_native_video(&f.conversation, &f.session, &f.right_id)
        .await
        .unwrap();
    let right = f
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
    let ((mut left, lo), (mut right, ro)) = tokio::join!(
        codec::participate(left, &encoded, 0x40, 0x70, 60),
        codec::participate(right, &encoded, 0x70, 0x40, 60)
    );
    let decoded_left = codec::decode(&f.temporary.path().join("left.ivf"), &lo).await;
    let decoded_right = codec::decode(&f.temporary.path().join("right.ivf"), &ro).await;
    let began = std::time::Instant::now();
    left.end(&mut f.left).await.unwrap();
    right.end(&mut f.right).await.unwrap();
    drop(left);
    drop(right);
    drop(f.left);
    drop(f.right);
    tokio::time::timeout(Duration::from_secs(5), f.server.shutdown())
        .await
        .unwrap()
        .unwrap();
    println!(
        "VIDEO_RESULT {}",
        serde_json::json!({"path":"native-native","seconds":60,"left_changing":decoded_left,
        "right_changing":decoded_right,"left_audio":lo.audio,"right_audio":ro.audio,"teardown_seconds":began.elapsed().as_secs_f64()})
    );
}
