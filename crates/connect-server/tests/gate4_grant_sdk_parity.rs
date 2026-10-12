//! Five actual language SDK lanes and the shipped protected JSON CLI.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
use connect_client::{tasks::*, ClientConfig};
use connect_protocol::{Operation, Outcome, Response};
use connect_server::cli::{init, task_access::GrantFile};
use native_tasks::Harness;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::AsyncWriteExt;

async fn run(mut command: tokio::process::Command, input: &[u8]) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protected_export_and_language_clients_share_narrow_native_grant_without_parent_fallback() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = grants::task(&mut h, "sdk-task").await;
    let other = grants::task(&mut h, "other-task").await;
    let grant = grants::issue(
        &h,
        &task,
        "sdk-export-issue",
        vec![TaskPermission::History, TaskPermission::Send],
        chrono::Utc::now().timestamp() + 180,
    )
    .await;
    let directory = h.owner_config.trust_certificate.parent().unwrap();
    let developer = directory.join("developer.toml");
    init::private_write(
        &developer,
        toml::to_string_pretty(&h.owner_config).unwrap().as_bytes(),
    )
    .unwrap();
    let config = directory.join("model.toml");
    let grant_file = directory.join("grant.json");
    let request = TaskAccessRequest::new(TaskAccessCommand::Issue {
        request_id: "sdk-export-issue".into(),
        definition: GrantDefinition {
            task_id: grant.grant.task_id.clone(),
            binding: grant.grant.binding.clone(),
            operations: grant.grant.operations.clone(),
            expires_at: grant.grant.expires_at,
        },
    });
    init::private_write(
        &grant_file,
        &serde_json::to_vec(&GrantFile {
            profile: TASK_ACCESS_PROFILE.into(),
            request,
            grant: Some(grant),
        })
        .unwrap(),
    )
    .unwrap();
    let mut export = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    export
        .args(["task-access", "export-native", "--config"])
        .arg(&developer)
        .arg("--grant-file")
        .arg(&grant_file)
        .arg("--output")
        .arg(&config);
    let result = run(export, b"").await;
    assert!(
        result.status.success(),
        "export: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let original = std::fs::read(&config).unwrap();
    let mut retry = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    retry
        .args(["task-access", "export-native", "--config"])
        .arg(&developer)
        .arg("--grant-file")
        .arg(&grant_file)
        .arg("--output")
        .arg(&config);
    assert!(!run(retry, b"").await.status.success());
    assert_eq!(std::fs::read(&config).unwrap(), original);
    let native = ClientConfig::load(&config).unwrap();
    assert_ne!(native.credential, h.owner_config.credential);
    assert!(!std::fs::read_to_string(&config)
        .unwrap()
        .contains(&h.owner_config.credential));
    for bytes in [&result.stdout, &result.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(&native.credential));
        assert!(!text.contains(&h.owner_config.credential));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    for (language, mode) in [
        ("python", "native"),
        ("python", "http"),
        ("typescript", "native"),
        ("typescript", "http"),
        ("typescript", "browser"),
    ] {
        let python = language == "python";
        let mut command = tokio::process::Command::new(if python { "python3" } else { "node" });
        command
            .arg(root.join(if python {
                "sdk/tests/native_grant_probe.py"
            } else {
                "sdk/tests/native_grant_probe.mjs"
            }))
            .arg(&config)
            .arg(grants::url(&h))
            .arg(env!("CARGO_BIN_EXE_connect"))
            .arg(mode);
        if python {
            command.env("PYTHONPATH", root.join("sdk/python"));
        }
        let data = serde_json::json!({"token":native.credential,"task":task.id,"other":other.id,"cid":h.cid});
        let result = run(command, &serde_json::to_vec(&data).unwrap()).await;
        assert!(
            result.status.success(),
            "{language}/{mode}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!String::from_utf8_lossy(&result.stdout).contains(&native.credential));
    }
    let read = Operation::Task {
        request: Box::new(TaskRequest::new(grants::wrapped(
            &task.id,
            "cli-read",
            Operation::History {
                conversation_id: h.cid.clone(),
                after: None,
                limit: 10,
            },
        ))),
    };
    for _ in 0..2 {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
        command
            .arg("request")
            .arg("--config")
            .arg(&config)
            .arg("--task-id")
            .arg(&task.id)
            .arg("--envelope");
        let result = run(command, &serde_json::to_vec(&read).unwrap()).await;
        assert!(
            result.status.success(),
            "CLI: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let response: Response = serde_json::from_slice(&result.stdout).unwrap();
        assert!(matches!(response.result, Ok(Outcome::Task(_))));
        assert!(!String::from_utf8_lossy(&result.stdout).contains(&native.credential));
    }
    assert_eq!(
        h.peer
            .history(&h.cid, None, 10)
            .await
            .unwrap()
            .messages
            .len(),
        5
    );
    h.close().await;
}
