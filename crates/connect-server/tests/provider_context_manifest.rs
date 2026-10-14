//! Preserve authorized legacy context through manifest admission and the real Vapi SDK.
#![allow(clippy::unwrap_used)]
#[path = "support/native_tasks.rs"]
mod native_tasks;

use connect_connectors::Vapi;
use connect_protocol::ContextView;
use connect_server::providers::Worker;
use connect_store::{Route, Scope};
use native_tasks::Harness;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";

#[tokio::test(flavor = "multi_thread")]
async fn escaped_ordinary_context_over_128k_reaches_vapi_intact_with_source_provenance() {
    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/assistant/{ASSISTANT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": ASSISTANT,
            "model": {"messages": [{"role": "system", "content": "History: {{connect_context}}"}]}
        })))
        .expect(1)
        .mount(&provider)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .and(body_partial_json(
            json!({"input": "Use the attributed history"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "owned-loopback-chat",
            "output": [{"role": "assistant", "content": "Fixture answer"}]
        })))
        .expect(1)
        .mount(&provider)
        .await;
    let adapter = Arc::new(Vapi::new("owned-loopback-key", &provider.uri()).unwrap());
    let mut h = Harness::start().await;
    let body = "\u{1}".repeat(8_000);
    let mut original = Vec::new();
    // Each public send fits the established wire and message limits. Select the
    // route afterward so these are history, rather than oversized provider input.
    for index in 0..3 {
        original.push(
            h.owner
                .send(&format!("escaped-history-{index}"), &h.cid, &body)
                .await
                .unwrap(),
        );
    }
    let delegate = h
        .server
        .store
        .issue(
            &h.peer_id,
            Scope::messaging(vec![h.owner_config.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    h.server
        .store
        .configure_route(
            &h.owner_config.credential_id,
            &Route {
                id: "escaped-context-vapi".into(),
                identity_id: h.peer_id.clone(),
                credential_id: delegate.id,
                conversation_id: h.cid.clone(),
                connector: "vapi".into(),
                target: ASSISTANT.into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let worker = Worker::with_drivers(h.server.service.clone(), vec![adapter]).unwrap();
    let question = h
        .owner
        .send(
            "escaped-context-question",
            &h.cid,
            "Use the attributed history",
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let jobs = h.owner.provider_status(&h.cid).await.unwrap();
            if jobs.len() == 1 && jobs[0].state == "completed" {
                break;
            }
            assert!(!jobs.iter().any(|job| job.state == "failed"));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let requests = provider.received_requests().await.unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|request| request.method == "POST")
        .collect();
    assert_eq!(posts.len(), 1);
    let request: Value = posts[0].body_json().unwrap();
    assert_eq!(request["assistantId"], ASSISTANT);
    assert_eq!(request["input"], "Use the attributed history");
    assert!(request.get("previousChatId").is_none());
    let context = request["assistantOverrides"]["variableValues"]["connect_context"]
        .as_str()
        .unwrap();
    let (_, history) = context.split_once("History JSON: ").unwrap();
    assert!(history.len() > 128_000);
    let selected: ContextView = serde_json::from_str(history).unwrap();
    assert_eq!(selected.conversation_id, h.cid);
    for receipt in &original {
        let source = selected
            .items
            .iter()
            .find(|item| item.source_id == receipt.message_id)
            .unwrap();
        assert_eq!(source.body, body);
        assert_eq!(source.actor_id, h.owner_config.identity_id);
        assert!(source.sequence > 0 && !source.kind.is_empty());
    }
    assert!(selected
        .items
        .iter()
        .any(|item| item.source_id == question.message_id));
    assert!(selected
        .items
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert!(!request.to_string().contains("owned-loopback-key"));
    assert_eq!(
        h.owner
            .send(
                "escaped-context-question",
                &h.cid,
                "Use the attributed history"
            )
            .await
            .unwrap()
            .message_id,
        question.message_id
    );
    worker.shutdown().await.unwrap();
    assert_eq!(
        provider
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    h.close().await;
}
