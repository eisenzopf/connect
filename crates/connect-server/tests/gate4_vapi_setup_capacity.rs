//! Server composition bounds idle task endpoints without touching provider state.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::ConnectClient;
use connect_connectors::Vapi;
use connect_mcp::MAX_MCP_LISTENERS;
use connect_store::Store;
use std::{sync::Arc, time::Duration};

async fn provider() -> (wiremock::MockServer, Arc<Vapi>) {
    let api = wiremock::MockServer::start().await;
    let base = api.uri();
    let client =
        tokio::task::spawn_blocking(move || Arc::new(Vapi::new("local-fixture", &base).unwrap()))
            .await
            .unwrap();
    (api, client)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_rejects_excess_setups_before_private_intent_and_releases_all_on_shutdown() {
    // Prepare the reusable HTTP client before opening native control sockets.
    let (api, provider) = provider().await;
    let mut f = media::Fixture::start().await;
    call_setup::route(&f);
    let (_, config) = f
        .grant("capacity-native", 240, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut addresses = Vec::new();
    assert!(f.h.server.mcp_address.is_none());
    for index in 0..MAX_MCP_LISTENERS {
        let session = f.session(&mut client, &format!("capacity-{index}")).await;
        let mut definition = call_setup::definition(&f);
        definition.request_id = format!("capacity-grant-{index}");
        addresses.push(
            f.h.server
                .prepare_vapi_task_session(
                    &session.id,
                    definition,
                    &directory.path().join(index.to_string()),
                    provider.clone(),
                )
                .await
                .unwrap(),
        );
    }
    let session = f.session(&mut client, "excess-session").await;
    let rejected = directory.path().join("excess");
    let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let error =
        f.h.server
            .prepare_vapi_task_session(
                &session.id,
                call_setup::definition(&f),
                &rejected,
                provider.clone(),
            )
            .await
            .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert!(!rejected.exists());
    assert_eq!(
        Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    assert!(api.received_requests().await.unwrap().is_empty());
    let intent = std::fs::read(directory.path().join("0/intent.json")).unwrap();
    client.close().await;
    f.h.close().await;
    for address in addresses {
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
    }
    assert_eq!(
        std::fs::read(directory.path().join("0/intent.json")).unwrap(),
        intent
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_idle_setup_closes_endpoint_but_preserves_task_and_recovery_intent() {
    // Prepare the reusable HTTP client before opening native control sockets.
    let (api, provider) = provider().await;
    let mut f = media::Fixture::start().await;
    call_setup::route(&f);
    let (_, config) = f
        .grant("expiry-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "expiry-session").await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("setup");
    let mut definition = call_setup::definition(&f);
    definition.expires_at = chrono::Utc::now().timestamp() + 3;
    let address =
        f.h.server
            .prepare_vapi_task_session(&session.id, definition, &path, provider.clone())
            .await
            .unwrap();
    let intent = std::fs::read(path.join("intent.json")).unwrap();
    let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    assert!(tokio::net::TcpStream::connect(address).await.is_ok());
    tokio::time::timeout(Duration::from_secs(6), async {
        while tokio::net::TcpStream::connect(address).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(path.join("intent.json")).unwrap(), intent);
    assert_eq!(
        Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    assert!(api.received_requests().await.unwrap().is_empty());
    // Closed metadata still blocks a fresh setup for the same session; expiry
    // never silently substitutes a new task tool credential or ordinary call.
    let replacement = directory.path().join("replacement");
    assert!(f
        .h
        .server
        .prepare_vapi_task_session(
            &session.id,
            call_setup::definition(&f),
            &replacement,
            provider,
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("already has"));
    client.negotiate_task(&f.task.id).await.unwrap();
    client.close().await;
    f.h.close().await;
}
