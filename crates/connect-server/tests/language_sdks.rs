#![allow(clippy::unwrap_used)]
use connect_client::ClientConfig;
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use std::{path::PathBuf, process::Stdio};
use tokio::io::AsyncWriteExt;
#[tokio::test(flavor = "multi_thread")]
async fn python_and_typescript_sdks_use_verified_native_quic_and_share_http_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("demo");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&dir.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let mut config = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    init::private_write(
        &dir.join("client.toml"),
        toml::to_string_pretty(&config).unwrap().as_bytes(),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    assert!(
        root.join("sdk/typescript/dist/index.js").exists(),
        "Build TypeScript SDK before this test: cd sdk/typescript && npm ci && npm run build"
    );
    let url = format!("http://{}", server.http_address.unwrap());
    let exe = env!("CARGO_BIN_EXE_connect");
    let py = tokio::process::Command::new("python3")
        .arg(root.join("sdk/tests/python_probe.py"))
        .arg(dir.join("client.toml"))
        .arg(&url)
        .arg(exe)
        .env("PYTHONPATH", root.join("sdk/python"))
        .output()
        .await
        .unwrap();
    assert!(
        py.status.success(),
        "Python failed: {}",
        String::from_utf8_lossy(&py.stderr)
    );
    let mut child = tokio::process::Command::new("node")
        .arg(root.join("sdk/tests/typescript_probe.mjs"))
        .arg(dir.join("client.toml"))
        .arg(&url)
        .arg(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::to_vec(&serde_json::json!({"credential":config.credential}))
                .unwrap()
                .as_slice(),
        )
        .await
        .unwrap();
    let js = child.wait_with_output().await.unwrap();
    assert!(
        js.status.success(),
        "TypeScript failed: {}",
        String::from_utf8_lossy(&js.stderr)
    );
    server.shutdown().await.unwrap();
}
