//! Actual launcher ownership, retained interruption and safe operator inspection.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_fixture.rs"]
#[allow(dead_code)]
mod host;
use connect_protocol::tasks::{ProtectedTaskGrant, TaskPermission};
use connect_server::cli::{
    init::{private_replace, private_write},
    task_access::codex_host::{
        self,
        diagnostics::{inspect, HostState},
    },
};
use connect_store::Store;
use std::{path::Path, time::Duration};

async fn setup() -> fixture::Fixture {
    fixture::Fixture::start_permitted(true, vec![TaskPermission::History, TaskPermission::Inbox])
        .await
}
#[tokio::test]
async fn stopped_incomplete_turn_keeps_exact_session_and_metadata_omits_private_prompt() {
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert_eq!(inspect(None, &f.task.id).state, HostState::Unconfigured);
    assert_eq!(inspect(Some(&dir), &f.task.id).state, HostState::NotStarted);
    assert!(!dir.join("host.lock").exists() && !dir.join("host-state.json").exists());
    host::mode(&dir, "interrupt");
    assert!(
        codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
            .await
            .is_err()
    );
    let saved = host::state(&dir);
    let observation = inspect(Some(&dir), &f.task.id);
    assert_eq!(observation.state, HostState::Interrupted);
    assert_eq!(
        observation.readiness().reason,
        "original-worker-recovery-required"
    );
    assert_eq!(
        observation.session_id.as_deref(),
        saved["session_id"].as_str()
    );
    let mut private = saved.clone();
    private["prompt"] = serde_json::json!("PRIVATE-PROMPT-NOT-FOR-DIAGNOSTICS");
    private_replace(
        &dir.join("host-state.json"),
        &serde_json::to_vec(&private).unwrap(),
    )
    .unwrap();
    let encoded = serde_json::to_string(&inspect(Some(&dir), &f.task.id)).unwrap();
    assert!(
        !encoded.contains("PRIVATE-PROMPT")
            && !encoded.contains(f.issued.expose_secret())
            && !encoded.contains(&f.owner.credential)
    );
    assert_eq!(host::state(&dir), private);
    host::mode(&dir, "complete");
    codex_host::run(&dir, &binary, None, 15).await.unwrap();
    assert_eq!(
        inspect(Some(&dir), &f.task.id).state,
        HostState::TurnComplete
    );
    assert_eq!(host::state(&dir)["session_id"], saved["session_id"]);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.close().await;
}
#[tokio::test]
async fn owned_launcher_lock_then_deadline_reports_active_then_retained_interruption() {
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
        .await
        .unwrap();
    host::mode(&dir, "stall");
    let d = dir.clone();
    let b = binary.clone();
    let running = tokio::spawn(async move { codex_host::run(&d, &b, None, 2).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while host::state(&dir)["completed"] != false {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!running.is_finished());
    let active = inspect(Some(&dir), &f.task.id);
    assert_eq!(active.state, HostState::TurnActive);
    assert_eq!(
        active.readiness().state,
        connect_protocol::ReadinessState::Uncertain
    );
    assert!(running.await.unwrap().is_err());
    let stopped = inspect(Some(&dir), &f.task.id);
    assert_eq!(stopped.state, HostState::Interrupted);
    assert_eq!(
        stopped.readiness().state,
        connect_protocol::ReadinessState::Unavailable
    );
    assert_eq!(stopped.session_id, active.session_id);
    assert_eq!(stopped.run_id, active.run_id);
    f.close().await;
}
#[tokio::test]
async fn changed_binding_missing_session_and_invalid_private_files_require_repair() {
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "before");
    assert!(
        codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
            .await
            .is_err()
    );
    let saved = host::state(&dir);
    assert_eq!(
        inspect(Some(&dir), &f.task.id).state,
        HostState::MissingSession
    );
    assert_eq!(
        inspect(Some(&dir), "another-task").state,
        HostState::BindingMismatch
    );
    let mut changed = saved.clone();
    changed["grant_id"] = serde_json::json!("another-grant");
    private_replace(
        &dir.join("host-state.json"),
        &serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        inspect(Some(&dir), &f.task.id).state,
        HostState::BindingMismatch
    );
    private_replace(&dir.join("host-state.json"), b"PRIVATE-INVALID-JSON").unwrap();
    assert_eq!(
        inspect(Some(&dir), &f.task.id).state,
        HostState::InvalidState
    );
    private_replace(
        &dir.join("host-state.json"),
        &serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    private_replace(
        &dir.join(".agents/skills/connect/SKILL.md"),
        b"PRIVATE-CHANGED-SKILL",
    )
    .unwrap();
    let report = inspect(Some(&dir), &f.task.id);
    assert_eq!(report.state, HostState::InvalidInstallation);
    assert!(!serde_json::to_string(&report).unwrap().contains("PRIVATE"));
    f.close().await;
}
#[tokio::test]
async fn independent_cli_reports_local_worker_and_current_revocation_denies_report() {
    let f = setup().await;
    let (dir, _) = host::install(&f);
    let config = f
        .owner
        .clone()
        .with_task_grant(&ProtectedTaskGrant::new(
            f.issued.grant.clone(),
            f.issued.expose_secret().into(),
        ))
        .unwrap();
    let path = dir.join("diagnostic.toml");
    private_write(&path, toml::to_string(&config).unwrap().as_bytes()).unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let output = cli(&path, &dir, &f.task.id).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["codex_worker"]["state"], "not-started");
    assert_eq!(report["execution_readiness"]["state"], "unavailable");
    assert!(report["execution_readiness"]["components"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["component"] == "codex-host" && c["reason"] == "worker-not-started"));
    assert_eq!(report["task"]["task_id"], f.task.id);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    let revoked = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let denied = cli(&path, &dir, &f.task.id).await;
    assert!(!denied.status.success() && denied.stdout.is_empty());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        revoked
    );
    f.close().await;
}
async fn cli(config: &Path, installation: &Path, task: &str) -> std::process::Output {
    tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
            .arg("task-diagnostics")
            .arg("--config")
            .arg(config)
            .arg("--task-id")
            .arg(task)
            .arg("--codex-installation")
            .arg(installation)
            .env_clear()
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}
