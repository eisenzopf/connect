//! Explicit local Chromium fixture. Credentials travel over a private pipe.
#![allow(clippy::unwrap_used)]
use serde_json::Value;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

pub async fn run(root: &Path, signaling: &str, tokens: &[String], seconds: u64) -> Value {
    let node = std::env::var("CONNECT_NODE_BINARY").unwrap_or_else(|_| "node".into());
    let mut command = Command::new(node);
    command.arg(root.join("scripts/video/browser-qualify.mjs"));
    command
        .env_remove("TELNYX_API_KEY")
        .env_remove("VAPI_PRIVATE_KEY");
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(
            &serde_json::to_vec(&serde_json::json!({"root":root,"signalingURL":signaling,
        "tokens":tokens,"seconds":seconds}))
            .unwrap(),
        )
        .await
        .unwrap();
    input.shutdown().await.unwrap();
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "Chromium qualification failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["status"], "passed");
    report
}
