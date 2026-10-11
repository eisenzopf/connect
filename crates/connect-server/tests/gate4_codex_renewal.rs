//! Owner-issued renewal preserves the exact host session through real local MCP.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_fixture.rs"]
mod host;
use connect_protocol::tasks::*;
use connect_server::cli::{
    init::{private_replace, private_write},
    task_access::{codex_host, GrantFile},
};
use connect_store::Store;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

async fn setup(ttl: i64) -> fixture::Fixture {
    fixture::Fixture::start_permitted_with_grant_ttl(
        vec![TaskPermission::History, TaskPermission::Inbox],
        ttl,
    )
    .await
}

async fn replacement(f: &fixture::Fixture) -> PathBuf {
    let request_id = uuid::Uuid::new_v4().to_string();
    let definition = GrantDefinition {
        task_id: f.task.id.clone(),
        binding: f.binding.clone(),
        operations: f.issued.grant.operations.clone(),
        expires_at: chrono::Utc::now().timestamp() + 180,
    };
    let issued = f
        .server
        .service
        .issue_task_grant(
            f.owner.credential_id.clone(),
            request_id.clone(),
            definition.clone(),
        )
        .await
        .unwrap();
    let file = GrantFile {
        profile: TASK_ACCESS_PROFILE.into(),
        request: TaskAccessRequest::new(TaskAccessCommand::Issue {
            request_id,
            definition,
        }),
        grant: Some(ProtectedTaskGrant::new(
            issued.grant.clone(),
            issued.expose_secret().into(),
        )),
    };
    let path = f
        .directory()
        .join(format!("renewal-{}.json", uuid::Uuid::new_v4()));
    private_write(&path, &serde_json::to_vec(&file).unwrap()).unwrap();
    path
}

fn original_files(directory: &Path) -> Vec<Vec<u8>> {
    [
        "install.json",
        "task-grant.json",
        "host-state.json",
        "mcp-override.txt",
    ]
    .iter()
    .map(|name| std::fs::read(directory.join(name)).unwrap())
    .collect()
}

#[tokio::test]
async fn real_expiry_then_renewal_resumes_the_same_session_without_rewriting_original_intent() {
    let f = setup(5).await;
    let (dir, binary) = host::install(&f);
    codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
        .await
        .unwrap();
    let saved = host::state(&dir);
    tokio::time::timeout(Duration::from_secs(6), async {
        while chrono::Utc::now().timestamp() < f.issued.grant.expires_at {
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    let file = replacement(&f).await;
    let originals = original_files(&dir);
    let store = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
            .args(["task-access", "renew-codex", "--installation"])
            .arg(&dir)
            .arg("--grant-file")
            .arg(&file)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["status"], "renewed-not-invoked");
    let renewed = std::fs::read(dir.join("grant-renewal.json")).unwrap();
    codex_host::renew(&dir, &file).unwrap();
    assert_eq!(
        std::fs::read(dir.join("grant-renewal.json")).unwrap(),
        renewed
    );
    assert_eq!(original_files(&dir), originals);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        store
    );
    assert_eq!(
        codex_host::diagnostics::inspect(Some(&dir), &f.task.id).state,
        codex_host::diagnostics::HostState::TurnComplete
    );
    let result = host::cli(&dir, &binary, false).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(host::state(&dir)["session_id"], saved["session_id"]);
    assert_eq!(host::state(&dir)["task_id"], saved["task_id"]);
    assert_ne!(host::state(&dir)["run_id"], saved["run_id"]);
    let current: GrantFile = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert!(!host::events(&dir).contains(current.grant.unwrap().expose_secret()));
    f.close().await;
}

#[tokio::test]
async fn interrupted_turn_retains_run_and_prompt_and_revoked_parent_still_denies_renewed_tools() {
    let f = setup(180).await;
    let (dir, binary) = host::install(&f);
    host::mode(&dir, "interrupt");
    assert!(
        codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
            .await
            .is_err()
    );
    let saved = host::state(&dir);
    let file = replacement(&f).await;
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    codex_host::renew(&dir, &file).unwrap();
    host::mode(&dir, "complete");
    codex_host::run(&dir, &binary, None, 15).await.unwrap();
    for field in ["session_id", "task_id", "prompt", "run_id"] {
        assert_eq!(host::state(&dir)[field], saved[field]);
    }
    f.revoke_parent();
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
async fn mismatched_delegation_pending_file_and_active_turn_cannot_replace_authority() {
    use fs2::FileExt;
    let f = setup(180).await;
    let other = setup(180).await;
    let (dir, binary) = host::install(&f);
    codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
        .await
        .unwrap();
    let originals = original_files(&dir);
    let foreign = replacement(&other).await;
    assert!(codex_host::renew(&dir, &foreign).is_err());
    let file = replacement(&f).await;
    let valid = std::fs::read(&file).unwrap();
    let mut changed: GrantFile = serde_json::from_slice(&valid).unwrap();
    changed.grant = None;
    private_replace(&file, &serde_json::to_vec(&changed).unwrap()).unwrap();
    assert!(codex_host::renew(&dir, &file).is_err());
    // Even internally consistent file edits cannot switch a running host's
    // delegated operations or invocation identity during credential adoption.
    for variant in ["operations", "binding", "expired"] {
        let mut changed: GrantFile = serde_json::from_slice(&valid).unwrap();
        let TaskAccessCommand::Issue { definition, .. } = &mut changed.request.command else {
            panic!("issue");
        };
        let grant = &mut changed.grant.as_mut().unwrap().grant;
        match variant {
            "operations" => {
                definition.operations = vec![TaskPermission::History];
                grant.operations = definition.operations.clone();
            }
            "binding" => {
                definition.binding.external_id = "another-session".into();
                grant.binding = definition.binding.clone();
            }
            _ => {
                definition.expires_at = chrono::Utc::now().timestamp() - 1;
                grant.expires_at = definition.expires_at;
            }
        }
        private_replace(&file, &serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(codex_host::renew(&dir, &file).is_err(), "{variant}");
    }
    private_replace(&file, &valid).unwrap();
    let held = std::fs::File::open(dir.join("host.lock")).unwrap();
    held.try_lock_exclusive().unwrap();
    assert!(codex_host::renew(&dir, &file)
        .unwrap_err()
        .to_string()
        .contains("active host turn"));
    FileExt::unlock(&held).unwrap();
    assert!(!dir.join("grant-renewal.json").exists());
    assert_eq!(original_files(&dir), originals);
    codex_host::renew(&dir, &file).unwrap();
    f.close().await;
    other.close().await;
}

#[tokio::test]
async fn invalid_renewal_pointer_never_falls_back_to_the_original_still_valid_grant() {
    let f = setup(180).await;
    let (dir, binary) = host::install(&f);
    codex_host::run(&dir, &binary, Some(&dir.join("prompt.txt")), 15)
        .await
        .unwrap();
    let file = replacement(&f).await;
    codex_host::renew(&dir, &file).unwrap();
    let path = dir.join("grant-renewal.json");
    let valid = std::fs::read(&path).unwrap();
    let originals = original_files(&dir);
    private_replace(&path, b"incomplete").unwrap();
    assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    assert!(codex_host::renew(&dir, &file).is_err());
    assert_eq!(original_files(&dir), originals);
    private_replace(&path, &valid).unwrap();
    // An interrupted private_replace can leave an unrelated temporary file;
    // only the committed pointer is eligible to supply current credentials.
    private_write(&dir.join("grant-renewal.abandoned.tmp"), b"partial").unwrap();
    codex_host::run(&dir, &binary, None, 15).await.unwrap();
    #[cfg(unix)]
    {
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(dir.join("missing"), &path).unwrap();
        assert!(codex_host::run(&dir, &binary, None, 15).await.is_err());
    }
    f.close().await;
}
