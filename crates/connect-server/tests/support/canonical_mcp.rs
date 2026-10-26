//! Official independent MCP client against current production service control.
use super::*;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ClientConfig as McpClientConfig},
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
type McpClient = RunningService<RoleClient, McpClientConfig>;
async fn client(address: std::net::SocketAddr, token: &str) -> McpClient {
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    McpClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(format!("http://{address}/mcp"))
                .auth_header(token),
        ))
        .await
        .unwrap()
}
async fn call(client: &McpClient, name: &str, input: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(input.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}
fn success(result: CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "{:?}",
        result.structured_content
    );
    let value = result.structured_content.unwrap();
    assert_eq!(value["kind"], "canonical");
    assert_eq!(value["status"], 200);
    assert_eq!(value["data"]["profile"], "connect/2");
    value
}
#[tokio::test]
async fn canonical_mcp_public_control_preserves_messages_session_ids_original_status_and_current_revocation(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_extra_scopes(algorithm, 8, true, false, false, &["session.handoff"])
            .await;
        let listener = connect_mcp::RunningMcp::start_local(
            f.server.service.clone(),
            std::sync::Arc::new(connect_mcp::LocalAuth::new(f.server.service.clone())),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let mcp = client(listener.address, &f.client.credential).await;
        let tools = mcp.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|t| t.name == "connect_session_handoff"));
        assert!(tools.iter().any(|t| t.name == "connect_operation_get"));
        assert!(!tools.iter().any(|t| matches!(
            t.name.as_ref(),
            "connect_send"
                | "connect_create"
                | "connect_history"
                | "connect_request_status"
                | "connect_participant_add"
        )));
        let names = tools
            .iter()
            .map(|t| t.name.as_ref())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names.len(), tools.len());
        let input = json!({"operation_id":"mcp-create","payload":{"tenant_id":"tnt_default","policy":"persistent","initial_participants":[{"identity_id":f.peer.identity_id,"role":"participant"}]}});
        let created = success(call(&mcp, "connect_conversation_create", input.clone()).await);
        let cid = created["data"]["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "ack")
            .unwrap()["payload"]["details"]["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let replay = success(call(&mcp, "connect_conversation_create", input).await);
        assert_eq!(
            replay["data"]["envelopes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["type"] == "ack")
                .unwrap()["payload"]["details"]["conversation"]["id"],
            cid
        );
        let mut forged = json!({"conversation_id":cid,"operation_id":"forged","payload":{},"actor":"forged-owner"});
        assert!(mcp
            .call_tool(
                CallToolRequestParams::new("connect_message_send")
                    .with_arguments(forged.as_object_mut().unwrap().clone())
            )
            .await
            .is_err());
        let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut http =
            connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
                .await
                .unwrap();
        let baseline = success(
            call(
                &mcp,
                "connect_conversation_events",
                json!({"conversation_id":cid,"payload":{"live":false,"limit":100}}),
            )
            .await,
        );
        let cursor = baseline["data"]["envelopes"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["payload"]["details"]["cursor"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut draft = connect_client::MessageDraft::text_all("current canonical MCP message");
        draft.msg_id = Some("mcp-message".into());
        let input = json!({"conversation_id":cid,"operation_id":"mcp-send","payload":draft});
        let sent = success(call(&mcp, "connect_message_send", input.clone()).await);
        assert_eq!(
            sent["data"]["envelopes"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["payload"]["details"]["data"]["receipt"]["operation_id"],
            "mcp-send"
        );
        success(call(&mcp, "connect_message_send", input).await);
        let history = native
            .history(&cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].msg_id.as_str(), "mcp-message");
        assert_eq!(
            http.history(&cid, serde_json::from_value(json!({"limit":100})).unwrap())
                .await
                .unwrap()
                .messages[0]
                .msg_id
                .as_str(),
            "mcp-message"
        );
        let events = success(
            call(
                &mcp,
                "connect_conversation_events",
                json!({"conversation_id":cid,"payload":{"after":cursor,"live":false,"limit":100}}),
            )
            .await,
        );
        assert!(events["data"]["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["type"] == "conversation.event"));
        let invite = json!({"conversation_id":cid,"session_id":"mcp-session","operation_id":"mcp-invite","payload":{"to":[],"medium":"voice","intent":"conference","capabilities_offer":{"audio":["g.711-mu"]}}});
        let invited = success(call(&mcp, "connect_session_invite", invite.clone()).await);
        assert!(invited["data"]["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["sid"] == "mcp-session"));
        success(call(&mcp, "connect_session_invite", invite).await);
        let status = success(call(&mcp,"connect_operation_get",json!({"conversation_id":cid,"session_id":"mcp-session","payload":{"operation_id":"mcp-invite"}})).await);
        let record = status["data"]["envelopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["type"] == "operation.updated")
            .unwrap();
        assert_eq!(record["payload"]["operation_id"], "mcp-invite");
        assert_eq!(record["payload"]["state"], "completed");
        let refused = call(&mcp,"connect_operation_get",json!({"conversation_id":"inaccessible-room","session_id":"mcp-session","payload":{"operation_id":"mcp-invite"}})).await;
        assert_eq!(refused.is_error, Some(true));
        assert!(
            refused.structured_content.unwrap()["status"]
                .as_u64()
                .unwrap()
                >= 400
        );
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let credential_id = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &credential_id))
            .await
            .unwrap()
            .unwrap();
        assert!(mcp.call_tool(CallToolRequestParams::new("connect_operation_get").with_arguments(json!({"conversation_id":cid,"session_id":"mcp-session","payload":{"operation_id":"mcp-invite"}}).as_object().unwrap().clone())).await.is_err());
        native.shutdown().await.unwrap();
        mcp.cancel().await.unwrap();
        listener.shutdown().await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!("canonical MCP {algorithm}: exact Message once, same HTTP/native history, Session original replay/status, current revocation, official client/listener/server joined; no media readiness inferred");
    }
}
