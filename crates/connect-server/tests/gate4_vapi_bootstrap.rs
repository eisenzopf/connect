//! First callback binds from private nonce proof, including lost POST receipt.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_bootstrap.rs"]
mod bootstrap;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/vapi_bindings.rs"]
mod provider;
use connect_client::ConnectClient;
use connect_mcp::VapiOrigin;
use connect_protocol::tasks::{TaskCommand, TaskPermission};
use connect_server::vapi_bootstrap;
use connect_store::Store;
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::json;
use std::time::Duration;

async fn client(
    setup: &bootstrap::Setup,
) -> rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig> {
    let http = provider::http(provider::headers(VapiOrigin::ChatSession, "session-one")).await;
    rmcp::model::ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(format!(
                "http://{}/mcp",
                setup.endpoint.address
            ))
            .auth_header(setup.secret()),
        ))
        .await
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callback_before_creation_receipt_binds_once_and_retries_one_real_mcp_effect() {
    let f = fixture::Fixture::start_permitted(
        false,
        vec![TaskPermission::History, TaskPermission::Send],
    )
    .await;
    let setup = bootstrap::Setup::start(&f, 200, Duration::from_secs(3)).await;
    let http = provider::http(provider::headers(VapiOrigin::ChatSession, "session-one")).await;
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"first-callback-fixture","version":"1"}}});
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(http
        .post(format!("http://{}/mcp", setup.endpoint.address))
        .header("Accept", "application/json, text/event-stream")
        .bearer_auth(setup.secret())
        .json(&init)
        .send()
        .await
        .unwrap()
        .status()
        .is_client_error());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let service = f.server.service.clone();
    let sdk = setup.provider.clone();
    let directory = setup.directory.clone();
    let creating =
        tokio::spawn(
            async move { vapi_bootstrap::create_session(service, &sdk, &directory).await },
        );
    setup.posted().await;
    assert!(!creating.is_finished());
    let client = client(&setup).await;
    let mut first = None;
    for _ in 0..2 {
        let result=client.call_tool(CallToolRequestParams::new("connect_send").with_arguments(json!({"request_id":"first-bootstrap-send","step_id":"draft","body":"Draft from the verified first callback"}).as_object().unwrap().clone())).await.unwrap();
        assert_ne!(result.is_error, Some(true));
        let content = result.structured_content.unwrap();
        assert_eq!(content["data"]["task_id"], f.task.id);
        if let Some(previous) = &first {
            assert_eq!(&content, previous);
        } else {
            first = Some(content);
        }
    }
    assert_eq!(
        bootstrap::read(&setup.directory)["external_id"],
        "session-one"
    );
    assert_eq!(creating.await.unwrap().unwrap(), "session-one");
    let requests = setup.api.received_requests().await.unwrap();
    let post = requests.iter().find(|r| r.method == "POST").unwrap();
    let body: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert_eq!(
        body["assistantOverrides"]["variableValues"]["connect_invocation_id"],
        setup.state["nonce"]
    );
    assert_eq!(
        body["assistantOverrides"]["tools:append"][0]["server"]["url"],
        format!("http://{}/mcp", setup.endpoint.address)
    );
    assert!(body["assistantOverrides"].get("model").is_none());
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    assert_eq!(
        peer.history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    peer.close().await;
    let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
    owner
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    owner
        .task(TaskCommand::Cancel {
            task_id: f.task.id.clone(),
            request_id: "cancel-bootstrap-task".into(),
            reason: None,
        })
        .await
        .unwrap();
    owner.close().await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(client.list_tools(None).await.is_err());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    client.cancel().await.unwrap();
    setup.close().await;
    f.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lost_creation_receipt_keeps_original_intent_and_callback_recovers_without_second_post() {
    let f = fixture::Fixture::start_permitted(false, vec![TaskPermission::History]).await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    assert!(vapi_bootstrap::create_session(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory
    )
    .await
    .is_err());
    let state = bootstrap::read(&setup.directory);
    assert_eq!(state["posted"], true);
    assert!(state["external_id"].is_null());
    assert!(vapi_bootstrap::create_session(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory
    )
    .await
    .is_err());
    assert_eq!(bootstrap::read(&setup.directory), state);
    let client = client(&setup).await;
    let result = client
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .unwrap();
    assert_eq!(result.structured_content.unwrap()["data"]["id"], f.task.id);
    assert_eq!(
        vapi_bootstrap::create_session(f.server.service.clone(), &setup.provider, &setup.directory)
            .await
            .unwrap(),
        "session-one"
    );
    let final_state = bootstrap::read(&setup.directory);
    assert_eq!(final_state["nonce"], state["nonce"]);
    assert_eq!(final_state["secret"], state["secret"]);
    assert_eq!(final_state["definition"], state["definition"]);
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
    client.cancel().await.unwrap();
    setup.close().await;
    f.close().await;
}
