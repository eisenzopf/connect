//! Source-lane third route proof runs distinct shipped clients and public CLI only.
#![allow(clippy::unwrap_used)]
use std::{path::Path, process::Stdio, time::Duration};
use tokio::process::Command;

#[tokio::test]
#[ignore = "requires built connect/connect-video, explicit FFmpeg and selected local Chromium; 60-second AV markers"]
async fn native_process_and_browser_decode_duplex_av_through_public_interfaces() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let binary = std::env::var("CONNECT_BINARY")
        .unwrap_or_else(|_| root.join("target/debug/connect").display().to_string());
    let video = std::env::var("CONNECT_VIDEO_BINARY").unwrap_or_else(|_| {
        root.join("target/debug/connect-video")
            .display()
            .to_string()
    });
    let ffmpeg = std::env::var("CONNECT_FFMPEG_BINARY")
        .expect("Select CONNECT_FFMPEG_BINARY explicitly for capture and independent decoding");
    let node = std::env::var("CONNECT_NODE_BINARY")
        .expect("Select CONNECT_NODE_BINARY explicitly for the browser proof");
    for name in ["CONNECT_CHROME_BINARY", "CONNECT_PLAYWRIGHT_MODULE"] {
        std::env::var(name)
            .unwrap_or_else(|_| panic!("Select {name} explicitly for the browser proof"));
    }
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .await
        .unwrap();
    assert!(revision.status.success());
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        Command::new("python3")
            .arg(root.join("scripts/check-video-paths.py"))
            .arg("--runtime")
            .arg(root)
            .arg("--source-path")
            .arg(root)
            .arg("--server-binary")
            .arg(binary)
            .arg("--video-binary")
            .arg(video)
            .arg("--ffmpeg")
            .arg(ffmpeg)
            .arg("--node")
            .arg(node)
            .arg("--source-revision")
            .arg(String::from_utf8(revision.stdout).unwrap().trim())
            .arg("--work-root")
            .arg(root.join(".local/gate5/source-native-browser"))
            .args(["--path", "native-browser", "--seconds", "60"])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "Public-interface process qualification failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "passed-clean");
    assert_eq!(report["tested_inputs_stable"], true);
    println!("VIDEO_RESULT {report}");
}
