//! Exact session resume and process failure proofs, using real task MCP.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_fixture.rs"]
mod host;
use connect_protocol::tasks::TaskPermission;
use connect_server::cli::{init::private_replace, task_access::codex_host};
use connect_store::Store;
use serde_json::json;
use std::time::Duration;

async fn setup() -> fixture::Fixture {
    fixture::Fixture::start_permitted(true, vec![TaskPermission::History, TaskPermission::Inbox])
        .await
}
#[tokio::test]
async fn independent_tasks_resume_exact_sessions_and_retain_real_pending_handles() {
    let a = setup().await;
    let b = setup().await;
    let (ad, ab) = host::install(&a);
    let (bd, bb) = host::install(&b);
    let ap = ad.join("prompt.txt");
    let bp = bd.join("prompt.txt");
    let (ar, br) = tokio::join!(
        codex_host::run(&ad, &ab, Some(&ap), 15),
        codex_host::run(&bd, &bb, Some(&bp), 15)
    );
    ar.unwrap();
    br.unwrap();
    let asaved = host::state(&ad);
    let bsaved = host::state(&bd);
    assert_ne!(asaved["session_id"], bsaved["session_id"]);
    assert_eq!(asaved["task_id"], a.task.id);
    assert_eq!(bsaved["task_id"], b.task.id);
    codex_host::run(&ad, &ab, None, 15).await.unwrap();
    assert_eq!(host::state(&ad)["session_id"], asaved["session_id"]);
    assert_eq!(host::state(&bd), bsaved);
    let events = host::events(&ad);
    assert!(events.contains(&a.task.id));
    assert!(!events.contains(&b.task.id));
    assert!(events.contains("pending"));
    assert!(!events.contains(a.issued.expose_secret()));
    assert!(!events.contains(&a.owner.credential));
    assert!(events.contains("[REDACTED]"));
    a.close().await;
    b.close().await;
}
#[tokio::test]
async fn interrupted_turn_reuses_original_intent_and_current_revocation_denies_resume() {
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "interrupt");
    assert!(
        codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
            .await
            .is_err()
    );
    let saved = host::state(&dir);
    assert_eq!(saved["completed"], false);
    assert!(saved["session_id"].is_string());
    host::mode(&dir, "complete");
    codex_host::run(&dir, &binary, None, 15).await.unwrap();
    let recovered = host::state(&dir);
    assert_eq!(recovered["session_id"], saved["session_id"]);
    assert_eq!(recovered["run_id"], saved["run_id"]);
    assert_eq!(recovered["prompt"], saved["prompt"]);
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    assert_eq!(host::state(&dir)["session_id"], saved["session_id"]);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.close().await;
}
#[tokio::test]
async fn timeout_lock_session_switch_and_modified_assets_never_select_replacement_work() {
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
        .await
        .unwrap();
    host::mode(&dir, "stall");
    let workdir = dir.clone();
    let executable = binary.clone();
    let running = tokio::spawn(async move {
        codex_host::run(&workdir, &executable, Some(&workdir.join("prompt.txt")), 2).await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if host::state(&dir)["completed"] == false {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15)
        .await
        .unwrap_err()
        .to_string()
        .contains("active host turn"));
    assert!(running
        .await
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("deadline"));
    let saved = host::state(&dir);
    host::mode(&dir, "switch");
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    assert_eq!(host::state(&dir)["session_id"], saved["session_id"]);
    // Even task-local trusted files cannot silently substitute another task.
    let mut forged = host::state(&dir);
    forged["task_id"] = json!("another-task");
    private_replace(
        &dir.join("host-state.json"),
        &serde_json::to_vec(&forged).unwrap(),
    )
    .unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    private_replace(
        &dir.join("host-state.json"),
        &serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    private_replace(
        &dir.join(".agents/skills/connect/SKILL.md"),
        b"changed skill",
    )
    .unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    f.close().await;
}

#[tokio::test]
async fn fresh_launcher_process_recovers_original_session_and_missing_id_requires_repair() {
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "interrupt");
    assert!(!host::cli(&dir, &binary, true).await.status.success());
    let saved = host::state(&dir);
    assert!(saved["session_id"].is_string());
    host::mode(&dir, "complete");
    let result = host::cli(&dir, &binary, false).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["status"], "host-turn-completed");
    assert_eq!(report["session_id"], saved["session_id"]);
    assert_eq!(report["run_id"], saved["run_id"]);
    assert_eq!(report["task_id"], f.task.id);
    f.close().await;
    let f = setup().await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "before");
    assert!(!host::cli(&dir, &binary, true).await.status.success());
    let saved = host::state(&dir);
    assert!(saved["session_id"].is_null());
    host::mode(&dir, "complete");
    assert!(!host::cli(&dir, &binary, false).await.status.success());
    assert_eq!(host::state(&dir), saved);
    f.close().await;
}
