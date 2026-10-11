//! The independent app's attributed business result is readable through real MCP.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/connected_app.rs"]
mod support;
use connect_client::{tasks::*, Operation};
use rmcp::{
    model::{CallToolRequestParams, ClientConfig},
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::json;
use std::{process::Stdio, time::Duration};
use support::Fixture;
use tokio::process::Command;

#[tokio::test]
async fn business_result_is_source_backed_through_sdk_and_grant_bound_mcp() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None);
    let question = f.question().await;
    let command=TaskCommand::Reply {request_id:"mcp-review".into(),conversation_id:question.conversation_id.clone(),reply_to:question.id.clone(),body:json!({"action":"review_decision","draft_id":"draft-002","decision":"approved","note":"Publish it."}).to_string()};
    f.request(
        "agent",
        Operation::Task {
            request: Box::new(TaskRequest::new(command)),
        },
    )
    .await;
    let stats = f.settled(1, 0).await;
    let result = &stats.results[0];
    assert_eq!(result.status, "approved");
    let history = f.history("agent", &question.conversation_id).await;
    let message = history
        .iter()
        .find(|m| m.body.contains("\"review_recorded\""))
        .unwrap();
    let request = TaskAccessRequest::new(TaskAccessCommand::Issue {
        request_id: "app-observer-grant".into(),
        definition: GrantDefinition {
            task_id: result.task_id.clone(),
            binding: InvocationBinding {
                kind: InvocationKind::Native,
                adapter: "connected-app-observer".into(),
                external_id: "local-result-reader".into(),
            },
            operations: vec![TaskPermission::History],
            expires_at: chrono::Utc::now().timestamp() + 120,
        },
    });
    let input = f.directory.join("reader-request.json");
    let output = f.directory.join("reader-grant.json");
    connect_server::cli::init::private_write(&input, &serde_json::to_vec(&request).unwrap())
        .unwrap();
    let status = Command::new(&f.binary)
        .args(["task-access", "issue", "--config"])
        .arg(f.directory.join("reviews.toml"))
        .args(["--http-address", &f.http_address.to_string(), "--input"])
        .arg(input)
        .arg("--output")
        .arg(&output)
        .stdout(Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let file: connect_server::cli::task_access::GrantFile =
        serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    let grant = file.grant.unwrap();
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let client = ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(format!("http://{}/mcp", f.mcp_address))
                .auth_header(grant.expose_secret()),
        ))
        .await
        .unwrap();
    let page = client
        .call_tool(
            CallToolRequestParams::new("connect_recall").with_arguments(
                json!({"after":null,"limit":100})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(page.is_error, Some(true));
    let page = page.structured_content.unwrap();
    let sources = page["data"]["sources"].as_array().unwrap();
    let source = sources
        .iter()
        .find(|s| s["source_id"] == message.id)
        .unwrap();
    let status = client
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .unwrap();
    assert_ne!(status.is_error, Some(true));
    let status = status.structured_content.unwrap();
    assert_eq!(status["data"]["conversation_id"], question.conversation_id);
    assert_eq!(status["data"]["id"], result.task_id);
    assert_eq!(source["actor_id"], f.identity("reviews"));
    assert_eq!(source["state"], "available");
    assert_eq!(source["body"], message.body);
    assert!(source["provenance"]
        .as_str()
        .unwrap()
        .starts_with("conversation-journal:"));
    // Developer explicitly installed only a history reader, with no owner token.
    let denied = client
        .call_tool(
            CallToolRequestParams::new("connect_task_cancel").with_arguments(
                json!({"request_id":"reader-cancel","reason":"test"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await;
    assert!(denied.is_err() || denied.unwrap().is_error == Some(true));
    assert_eq!(f.settled(1, 0).await.results[0].task_id, result.task_id);
    client.cancel().await.unwrap();
    f.finish(&mut app).await;
}
