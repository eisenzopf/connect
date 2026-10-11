//! Protected voice setup and independent MCP client fixtures.
#![allow(clippy::unwrap_used, dead_code)]
use super::media;
use connect_client::tasks::*;
use connect_connectors::vapi_origin::OriginKind;
use connect_server::vapi_bootstrap::Definition;
use connect_store::VoiceRoute;
use rmcp::{
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::Value;
use std::time::Duration;
pub fn definition(f: &media::Fixture) -> Definition {
    Definition {
        credential_id: f.h.owner_config.credential_id.clone(),
        request_id: "voice-bootstrap-grant".into(),
        task_id: f.task.id.clone(),
        assistant_id: "12345678-1234-1234-1234-123456789012".into(),
        origin: OriginKind::Call,
        operations: vec![TaskPermission::History, TaskPermission::Send],
        expires_at: chrono::Utc::now().timestamp() + 120,
    }
}
pub fn route(f: &media::Fixture) {
    f.h.server
        .store
        .configure_voice_route(
            &f.h.owner_config.credential_id,
            &VoiceRoute {
                id: "voice-bootstrap".into(),
                conversation_id: f.h.cid.clone(),
                identity_id: f.h.peer_id.clone(),
                assistant_id: definition(f).assistant_id,
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550101".into()],
                enabled: true,
            },
        )
        .unwrap();
}
pub async fn http() -> reqwest_mcp::Client {
    tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap()
}
pub async fn mcp(
    body: &Value,
) -> rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig> {
    let server = &body["assistantOverrides"]["tools:append"][0]["server"];
    let mut headers = reqwest_mcp::header::HeaderMap::new();
    headers.insert("x-call-id", "call-connect-gate-three".parse().unwrap());
    let client = tokio::task::spawn_blocking(move || {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    rmcp::model::ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            client,
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
