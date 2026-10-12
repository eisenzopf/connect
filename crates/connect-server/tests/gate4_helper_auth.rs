//! Typed authentication failures through real shipped Python/TypeScript helpers.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
use connect_client::{tasks::*, ConnectClient};
use connect_server::cli::init;
use native_tasks::Harness;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_helpers_classify_startup_and_existing_peer_expiry_without_owner_fallback() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = grants::task(&mut h, "helper-bound-task").await;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let dir = h
        .owner_config
        .trust_certificate
        .parent()
        .unwrap()
        .to_owned();
    for (runtime, program, probe) in [
        ("python", "python3", "helper_auth_probe.py"),
        ("typescript", "node", "helper_auth_probe.mjs"),
    ] {
        let expires_at = chrono::Utc::now().timestamp() + 45;
        let grant = grants::issue(
            &h,
            &task,
            runtime,
            vec![TaskPermission::History],
            expires_at,
        )
        .await;
        let config = h.owner_config.clone().with_task_grant(&grant).unwrap();
        let path = dir.join(format!("{runtime}-model.toml"));
        init::private_write(&path, toml::to_string_pretty(&config).unwrap().as_bytes()).unwrap();
        let mut bad = config.clone();
        bad.credential.push('x');
        let bad_path = dir.join(format!("{runtime}-bad.toml"));
        init::private_write(&bad_path, toml::to_string_pretty(&bad).unwrap().as_bytes()).unwrap();
        let mut command = tokio::process::Command::new(program);
        command
            .arg(root.join("sdk/tests").join(probe))
            .arg(path)
            .arg(bad_path)
            .arg(dir.join("deliberately-absent.toml"))
            .arg(env!("CARGO_BIN_EXE_connect"))
            .env("PYTHONPATH", root.join("sdk/python"))
            .env(
                "CONNECT_TYPESCRIPT_SDK",
                root.join("sdk/typescript/dist/index.js"),
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
            .write_all(
                &serde_json::to_vec(&serde_json::json!({"task":task.id,"expires_at":expires_at}))
                    .unwrap(),
            )
            .await
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(90), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.status.success(),
            "{runtime}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(output["status"], "ok");
        assert!(!String::from_utf8_lossy(&result.stdout).contains(grant.expose_secret()));
        assert!(!String::from_utf8_lossy(&result.stderr).contains(grant.expose_secret()));
        assert!(!String::from_utf8_lossy(&result.stdout).contains(&h.owner_config.credential));
        assert!(!String::from_utf8_lossy(&result.stderr).contains(&h.owner_config.credential));
    }
    // The observer has been idle during both expiry cases; use a fresh peer.
    let mut observer = ConnectClient::open(h.owner_config.clone()).await.unwrap();
    assert!(observer
        .history(&h.cid, None, 10)
        .await
        .unwrap()
        .messages
        .is_empty());
    observer.close().await;
    h.close().await;
}
