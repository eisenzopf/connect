//! Fake private API with real SDK lookup; no provider account or model operation.
#![allow(clippy::unwrap_used, dead_code)]
use super::fixture::Fixture;
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_server::vapi_task_setup::VapiGrantSetup;
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

pub fn resource(kind: OriginKind) -> Value {
    let mut value = json!({"id":"resource-one","assistantId":"assistant-one"});
    match kind {
        OriginKind::Call => value["status"] = json!("in-progress"),
        OriginKind::ChatSession => value["status"] = json!("active"),
        OriginKind::Chat => {}
    }
    value
}
pub async fn private_api(
    kind: OriginKind,
    response: Value,
    status: u16,
    expected: u64,
) -> (MockServer, Vapi) {
    let server = MockServer::start().await;
    let name = match kind {
        OriginKind::Call => "call",
        OriginKind::Chat => "chat",
        OriginKind::ChatSession => "session",
    };
    Mock::given(method("GET"))
        .and(path(format!("/{name}/resource-one")))
        .and(header("Authorization", "Bearer private-test-key"))
        .respond_with(ResponseTemplate::new(status).set_body_json(response))
        .expect(expected)
        .mount(&server)
        .await;
    let client = Vapi::new("private-test-key", &server.uri()).unwrap();
    (server, client)
}
pub fn setup(f: &Fixture, origin: OriginKind) -> VapiGrantSetup {
    VapiGrantSetup {
        credential_id: f.owner.credential_id.clone(),
        request_id: "verified-setup".into(),
        task_id: f.task.id.clone(),
        origin,
        external_id: "resource-one".into(),
        assistant_id: "assistant-one".into(),
        operations: f.task.scope.operations.clone(),
        expires_at: f.task.expires_at.min(chrono::Utc::now().timestamp() + 180),
    }
}
