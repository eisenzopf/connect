#![allow(clippy::unwrap_used)]
use connect_connectors::{
    vapi_origin::{OriginKind, OriginVerificationError},
    Vapi,
};
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

async fn fixture(kind: OriginKind, response: Value, status: u16) -> (MockServer, Vapi) {
    let server = MockServer::start().await;
    let resource = match kind {
        OriginKind::Call => "call",
        OriginKind::Chat => "chat",
        OriginKind::ChatSession => "session",
    };
    Mock::given(method("GET"))
        .and(path(format!("/{resource}/resource-one")))
        .and(header("Authorization", "Bearer private-test-key"))
        .respond_with(ResponseTemplate::new(status).set_body_json(response))
        .expect(1)
        .mount(&server)
        .await;
    let client = Vapi::new("private-test-key", &server.uri()).unwrap();
    (server, client)
}
fn resource(kind: OriginKind) -> Value {
    let mut value = json!({"id":"resource-one","assistantId":"assistant-one"});
    if kind == OriginKind::Call {
        value["status"] = json!("in-progress");
    }
    if kind == OriginKind::ChatSession {
        value["status"] = json!("active");
    }
    value
}

#[tokio::test]
async fn private_sdk_verifies_the_exact_assistant_and_resource_for_all_three_modes() {
    for kind in [OriginKind::Call, OriginKind::Chat, OriginKind::ChatSession] {
        let (server, client) = fixture(kind, resource(kind), 200).await;
        let origin = client
            .verify_origin(kind, "resource-one", "assistant-one")
            .await
            .unwrap();
        assert_eq!(origin.kind(), kind);
        assert_eq!(origin.external_id(), "resource-one");
        assert_eq!(origin.assistant_id(), "assistant-one");
        assert!(origin.verified_at() > 0);
        let debug = format!("{origin:?}");
        assert!(!debug.contains("private-test-key"));
        server.verify().await;
    }
}

#[tokio::test]
async fn wrong_missing_or_unrelated_provider_resources_never_produce_verification() {
    for kind in [OriginKind::Call, OriginKind::Chat, OriginKind::ChatSession] {
        for missing in ["id", "assistantId", "other-id", "other-assistant"] {
            let mut response = resource(kind);
            match missing {
                "id" => {
                    response.as_object_mut().unwrap().remove("id");
                }
                "assistantId" => {
                    response.as_object_mut().unwrap().remove("assistantId");
                }
                "other-id" => response["id"] = json!("resource-two"),
                _ => response["assistantId"] = json!("assistant-two"),
            }
            let (server, client) = fixture(kind, response, 200).await;
            assert!(client
                .verify_origin(kind, "resource-one", "assistant-one")
                .await
                .is_err());
            server.verify().await;
        }
    }
}

#[tokio::test]
async fn ended_calls_completed_sessions_and_session_chats_are_not_interchangeable() {
    for (kind, field, value, error) in [
        (
            OriginKind::Call,
            "status",
            "ended",
            OriginVerificationError::Inactive,
        ),
        (
            OriginKind::Call,
            "endedAt",
            "2026-10-10T00:00:00Z",
            OriginVerificationError::Inactive,
        ),
        (
            OriginKind::ChatSession,
            "status",
            "completed",
            OriginVerificationError::Inactive,
        ),
        (
            OriginKind::Chat,
            "sessionId",
            "session-two",
            OriginVerificationError::Mismatch,
        ),
    ] {
        let mut response = resource(kind);
        response[field] = json!(value);
        let (server, client) = fixture(kind, response, 200).await;
        assert_eq!(
            client
                .verify_origin(kind, "resource-one", "assistant-one")
                .await
                .unwrap_err(),
            error
        );
        server.verify().await;
    }
}

#[tokio::test]
async fn failed_private_lookup_has_no_retry_or_error_body_leak() {
    for status in [401, 403, 404, 429, 500] {
        let (server, client) = fixture(
            OriginKind::Call,
            json!({"error":"private-provider-body"}),
            status,
        )
        .await;
        let error = client
            .verify_origin(OriginKind::Call, "resource-one", "assistant-one")
            .await
            .unwrap_err();
        assert_eq!(error, OriginVerificationError::Unavailable);
        assert!(!error.to_string().contains("private-provider-body"));
        server.verify().await;
    }
}

#[tokio::test]
async fn identifiers_cannot_escape_the_intended_private_api_path() {
    let server = MockServer::start().await;
    let client = Vapi::new("private-test-key", &server.uri()).unwrap();
    for id in [
        "",
        "..",
        "a/b",
        "a?secret",
        "a#fragment",
        "a%2fb",
        "a\nb",
        "é",
        &"x".repeat(129),
    ] {
        for (external, assistant) in [(id, "assistant-one"), ("resource-one", id)] {
            assert_eq!(
                client
                    .verify_origin(OriginKind::Call, external, assistant)
                    .await
                    .unwrap_err(),
                OriginVerificationError::InvalidIdentifier
            );
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
