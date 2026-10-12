#![allow(clippy::unwrap_used)]
use connect_connector_api::DeliveryRequest;
use connect_connectors::{Connector, Failure, Telnyx, Vapi};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};
fn job(name: &str) -> DeliveryRequest {
    DeliveryRequest {
        target: if name == "vapi" {
            "12345678-1234-1234-1234-123456789012"
        } else {
            "+15555550101"
        }
        .into(),
        source: Some("+15555550102".into()),
        body: "hello".into(),
        previous_chat_id: Some("previous-chat".into()),
        context: None,
    }
}
#[tokio::test]
async fn telnyx_uses_users_sdk_and_submits_only_once_on_server_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(body_partial_json(
            serde_json::json!({"to":"+15555550101","from":"+15555550102","text":"hello"}),
        ))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let client = Telnyx::new("test-secret", &server.uri()).unwrap();
    assert_eq!(
        client.deliver(&job("telnyx")).await.err(),
        Some(Failure::Uncertain)
    );
}
#[tokio::test]
async fn vapi_preserves_context_and_never_repeats_uncertain_mutations() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat")).and(body_partial_json(serde_json::json!({"previousChatId":"previous-chat","input":"hello"}))).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"chat-2","output":[{"role":"assistant","content":"hello human"}]}))).expect(1).mount(&server).await;
    let client = Vapi::new("test-secret", &server.uri()).unwrap();
    let done = client.deliver(&job("vapi")).await.unwrap();
    assert_eq!(done.provider_id, "chat-2");
    assert_eq!(done.reply.as_deref(), Some("hello human"));
    server.reset().await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        client.deliver(&job("vapi")).await.err(),
        Some(Failure::Uncertain)
    );
}
#[tokio::test]
async fn definite_rejection_differs_from_unknown_remote_effect() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({"errors":[]})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        Telnyx::new("test-secret", &server.uri())
            .unwrap()
            .deliver(&job("telnyx"))
            .await
            .err(),
        Some(Failure::Rejected)
    );
}

#[tokio::test]
async fn vapi_chat_receives_authorized_history_separate_from_the_current_input() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"12345678-1234-1234-1234-123456789012",
            "model":{"messages":[{"role":"system","content":"History: {{connect_context}}"}]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"chat-with-voice-context", "output":[{"role":"assistant","content":"orchard violet"}]
        })))
        .expect(1).mount(&server).await;
    let mut request = job("vapi");
    request.context =
        Some("{\"items\":[{\"kind\":\"voice_transcript\",\"body\":\"orchard violet\"}]}".into());
    let reply = Vapi::new("test-secret", &server.uri())
        .unwrap()
        .deliver(&request)
        .await
        .unwrap();
    assert_eq!(reply.reply.as_deref(), Some("orchard violet"));
    let calls = server.received_requests().await.unwrap();
    let post = calls
        .iter()
        .find(|request| request.method == "POST")
        .unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&post.body).unwrap();
    assert!(payload.get("previousChatId").is_none());
    let context = payload
        .pointer("/assistantOverrides/variableValues/connect_context")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(context.contains("untrusted data"));
    assert!(context.contains("orchard violet"));
    assert_eq!(payload["input"], "hello");
}

#[tokio::test]
async fn chat_context_is_rejected_without_the_required_assistant_template() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"12345678-1234-1234-1234-123456789012",
            "model":{"messages":[{"role":"system","content":"Ordinary assistant"}]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut request = job("vapi");
    request.context = Some("authorized history".into());
    assert_eq!(
        Vapi::new("test-secret", &server.uri())
            .unwrap()
            .deliver(&request)
            .await
            .err(),
        Some(Failure::Rejected)
    );
    assert!(server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|request| request.method == "GET"));
}
