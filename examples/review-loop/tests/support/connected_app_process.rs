//! Owned fresh server lifecycle; does not touch any unrelated process.
#![allow(clippy::unwrap_used)]
use std::{fs::File, process::Stdio, time::Duration};
use tokio::process::{Child, Command};
pub(super) async fn spawn_server(
    binary: &std::path::Path,
    directory: &std::path::Path,
) -> (Child, std::net::SocketAddr, std::net::SocketAddr) {
    let path = directory.join(format!("server-{}.log", uuid::Uuid::new_v4()));
    let file = File::create(&path).unwrap();
    let mut child = Command::new(binary)
        .args(["serve", "--http-bind", "127.0.0.1:0", "--config"])
        .arg(directory.join("server.toml"))
        .stdout(Stdio::from(file.try_clone().unwrap()))
        .stderr(Stdio::from(file))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if std::fs::read_to_string(&path)
                .unwrap()
                .contains("ready transport=quic")
            {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "Server exited: {}",
                std::fs::read_to_string(&path).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let log = std::fs::read_to_string(&path).unwrap();
    let mcp = log
        .lines()
        .find_map(|l| l.strip_prefix("mcp_streamable_http=http://"))
        .unwrap()
        .trim_end_matches("/mcp")
        .parse()
        .unwrap();
    let http = log
        .lines()
        .find_map(|l| l.strip_prefix("http_json="))
        .unwrap()
        .parse()
        .unwrap();
    (child, mcp, http)
}
pub(super) async fn signal_stop(child: &mut Child) {
    if child.try_wait().unwrap().is_some() {
        return;
    }
    let status = Command::new("kill")
        .args(["-INT", &child.id().unwrap().to_string()])
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let status = tokio::time::timeout(Duration::from_secs(20), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
}
