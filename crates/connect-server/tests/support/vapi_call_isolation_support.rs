//! Protected call setup and independent first-callback MCP client helpers.
#![allow(clippy::unwrap_used)]
use super::{grants, media};
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::{Operation, Outcome, Session};
use connect_store::Store;
use rmcp::{
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde_json::{json, Value};
use std::time::Duration;
pub async fn tools(
    event: &Value,
) -> rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig> {
    let server = &event["body"]["assistantOverrides"]["tools:append"][0]["server"];
    let mut headers = reqwest_mcp::header::HeaderMap::new();
    headers.insert(
        "x-call-id",
        event["resource"]["id"].as_str().unwrap().parse().unwrap(),
    );
    let http = tokio::task::spawn_blocking(move || {
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
pub fn server(event: &Value) -> &Value {
    &event["body"]["assistantOverrides"]["tools:append"][0]["server"]
}
pub fn read(directory: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(directory.join("intent.json")).unwrap()).unwrap()
}
pub async fn session(
    f: &mut media::Fixture,
    client: &mut ConnectClient,
    task: &TaskRecord,
) -> Session {
    client.negotiate_task(&task.id).await.unwrap();
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = client
        .task(grants::wrapped(
            &task.id,
            "second-invite",
            Operation::SessionInvite {
                request_id: "second-invite".into(),
                conversation_id: f.h.cid.clone(),
                participant_id: f.h.peer_id.clone(),
                media: vec!["audio".into()],
            },
        ))
        .await
        .unwrap()
    else {
        panic!("invite")
    };
    let Outcome::Session(session) = *outcome else {
        panic!("session")
    };
    f.h.peer
        .session_accept("second-accept", &f.h.cid, &session.id)
        .await
        .unwrap()
}

pub async fn settled(f: &mut media::Fixture) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let page =
                f.h.owner
                    .context_deliveries(&f.h.cid, None, 10)
                    .await
                    .unwrap();
            if page.deliveries.len() == 2 && page.deliveries.iter().all(|r| r.state == "delivered")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

pub async fn cross_denials(
    f: &media::Fixture,
    first_post: &Value,
    second_post: &Value,
    directories: [&std::path::Path; 2],
) {
    let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let states = [read(directories[0]), read(directories[1])];
    let http = super::call_setup::http().await;
    // Foreign setup secrets and foreign resource IDs independently fail before
    // either first callback is allowed to establish a task binding.
    for (target, credential, origin) in [
        (first_post, second_post, first_post),
        (first_post, first_post, second_post),
        (second_post, first_post, second_post),
        (second_post, second_post, first_post),
    ] {
        let result = http
            .post(server(target)["url"].as_str().unwrap())
            .header(
                "Authorization",
                server(credential)["headers"]["Authorization"]
                    .as_str()
                    .unwrap(),
            )
            .header("x-call-id", origin["resource"]["id"].as_str().unwrap())
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(result.status(), 401);
        let body = result.text().await.unwrap();
        for source in [first_post, second_post] {
            let secret = server(source)["headers"]["Authorization"]
                .as_str()
                .unwrap()
                .strip_prefix("Bearer ")
                .unwrap();
            assert!(!body.contains(secret));
        }
        assert!(!body.contains(&f.h.owner_config.credential));
    }
    assert_eq!(
        Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    assert_eq!([read(directories[0]), read(directories[1])], states);
}
