//! Private SDK verification → atomic issuance → actual MCP → public QUIC client.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/vapi_bindings.rs"]
mod provider;
#[path = "support/vapi_setup.rs"]
mod setup;
use connect_client::ConnectClient;
use connect_connectors::vapi_origin::OriginKind;
use connect_mcp::{RunningMcp, VapiOrigin};
use connect_protocol::tasks::TaskPermission;
use connect_server::vapi_task_setup;
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::json;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verified_call_chat_and_session_setup_replay_one_authorized_mcp_effect() {
    for (kind, origin, namespace) in [
        (OriginKind::Call, VapiOrigin::Call, "vapi-call"),
        (OriginKind::Chat, VapiOrigin::Chat, "vapi-chat"),
        (
            OriginKind::ChatSession,
            VapiOrigin::ChatSession,
            "vapi-chat-session",
        ),
    ] {
        // Construct the SDK before the QUIC fixture, avoiding synchronous macOS
        // proxy discovery inside an active media/client initialization window.
        let (api, sdk) = setup::private_api(kind, setup::resource(kind), 200, 2).await;
        let f = fixture::Fixture::start_permitted(
            false,
            vec![TaskPermission::History, TaskPermission::Send],
        )
        .await;
        let request = setup::setup(&f, kind);
        let expiry = request.expires_at;
        let verified = vapi_task_setup::issue(f.server.service.clone(), &sdk, request)
            .await
            .unwrap();
        let mut retry = setup::setup(&f, kind);
        retry.expires_at = expiry;
        let replay = vapi_task_setup::issue(f.server.service.clone(), &sdk, retry)
            .await
            .unwrap();
        assert_eq!(verified.issued().grant, replay.issued().grant);
        assert_eq!(
            verified.issued().expose_secret(),
            replay.issued().expose_secret()
        );
        assert_eq!(verified.issued().grant.binding.adapter, namespace);
        let auth = verified.authenticator(f.server.service.clone()).unwrap();
        let listener = RunningMcp::start_local(
            f.server.service.clone(),
            Arc::new(auth),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let url = format!("http://{}/mcp", listener.address);
        let http = provider::http(provider::headers(origin, "resource-one")).await;
        let client = rmcp::model::ClientConfig::default()
            .serve(StreamableHttpClientTransport::with_client(
                http.clone(),
                StreamableHttpClientTransportConfig::with_uri(url.clone())
                    .auth_header(verified.issued().expose_secret()),
            ))
            .await
            .unwrap();
        let status = client
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap();
        assert_eq!(status.structured_content.unwrap()["data"]["id"], f.task.id);
        let arguments = json!({"request_id":"verified-draft","step_id":"draft","body":"Verified provider draft"});
        let mut receipt = None;
        for _ in 0..2 {
            let result = client
                .call_tool(
                    CallToolRequestParams::new("connect_send")
                        .with_arguments(arguments.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            assert_ne!(result.is_error, Some(true));
            let result = result.structured_content.unwrap();
            assert_eq!(result["data"]["task_id"], f.task.id);
            if let Some(first) = &receipt {
                assert_eq!(&result, first);
            } else {
                receipt = Some(result);
            }
        }
        client.cancel().await.unwrap();
        f.server
            .service
            .revoke_task_grant(
                f.owner.credential_id.clone(),
                verified.issued().grant.id.clone(),
            )
            .await
            .unwrap();
        let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
        let denied = http
            .post(&url)
            .header("accept", "application/json, text/event-stream")
            .bearer_auth(verified.issued().expose_secret())
            .json(&provider::mutation("verified-draft"))
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 401);
        assert_eq!(
            connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
        let messages = peer
            .history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Verified provider draft");
        peer.close().await;
        api.verify().await;
        listener.shutdown().await.unwrap();
        f.close().await;
    }
}
