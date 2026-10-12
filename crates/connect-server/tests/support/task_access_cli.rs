//! Exercise the shipped binary; never put its credentials in process arguments.
#![allow(clippy::unwrap_used, dead_code)]
use super::fixture::Fixture;
use connect_server::cli::{init, task_access::GrantFile};
use std::{net::SocketAddr, path::Path, process::Output, time::Duration};

pub fn config(f: &Fixture) -> std::path::PathBuf {
    let path = f.directory().join("developer.toml");
    init::private_write(&path, toml::to_string(&f.owner).unwrap().as_bytes()).unwrap();
    path
}
pub async fn run(
    config: &Path,
    address: SocketAddr,
    command: &str,
    args: &[(&str, &Path)],
) -> Output {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    child
        .kill_on_drop(true)
        .args(["task-access", command, "--config"])
        .arg(config)
        .arg("--http-address")
        .arg(address.to_string());
    for (flag, path) in args {
        child.arg(flag).arg(path);
    }
    tokio::time::timeout(Duration::from_secs(30), child.output())
        .await
        .unwrap()
        .unwrap()
}
pub fn file(path: &Path) -> GrantFile {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}
pub fn no_secret(output: &Output, secrets: &[&str]) {
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        for secret in secrets {
            assert!(!text.contains(secret), "CLI reflected a private credential");
        }
    }
}
pub async fn proxy(
    f: &Fixture,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<(String, String)>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let destination = super::access::url(f);
    let (send, receive) = tokio::sync::oneshot::channel();
    let sender = std::sync::Arc::new(std::sync::Mutex::new(Some(send)));
    let app = axum::Router::new().route(
        "/v1/task-access",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, bytes: axum::body::Bytes| {
                let destination = destination.clone();
                let sender = sender.clone();
                async move {
                    let result: connect_protocol::tasks::TaskAccessResponse = super::access::http()
                        .post(destination)
                        .header("authorization", &headers["authorization"])
                        .header("content-type", "application/json")
                        .body(bytes)
                        .send()
                        .await
                        .unwrap()
                        .json()
                        .await
                        .unwrap();
                    let grant = super::access::issued(result);
                    sender
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .send((grant.grant.id.clone(), grant.expose_secret().into()))
                        .unwrap();
                    // The server committed issuance but its intended result never reaches the CLI.
                    (
                        axum::http::StatusCode::BAD_GATEWAY,
                        "interrupted proxy response",
                    )
                }
            },
        ),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, task, receive)
}
