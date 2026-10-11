//! Retained operator configuration cannot refresh or replace an invocation.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_bootstrap.rs"]
mod bootstrap;
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_client::ConnectClient;
use connect_protocol::tasks::TaskCommand;
use connect_server::vapi_bootstrap::{self, Definition};
use connect_store::Store;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retained_setup_requires_exact_configuration_and_current_authority() {
    let f = fixture::Fixture::start().await;
    let setup = bootstrap::Setup::start(&f, 200, Duration::ZERO).await;
    // Satisfy the fixture's expected POST; verify_prepared itself never creates.
    vapi_bootstrap::create_session(f.server.service.clone(), &setup.provider, &setup.directory)
        .await
        .unwrap();
    let state = bootstrap::read(&setup.directory);
    let definition: Definition = serde_json::from_value(state["definition"].clone()).unwrap();
    let url = state["mcp_url"].as_str().unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    vapi_bootstrap::verify_prepared(
        f.server.service.clone(),
        &definition,
        Some(url),
        &setup.directory,
    )
    .await
    .unwrap();
    let mut longer = definition.clone();
    longer.expires_at += 1;
    let mut other_task = definition.clone();
    other_task.task_id = "other-task".into();
    let mut other_credential = definition.clone();
    other_credential.credential_id = "other-owner".into();
    let mut other_permissions = definition.clone();
    other_permissions.operations.clear();
    for changed in [longer, other_task, other_credential, other_permissions] {
        assert!(vapi_bootstrap::verify_prepared(
            f.server.service.clone(),
            &changed,
            Some(url),
            &setup.directory
        )
        .await
        .is_err());
    }
    assert!(vapi_bootstrap::verify_prepared(
        f.server.service.clone(),
        &definition,
        Some("https://other.example.invalid/mcp"),
        &setup.directory
    )
    .await
    .is_err());
    assert_eq!(bootstrap::read(&setup.directory), state);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
    owner
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    owner
        .task(TaskCommand::Cancel {
            task_id: f.task.id.clone(),
            request_id: "cancel-retained".into(),
            reason: None,
        })
        .await
        .unwrap();
    assert!(vapi_bootstrap::verify_prepared(
        f.server.service.clone(),
        &definition,
        Some(url),
        &setup.directory
    )
    .await
    .is_err());
    owner.close().await;
    setup.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reopening_uncertain_setup_never_reposts_or_refreshes_intent() {
    let f = fixture::Fixture::start().await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    assert!(vapi_bootstrap::create_session(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory
    )
    .await
    .is_err());
    let state = bootstrap::read(&setup.directory);
    let definition: Definition = serde_json::from_value(state["definition"].clone()).unwrap();
    vapi_bootstrap::verify_prepared(
        f.server.service.clone(),
        &definition,
        state["mcp_url"].as_str(),
        &setup.directory,
    )
    .await
    .unwrap();
    assert!(vapi_bootstrap::create_session(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory
    )
    .await
    .is_err());
    assert_eq!(bootstrap::read(&setup.directory), state);
    assert_eq!(
        setup
            .api
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    setup.close().await;
    f.close().await;
}
