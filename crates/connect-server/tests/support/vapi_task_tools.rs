//! Protected session setup; model interactions use the actual stateless MCP SDK.
#![allow(clippy::unwrap_used, dead_code)]
use super::api;
use connect_client::{tasks::*, ClientConfig};
use connect_connectors::vapi_origin::OriginKind;
use connect_mcp::RunningMcp;
use connect_server::{
    runtime::RunningServer,
    vapi_bootstrap::{self, BootstrapAuth, Definition},
};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
pub type Client = rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig>;
pub struct Endpoint {
    pub listener: RunningMcp,
    directory: PathBuf,
}
impl Endpoint {
    pub async fn start(
        server: &RunningServer,
        owner: &ClientConfig,
        task: &TaskRecord,
        api: &api::Fixture,
        directory: &Path,
        operations: Vec<TaskPermission>,
    ) -> Self {
        let listener = RunningMcp::start_local_auxiliary(
            server.service.clone(),
            Arc::new(BootstrapAuth::new(
                server.service.clone(),
                api.provider.clone(),
                directory.into(),
            )),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        vapi_bootstrap::prepare(
            server.service.clone(),
            Definition {
                credential_id: owner.credential_id.clone(),
                request_id: "continued-session-grant".into(),
                task_id: task.id.clone(),
                assistant_id: api::ASSISTANT.into(),
                origin: OriginKind::ChatSession,
                operations,
                expires_at: task.expires_at.min(chrono::Utc::now().timestamp() + 120),
            },
            &format!("http://{}/mcp", listener.address),
            directory,
        )
        .await
        .unwrap();
        assert_eq!(
            vapi_bootstrap::create_session(server.service.clone(), &api.provider, directory)
                .await
                .unwrap(),
            api::SESSION
        );
        Self {
            listener,
            directory: directory.into(),
        }
    }
    pub async fn client(&self) -> Client {
        let state: Value =
            serde_json::from_slice(&std::fs::read(self.directory.join("intent.json")).unwrap())
                .unwrap();
        let mut headers = reqwest_mcp::header::HeaderMap::new();
        headers.insert("x-session-id", api::SESSION.parse().unwrap());
        headers.insert("x-chat-id", "continuation-chat".parse().unwrap());
        let http = tokio::task::spawn_blocking(move || {
            reqwest_mcp::Client::builder()
                .no_proxy()
                .default_headers(headers)
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap()
        })
        .await
        .unwrap();
        rmcp::model::ClientConfig::default()
            .serve(StreamableHttpClientTransport::with_client(
                http,
                StreamableHttpClientTransportConfig::with_uri(format!(
                    "http://{}/mcp",
                    self.listener.address
                ))
                .auth_header(state["secret"].as_str().unwrap()),
            ))
            .await
            .unwrap()
    }
    pub async fn close(self) {
        self.listener.shutdown().await.unwrap();
    }
}
pub async fn call(client: &Client, name: &str, args: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap_or_else(|error| panic!("MCP {name} failed: {error}"))
}
pub fn outcome(result: CallToolResult) -> TaskOutcome {
    assert_ne!(result.is_error, Some(true));
    serde_json::from_value(result.structured_content.unwrap()).unwrap()
}
pub fn inbox(result: CallToolResult) -> (Vec<TaskEvent>, Option<String>) {
    let TaskOutcome::Inbox { events, next } = outcome(result) else {
        panic!("inbox")
    };
    (events, next)
}
pub fn sources(result: CallToolResult) -> Vec<TaskSource> {
    let TaskOutcome::Recall { sources, .. } = outcome(result) else {
        panic!("recall")
    };
    sources
}
