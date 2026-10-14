//! Actual Chromium decoders, generated camera/audio, no owner devices/providers.
#![allow(clippy::unwrap_used)]
#[path = "gate5_support/browser.rs"]
mod browser;
#[path = "gate5_support/video.rs"]
mod video;
use std::{path::Path, time::Duration};

#[tokio::test]
#[ignore = "requires selected Playwright/Chrome; 60-second browser decoder profile"]
async fn browser_to_browser_decodes_duplex_moving_video_and_audio_for_sixty_seconds() {
    let f = video::Fixture::new().await;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let report = browser::run(
        root,
        &format!("ws://{}", f.server.webrtc_address.unwrap()),
        &[f.left_token, f.right_token],
        60,
    )
    .await;
    drop(f.left);
    drop(f.right);
    tokio::time::timeout(Duration::from_secs(5), f.server.shutdown())
        .await
        .unwrap()
        .unwrap();
    println!(
        "VIDEO_RESULT {}",
        serde_json::json!({"path":"browser-browser","result":report})
    );
}
