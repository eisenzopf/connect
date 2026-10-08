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
