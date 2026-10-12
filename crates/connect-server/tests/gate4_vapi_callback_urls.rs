//! Callback publication is protected configuration, not a new listener exposure.
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
use connect_store::Store;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_addresses_have_no_side_effect_and_https_never_opens_public_ingress() {
    let api = wiremock::MockServer::start().await;
    let base = api.uri();
    let provider =
        tokio::task::spawn_blocking(move || Arc::new(Vapi::new("local-private", &base).unwrap()))
            .await
            .unwrap();
    let mut f = media::Fixture::start().await;
    call_setup::route(&f);
    let (_, config) = f
        .grant("url-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "url-session").await;
    let temp = tempfile::tempdir().unwrap();
    let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    for (index, url) in [
        "http://example.invalid/mcp",
        "http://0.0.0.0/mcp",
        "ftp://example.invalid/mcp",
        "https://user@example.invalid/mcp",
        "https://user:password@example.invalid/mcp",
        "https://example.invalid/mcp?task=other",
        "https://example.invalid/mcp#other",
        "https://example.invalid/other",
        "not a URL",
    ]
    .into_iter()
    .enumerate()
    {
        let directory = temp.path().join(format!("invalid-{index}"));
        assert!(f
            .h
            .server
            .prepare_vapi_task_session_with_callback(
                &session.id,
                call_setup::definition(&f),
                &directory,
                provider.clone(),
                Some(url),
            )
            .await
            .is_err());
        assert!(!directory.exists());
        assert_eq!(
            Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
            before
        );
    }
    let directory = temp.path().join("https-setup");
    let advertised = "https://connect.example.invalid/mcp";
    let local =
        f.h.server
            .prepare_vapi_task_session_with_callback(
                &session.id,
                call_setup::definition(&f),
                &directory,
                provider.clone(),
                Some(advertised),
            )
            .await
            .unwrap();
    assert!(local.ip().is_loopback());
    let intent = std::fs::read(directory.join("intent.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&intent).unwrap();
    assert_eq!(saved["mcp_url"], advertised);
    assert!(f
        .h
        .server
        .store
        .vapi_session_requires_task_setup(&session.id)
        .unwrap());
    assert!(f
        .h
        .server
        .prepare_vapi_task_session_with_callback(
            &session.id,
            call_setup::definition(&f),
            &temp.path().join("replacement"),
            provider,
            Some("https://different.example.invalid/mcp"),
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("already has"));
    assert_eq!(
        std::fs::read(directory.join("intent.json")).unwrap(),
        intent
    );
    assert!(api.received_requests().await.unwrap().is_empty());
    client.close().await;
    f.close().await;
    assert!(tokio::net::TcpStream::connect(local).await.is_err());
}
