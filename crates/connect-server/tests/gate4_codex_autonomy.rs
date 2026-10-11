//! A delegated host runs without approval prompts and preserves legacy intent.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_fixture.rs"]
mod host;
use connect_protocol::tasks::TaskPermission;
use connect_server::cli::{init::private_replace, task_access::codex_host};
use connect_store::Store;

#[tokio::test]
async fn legacy_export_keeps_task_session_and_files_while_current_policy_is_applied() {
    let f = fixture::Fixture::start_permitted(
        true,
        vec![TaskPermission::History, TaskPermission::Inbox],
    )
    .await;
    let (dir, binary) = host::install(&f);
    let manifest = std::fs::read(dir.join("install.json")).unwrap();
    let config_path = dir.join("mcp-override.txt");
    let current = std::fs::read_to_string(&config_path).unwrap();
    let policy = ",default_tools_approval_mode=\"approve\"";
    assert!(current.contains(policy));
    let legacy = current.replace(policy, "");
    private_replace(&config_path, legacy.as_bytes()).unwrap();
    // The process double rejects implicit/prompted policy, then calls real MCP.
    host::mode(&dir, "interrupt");
    assert!(
        codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
            .await
            .is_err()
    );
    let first = host::state(&dir);
    assert_eq!(first["task_id"], f.task.id);
    assert!(first["session_id"].is_string());
    assert_eq!(first["completed"], false);
    host::mode(&dir, "complete");
    let recovered = host::cli(&dir, &binary, false).await;
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(host::state(&dir)["session_id"], first["session_id"]);
    assert_eq!(host::state(&dir)["run_id"], first["run_id"]);
    let events = host::events(&dir);
    assert!(!events.contains(f.issued.expose_secret()));
    assert!(!events.contains(&f.owner.credential));
    assert_eq!(std::fs::read(&config_path).unwrap(), legacy.as_bytes());
    assert_eq!(std::fs::read(dir.join("install.json")).unwrap(), manifest);
    let saved = host::state(&dir);
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let tampered = current.replace("\"approve\"", "\"prompt\"");
    private_replace(&config_path, tampered.as_bytes()).unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15)
        .await
        .unwrap_err()
        .to_string()
        .contains("configuration has changed"));
    assert_eq!(host::state(&dir), saved);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    private_replace(&config_path, legacy.as_bytes()).unwrap();
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    assert_eq!(host::state(&dir)["session_id"], first["session_id"]);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.close().await;
}
