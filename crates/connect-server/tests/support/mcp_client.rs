//! Shared independent MCP client for public operation fixtures.
#![allow(clippy::unwrap_used, dead_code)]
use super::fixture;
use connect_protocol::tasks::TaskPermission as P;
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ClientConfig},
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde_json::Value;
use std::time::Duration;
type Client = RunningService<RoleClient, ClientConfig>;
pub fn permissions() -> Vec<P> {
    vec![
        P::Resolve,
        P::Capabilities,
        P::Send,
        P::History,
        P::ContextView,
        P::ContextDeliveries,
        P::Inbox,
        P::RequestStatus,
        P::ArtifactBegin,
        P::ArtifactAppend,
        P::ArtifactFinish,
        P::ArtifactGet,
        P::ArtifactDelete,
        P::SessionInvite,
        P::SessionAccept,
        P::SessionReject,
        P::SessionCancel,
        P::SessionEnd,
        P::SessionGet,
        P::CallStart,
        P::CallStatus,
        P::CallEnd,
        P::SipDial,
        P::SipStatus,
        P::SipEnd,
    ]
}
pub async fn client(f: &fixture::Fixture) -> Client {
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(format!(
                "http://{}/mcp",
                f.server.mcp_address.unwrap()
            ))
            .auth_header(f.issued.expose_secret()),
        ))
        .await
        .unwrap()
}
pub async fn call(client: &Client, name: &str, args: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}
pub fn success(result: CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "tool returned a business error"
    );
    result.structured_content.unwrap()
}
