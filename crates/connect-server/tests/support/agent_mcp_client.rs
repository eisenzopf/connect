//! Independent official MCP client; ordinary operations retain native outcomes.
#![allow(clippy::unwrap_used, dead_code)]
use connect_protocol::{Error, Operation, Outcome};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ClientConfig, ErrorCode},
    service::{RunningService, ServiceError},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde_json::Value;
use std::{net::SocketAddr, time::Duration};

pub type Client = RunningService<RoleClient, ClientConfig>;

pub async fn http() -> reqwest_mcp::Client {
    tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap()
    })
    .await
    .unwrap()
}

pub async fn client(address: SocketAddr, token: &str) -> Client {
    client_with_http(address, token, http().await).await
}

pub async fn client_with_http(
    address: SocketAddr,
    token: &str,
    http: reqwest_mcp::Client,
) -> Client {
    ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(format!("http://{address}/mcp"))
                .auth_header(token),
        ))
        .await
        .unwrap()
}

/// Native operation serialization is independent of the MCP implementation.
/// It defines parity expectations, including native optional fields and IDs.
pub async fn operation(client: &Client, operation: &Operation) -> CallToolResult {
    let mut args = serde_json::to_value(operation).unwrap();
    let name = format!(
        "connect_{}",
        args.as_object_mut()
            .unwrap()
            .remove("op")
            .unwrap()
            .as_str()
            .unwrap()
    );
    call(client, &name, args).await
}

pub fn params(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(args.as_object().unwrap().clone())
}

pub async fn call(client: &Client, name: &str, args: Value) -> CallToolResult {
    client.call_tool(params(name, args)).await.unwrap()
}

pub fn outcome(result: CallToolResult) -> Outcome {
    assert_ne!(
        result.is_error,
        Some(true),
        "ordinary communication was rejected"
    );
    serde_json::from_value(result.structured_content.unwrap()).unwrap()
}

pub fn error(result: CallToolResult) -> Error {
    assert_eq!(result.is_error, Some(true));
    serde_json::from_value(result.structured_content.unwrap()).unwrap()
}

/// A schema rejection must be an actual MCP parameter error, never a timeout.
pub async fn invalid(client: &Client, name: &str, args: Value) {
    match client.call_tool(params(name, args)).await {
        Err(ServiceError::McpError(error)) => assert!(matches!(
            error.code,
            ErrorCode::INVALID_PARAMS | ErrorCode::INVALID_REQUEST
        )),
        _ => panic!("expected a closed-schema MCP rejection"),
    }
}

pub fn initialize() -> Value {
    serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},
        "clientInfo":{"name":"ordinary-agent-test","version":"1"}}})
}
