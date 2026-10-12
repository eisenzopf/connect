//! Independent real MCP client follows the provider's installed tool URL.
#![allow(clippy::unwrap_used)]
use rmcp::{
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::Value;
use std::time::Duration;
pub async fn chat(
    body: &Value,
) -> rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig> {
    let http = tokio::task::spawn_blocking(|| {
        let mut headers = reqwest_mcp::header::HeaderMap::new();
        headers.insert("x-chat-id", "installed-chat".parse().unwrap());
        headers.insert("x-call-id", "installed-chat".parse().unwrap());
        headers.insert("x-session-id", "installed-session".parse().unwrap());
        reqwest_mcp::Client::builder()
            .no_proxy()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let server = &body["assistantOverrides"]["tools:append"][0]["server"];
    rmcp::model::ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(server["url"].as_str().unwrap())
                .auth_header(
                    server["headers"]["Authorization"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("Bearer ")
                        .unwrap(),
                ),
        ))
        .await
        .unwrap()
}
