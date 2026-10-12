//! Bounded subprocess drivers for the real SDK and JSON CLI matrix.
#![allow(clippy::unwrap_used)]
use super::task_matrix::Bindings;
use connect_client::ClientConfig;
use serde_json::json;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

pub(super) struct MatrixContext<'a> {
    pub(super) owner: &'a ClientConfig,
    pub(super) peer: &'a ClientConfig,
    pub(super) owner_path: &'a Path,
    pub(super) peer_path: &'a Path,
    pub(super) url: &'a str,
    pub(super) http: &'a reqwest::Client,
    pub(super) root: &'a Path,
}

pub(super) async fn cli(path: &Path, input: &[u8], discover: bool) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    command
        .arg("request")
        .arg("--config")
        .arg(path)
        .arg("--envelope");
    if discover {
        command.arg("--task-address").arg("connect:reviews@local");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
}

pub(super) async fn language(context: &MatrixContext<'_>, lane: &str, mut bindings: Bindings) {
    let MatrixContext {
        root,
        owner_path,
        peer_path,
        url,
        owner,
        peer,
        ..
    } = *context;
    let python = lane.starts_with("python-");
    let mut command = tokio::process::Command::new(if python { "python3" } else { "node" });
    command
        .arg(root.join(if python {
            "sdk/tests/task_matrix_probe.py"
        } else {
            "sdk/tests/task_matrix_probe.mjs"
        }))
        .arg(owner_path)
        .arg(peer_path)
        .arg(url)
        .arg(env!("CARGO_BIN_EXE_connect"))
        .arg(lane.split('-').nth(1).unwrap());
    if python {
        command.env("PYTHONPATH", root.join("sdk/python"));
    }
    bindings.insert(
        "credentials".into(),
        json!({"owner":owner.credential,"peer":peer.credential}),
    );
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&bindings).unwrap())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{lane} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
