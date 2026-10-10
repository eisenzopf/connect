//! Independent MCP client and raw HTTP authority/transport boundaries.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_mcp::{FixedBindingAuth, RunningMcp};
use connect_protocol::tasks::{TaskCommand, TaskRequest};
use fixture::Fixture;
use rmcp::{
    model::{CallToolRequestParams, ClientConfig, ProtocolVersion},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_mcp_client_negotiates_discovers_and_uses_only_its_bound_task() {
    let f = Fixture::start().await;
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        Arc::new(FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap()),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let url = format!("http://{}/mcp", listener.address);
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    for version in [
        ProtocolVersion::V_2024_11_05,
        ProtocolVersion::V_2025_03_26,
        ProtocolVersion::V_2025_06_18,
        ProtocolVersion::V_2025_11_25,
        ProtocolVersion::V_2026_07_28,
    ] {
        let expected = if version == ProtocolVersion::V_2026_07_28 {
            ProtocolVersion::V_2025_11_25
        } else {
            version.clone()
        };
        let transport = StreamableHttpClientTransport::with_client(
            http.clone(),
            StreamableHttpClientTransportConfig::with_uri(url.clone())
                .auth_header(f.issued.expose_secret()),
        );
        let client = tokio::time::timeout(
            Duration::from_secs(20),
            ClientConfig::default()
                .with_protocol_version(version)
                .serve(transport),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(client.peer_info().unwrap().protocol_version, expected);
        let tools = client.list_tools(None).await.unwrap();
        assert_eq!(
            tools
                .tools
                .iter()
                .map(|tool| tool.name.as_ref())
                .collect::<Vec<_>>(),
            ["connect_task_status", "connect_recall", "connect_history"]
        );
        assert_eq!(tools.tools[0].name, "connect_task_status");
        assert_eq!(tools.tools[0].input_schema["additionalProperties"], false);
        let status = client
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap();
        assert_ne!(status.is_error, Some(true));
        let data = status.structured_content.unwrap();
        assert_eq!(data["kind"], "record");
        assert_eq!(data["data"]["id"], f.task.id);
        assert_eq!(data["data"]["conversation_id"], f.task.conversation_id);
        assert!(!data.to_string().contains(f.issued.expose_secret()));
        for arguments in [
            json!({"task_id":"different-task"}),
            json!({"owner_id":"different-owner"}),
            json!({"binding":{"external_id":"forged"}}),
        ] {
            assert!(client
                .call_tool(
                    CallToolRequestParams::new("connect_task_status")
                        .with_arguments(arguments.as_object().unwrap().clone())
                )
                .await
                .is_err());
        }
        client.cancel().await.unwrap();
    }
    listener.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn raw_http_requires_current_task_grant_and_rejects_spoofing_and_oversized_bodies() {
    let f = Fixture::start().await;
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        Arc::new(FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap()),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let url = format!("http://{}/mcp", listener.address);
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"raw-independent-client","version":"1"}}});
    let request = || {
        http.post(&url)
            .header("accept", "application/json, text/event-stream")
    };
    for token in ["", "invalid", f.owner.credential.as_str()] {
        let response = request()
            .bearer_auth(token)
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    let granted = || request().bearer_auth(f.issued.expose_secret());
    for (header, value) in [
        ("origin", "https://unrelated.invalid"),
        ("host", "unrelated.invalid"),
        ("x-call-id", "forged-call"),
        ("x-chat-id", "forged-chat"),
    ] {
        let response = granted()
            .header(header, value)
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_client_error(),
            "spoofed {header} accepted"
        );
    }
    let response = granted().json(&initialize).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let result: serde_json::Value = response.json().await.unwrap();
    assert_eq!(result["result"]["protocolVersion"], "2025-11-25");
    let response = granted()
        .header("content-type", "application/json")
        .body("x".repeat(512 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    let stream = futures_util::stream::iter([
        Ok::<_, std::io::Error>(vec![b' '; 256 * 1024]),
        Ok(vec![b' '; 256 * 1024 + 1]),
    ]);
    let response = granted()
        .header("content-type", "application/json")
        .body(reqwest_mcp::Body::wrap_stream(stream))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        413,
        "chunked body bypassed request bound"
    );
    let response = granted()
        .header("content-type", "application/json")
        .body("{bad-json")
        .send()
        .await
        .unwrap();
    // rmcp 3.5.1 expect_json explicitly returns 415 for invalid JSON.
    assert_eq!(response.status(), 415);
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    let response = granted().json(&initialize).send().await.unwrap();
    assert_eq!(
        response.status(),
        401,
        "revoked grant could initialize again"
    );
    listener.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_denies_existing_mcp_client_and_nonloopback_binding_is_refused() {
    let f = Fixture::start().await;
    let auth =
        Arc::new(FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap());
    assert!(RunningMcp::start_local(
        f.server.service.clone(),
        auth.clone(),
        "0.0.0.0:0".parse().unwrap()
    )
    .await
    .is_err());
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        auth,
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", listener.address))
            .auth_header(f.issued.expose_secret()),
    );
    let client = ClientConfig::default()
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
        .serve(transport)
        .await
        .unwrap();
    client.list_tools(None).await.unwrap();
    f.server
        .service
        .execute_task(
            f.owner.credential_id.clone(),
            TaskRequest::new(TaskCommand::Cancel {
                request_id: "cancel-mcp-task".into(),
                task_id: f.task.id.clone(),
                reason: Some("cancel test".into()),
            }),
        )
        .await
        .unwrap();
    assert!(client.list_tools(None).await.is_err());
    assert!(client
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .is_err());
    let _ = client.cancel().await;
    listener.shutdown().await.unwrap();
    f.close().await;
}
