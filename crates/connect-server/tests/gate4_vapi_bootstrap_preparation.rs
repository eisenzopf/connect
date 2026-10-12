//! Preparation is local, private, current-authorized and never overwrites intent.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_connectors::vapi_origin::OriginKind;
use connect_protocol::tasks::TaskPermission;
use connect_server::vapi_bootstrap::{self, Definition};
use connect_store::Store;

fn definition(f: &fixture::Fixture) -> Definition {
    Definition {
        credential_id: f.owner.credential_id.clone(),
        request_id: "prepare-grant".into(),
        task_id: f.task.id.clone(),
        assistant_id: "assistant-one".into(),
        origin: OriginKind::ChatSession,
        operations: f.task.scope.operations.clone(),
        expires_at: chrono::Utc::now().timestamp() + 120,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_configuration_denies_without_state_and_valid_intent_cannot_be_overwritten() {
    let f = fixture::Fixture::start().await;
    let directory = f.directory().join("private-setup");
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    for url in [
        "http://example.com/mcp",
        "https://user:secret@example.com/mcp",
        "https://example.com/other",
        "https://example.com/mcp?task=other",
        "https://example.com/mcp#other",
        "ftp://example.com/mcp",
    ] {
        assert!(
            vapi_bootstrap::prepare(f.server.service.clone(), definition(&f), url, &directory)
                .await
                .is_err()
        );
        assert!(!directory.exists());
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
    }
    for assistant in [
        "",
        "../assistant",
        "assistant:other",
        "assistant.other",
        "assistant other",
    ] {
        let mut d = definition(&f);
        d.assistant_id = assistant.into();
        assert!(vapi_bootstrap::prepare(
            f.server.service.clone(),
            d,
            "http://127.0.0.1:5000/mcp",
            &directory
        )
        .await
        .is_err());
        assert!(!directory.exists());
    }
    let mut d = definition(&f);
    d.operations.push(TaskPermission::Send);
    assert!(vapi_bootstrap::prepare(
        f.server.service.clone(),
        d,
        "http://127.0.0.1:5000/mcp",
        &directory
    )
    .await
    .is_err());
    assert!(!directory.exists());
    let d = definition(&f);
    vapi_bootstrap::prepare(
        f.server.service.clone(),
        d.clone(),
        "http://127.0.0.1:5000/mcp",
        &directory,
    )
    .await
    .unwrap();
    let intent = std::fs::read(directory.join("intent.json")).unwrap();
    assert!(vapi_bootstrap::prepare(
        f.server.service.clone(),
        d,
        "http://127.0.0.1:5000/mcp",
        &directory
    )
    .await
    .is_err());
    assert_eq!(
        std::fs::read(directory.join("intent.json")).unwrap(),
        intent
    );
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for path in [
            &directory,
            &directory.join("intent.json"),
            &directory.join("bootstrap.lock"),
        ] {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }
    f.revoke_parent();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let unused = f.directory().join("revoked-setup");
    assert!(vapi_bootstrap::prepare(
        f.server.service.clone(),
        definition(&f),
        "http://127.0.0.1:5000/mcp",
        &unused
    )
    .await
    .is_err());
    assert!(!unused.exists());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.close().await;
}
