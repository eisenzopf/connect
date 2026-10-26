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
#[path = "canonical_mcp/application_dispatch.rs"]
mod application_dispatch;
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

/// A real MCP controller, without native peers, packets or guessed bindings.
pub(crate) struct TransitionProbe {
    client: McpClient,
    address: std::net::SocketAddr,
    token: String,
    arguments: Value,
    operation: String,
}
impl TransitionProbe {
    pub(crate) async fn submit(
        f: &Fixture,
        input: &connect_protocol::transitions::TransitionCommand,
        token: &str,
    ) -> Self {
        let mut command = input.clone();
        command.normalize().unwrap();
        assert_eq!(command.intent.required_scope(), "session.handoff");
        let mut payload = serde_json::to_value(&command.intent).unwrap();
        payload
            .as_object_mut()
            .unwrap()
            .retain(|key, value| key != "type" && !value.is_null());
        payload
            .as_object_mut()
            .unwrap()
            .extend(command.payload_extensions.clone());
        let mut arguments = json!({"conversation_id":command.conversation_id,"session_id":command.session_id,
            "operation_id":command.operation_id,"expected_revision":command.expected_revision,"payload":payload});
        if let Some(connection) = &command.connection_id {
            arguments["connection_id"] = json!(connection);
        }
        let address = f.server.mcp_address.unwrap();
        let client = client(address, token).await;
        success(call(&client, "connect_session_handoff", arguments.clone()).await);
        Self {
            client,
            address,
            token: token.into(),
            arguments,
            operation: command.operation_id,
        }
    }
    fn status_arguments(&self) -> Value {
        let mut scope = self.arguments.clone();
        scope.as_object_mut().unwrap().remove("operation_id");
        scope.as_object_mut().unwrap().remove("expected_revision");
        scope["payload"] = json!({"operation_id":self.operation});
        scope
    }
    async fn record(&self, client: &McpClient) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
        loop {
            let result = call(client, "connect_operation_get", self.status_arguments()).await;
            let value = result.structured_content.as_ref().unwrap();
            // Retry only current reads during bounded authority contention.
            // The mutation is never resubmitted to hide uncertainty.
            if result.is_error == Some(true) && value["status"] == 503 {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "MCP original status remained unavailable"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
            let value = success(result);
            let frames = value["data"]["envelopes"].as_array().unwrap();
            assert!(frames
                .iter()
                .all(|e| matches!(e["type"].as_str(), Some("operation.updated" | "ack"))));
            let record = frames
                .iter()
                .find(|e| e["type"] == "operation.updated")
                .unwrap()["payload"]
                .clone();
            assert_eq!(record["operation_id"], self.operation);
            if matches!(
                record["state"].as_str(),
                Some("completed" | "failed" | "cancelled" | "interrupted")
            ) {
                return record;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "MCP original operation did not settle"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    pub(crate) async fn failed_and_close(self, expected: &str) {
        let record = self.record(&self.client).await;
        assert_eq!(record["state"], "failed");
        assert_eq!(
            record["problem"],
            json!({"code":expected,"source":"native-handoff"})
        );
        success(
            call(
                &self.client,
                "connect_session_handoff",
                self.arguments.clone(),
            )
            .await,
        );
        let replay = self.record(&self.client).await;
        assert_eq!(replay["record_id"], record["record_id"]);
        assert_eq!(replay["problem"], record["problem"]);
        self.client.cancel().await.unwrap();
    }
    pub(crate) async fn completed(
        &self,
        expected: &connect_protocol::transitions::OperationRecord,
    ) {
        let record = self.record(&self.client).await;
        assert_eq!(record["state"], "completed");
        assert_eq!(record["record_id"], expected.record_id);
        assert_eq!(record["result"], expected.result.as_ref().unwrap().clone());
        let fresh = client(self.address, &self.token).await;
        let recovered = self.record(&fresh).await;
        assert_eq!(recovered["record_id"], record["record_id"]);
        assert_eq!(recovered["result"], record["result"]);
        fresh.cancel().await.unwrap();
    }
    pub(crate) async fn replay_and_close(self) {
        let before = self.record(&self.client).await;
        success(
            call(
                &self.client,
                "connect_session_handoff",
                self.arguments.clone(),
            )
            .await,
        );
        let after = self.record(&self.client).await;
        assert_eq!(after["record_id"], before["record_id"]);
        assert_eq!(after["result"], before["result"]);
        self.client.cancel().await.unwrap();
    }
}
#[tokio::test]
async fn canonical_mcp_public_control_preserves_messages_session_ids_original_status_and_current_revocation(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_configured_mcp(algorithm, &["session.handoff"]).await;
        let address = f.server.mcp_address.unwrap();
        let mcp = client(address, &f.client.credential).await;
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
        application_dispatch::receipt_contract(&f, &cid).await;
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
        f.server.shutdown().await.unwrap();
        assert!(
            std::net::TcpListener::bind(address).is_ok(),
            "owned MCP listener not drained"
        );
        eprintln!("canonical MCP {algorithm}: exact Message once, same HTTP/native history, Session original replay/status, current revocation, official client/listener/server joined; no media readiness inferred");
    }
}

#[tokio::test]
async fn canonical_mcp_configuration_rejects_public_or_provider_admission_before_startup() {
    let f = Fixture::new("EdDSA", 8).await;
    assert!(f.server.mcp_address.is_none());
    let mut public = f.config.clone();
    public.mcp = Some(connect_server::config::McpConfig {
        bind: "0.0.0.0:0".parse().unwrap(),
        binding: None,
    });
    assert!(public.validate().is_err());
    // A public MCP listener is invalid even when the canonical native profile
    // is Remote. It cannot become public by inheriting a native bind setting.
    public.network_profile = connect_protocol::native_network::NativeNetworkProfile::Remote;
    assert!(public.validate().is_err());
    for kind in [
        connect_protocol::tasks::InvocationKind::ProviderCall,
        connect_protocol::tasks::InvocationKind::ProviderChat,
    ] {
        let mut provider = f.config.clone();
        provider.mcp = Some(connect_server::config::McpConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            binding: Some(connect_protocol::tasks::InvocationBinding {
                kind,
                adapter: "fixture".into(),
                external_id: "verified-separately".into(),
            }),
        });
        assert!(provider.validate().is_err());
    }
    let mut value = serde_json::to_value(&f.config).unwrap();
    value["mcp"] = json!({"bind":"127.0.0.1:0","actor":"forged"});
    assert!(serde_json::from_value::<CanonicalServerConfig>(value).is_err());
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn canonical_mcp_failed_bind_releases_installation_and_repeated_shutdown_drop_releases_ports()
{
    let f = Fixture::with_configured_mcp("EdDSA", &[]).await;
    let config = f.config.clone();
    f.server.shutdown().await.unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut blocked = config.clone();
    blocked.mcp.as_mut().unwrap().bind = occupied.local_addr().unwrap();
    assert!(RunningCanonicalServer::start(blocked).await.is_err());
    drop(occupied);
    for explicit in [true, false, true] {
        let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
        let address = server.mcp_address.unwrap();
        assert!(std::net::TcpListener::bind(address).is_err());
        if explicit {
            server.shutdown().await.unwrap();
        } else {
            drop(server);
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if std::net::TcpListener::bind(address).is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn canonical_agent_export_retains_narrow_control_scope_and_current_revocation_without_parent_fallback(
) {
    let f = Fixture::with_configured_mcp("EdDSA", &[]).await;
    let issued = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope {
                operations: vec!["session.handoff".into(), "operation.get".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut source = f.client.clone();
    source.credential = issued.token.clone();
    let input = f.temp.path().join("control-source.toml");
    let config = f.temp.path().join("control-server.toml");
    let output = f.temp.path().join("control-export");
    connect_server::cli::init::private_write(&input, toml::to_string(&source).unwrap().as_bytes())
        .unwrap();
    connect_server::cli::init::private_write(
        &config,
        toml::to_string(&f.config).unwrap().as_bytes(),
    )
    .unwrap();
    let address = f.server.mcp_address.unwrap();
    let destination = output.clone();
    tokio::task::spawn_blocking(move || {
        connect_server::cli::agent_access::export(&input, &config, address, &destination)
    })
    .await
    .unwrap()
    .unwrap();
    let installed: toml::Value =
        toml::from_str(&std::fs::read_to_string(output.join("agent.toml")).unwrap()).unwrap();
    assert_eq!(
        installed["credential_id"].as_str(),
        Some(issued.id.as_str())
    );
    assert_eq!(
        installed["credential"].as_str(),
        Some(issued.token.as_str())
    );
    assert!(installed.get("signing_private_key").is_none());
    let mcp = client(address, installed["credential"].as_str().unwrap()).await;
    let tools = mcp.list_all_tools().await.unwrap();
    let names = tools
        .iter()
        .map(|t| t.name.as_ref())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "connect_inspect",
            "connect_session_handoff",
            "connect_operation_get"
        ])
    );
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let store = f.server.store.clone();
    tokio::task::spawn_blocking(move || store.revoke(&admin, &issued.id))
        .await
        .unwrap()
        .unwrap();
    assert!(mcp.list_all_tools().await.is_err());
    // The independent same-Identity credential remains available; the installed
    // narrow credential cannot substitute it after revocation.
    let independent = client(address, &f.client.credential).await;
    assert!(independent
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "connect_message_send"));
    independent.cancel().await.unwrap();
    mcp.cancel().await.unwrap();
    f.server.shutdown().await.unwrap();
}
