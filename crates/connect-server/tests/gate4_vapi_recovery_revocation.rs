//! Operator recovery cannot mint a replacement for a revoked original grant.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_bootstrap.rs"]
mod bootstrap;
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_client::tasks::*;
use connect_server::vapi_bootstrap::reconcile_created;
use connect_store::Store;
use std::time::Duration;

#[tokio::test]
async fn protected_resource_recovery_preserves_revocation_and_original_intent() {
    let f = fixture::Fixture::start().await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    setup.uncertain(&f).await;
    reconcile_created(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory,
        "session-one",
    )
    .await
    .unwrap();
    let frozen = bootstrap::read(&setup.directory);
    // Protected owner inspection of the original issuance receipt, not a new
    // verification mode or authority supplied to a model tool.
    let issued = f
        .server
        .service
        .issue_verified_provider_task_grant(
            f.owner.credential_id.clone(),
            frozen["definition"]["request_id"].as_str().unwrap().into(),
            GrantDefinition {
                task_id: f.task.id.clone(),
                binding: InvocationBinding {
                    kind: InvocationKind::ProviderChat,
                    adapter: "vapi-chat-session".into(),
                    external_id: "session-one".into(),
                },
                operations: f.task.scope.operations.clone(),
                expires_at: frozen["definition"]["expires_at"].as_i64().unwrap(),
            },
        )
        .await
        .unwrap();
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), issued.grant.id)
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(reconcile_created(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory,
        "session-one"
    )
    .await
    .is_err());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    assert_eq!(bootstrap::read(&setup.directory), frozen);
    assert_eq!(
        setup
            .api
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
    setup.close().await;
    f.close().await;
}
