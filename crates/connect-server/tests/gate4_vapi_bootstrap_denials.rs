//! Callback metadata is correlation, never proof or permission to mint anew.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_bootstrap.rs"]
mod bootstrap;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/vapi_bindings.rs"]
mod provider;
use connect_mcp::VapiOrigin;
use connect_protocol::tasks::{GrantDefinition, InvocationBinding, InvocationKind};
use connect_store::Store;
use serde_json::json;
use std::time::Duration;
use wiremock::{
    matchers::{method, path},
    Mock, ResponseTemplate,
};

async fn admit(setup: &bootstrap::Setup, headers: reqwest_mcp::header::HeaderMap) -> u16 {
    // Default-header application keeps only one value per name. Put duplicate
    // denial fixtures directly on the request so they reach the actual adapter.
    let http = provider::http(reqwest_mcp::header::HeaderMap::new()).await;
    http.post(format!("http://{}/mcp", setup.endpoint.address))
        .headers(headers)
        .header("Accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"bootstrap-denial-fixture","version":"1"}}}))
        .send().await.unwrap().status().as_u16()
}
fn headers(setup: &bootstrap::Setup, id: &str) -> reqwest_mcp::header::HeaderMap {
    let mut headers = provider::headers(VapiOrigin::ChatSession, id);
    headers.insert(
        "authorization",
        format!("Bearer {}", setup.secret()).parse().unwrap(),
    );
    headers
}
async fn resource(setup: &bootstrap::Setup, id: &str, nonce: &str, status: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/session/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":id,"assistantId":"assistant-one","status":status,"metadata":{"variableValues":{"connect_invocation_id":nonce}}})))
        .with_priority(1)
        .mount(&setup.api).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn malformed_or_foreign_headers_deny_before_private_lookup_or_mutation() {
    let f = fixture::Fixture::start().await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    setup.uncertain(&f).await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let state = bootstrap::read(&setup.directory);
    for case in 0..9 {
        let mut h = headers(&setup, "session-one");
        match case {
            0 => {
                h.remove("authorization");
            }
            1 => {
                h.insert(
                    "authorization",
                    format!("Bearer {}", "a".repeat(64)).parse().unwrap(),
                );
            }
            2 => {
                h.append(
                    "authorization",
                    format!("Bearer {}", setup.secret()).parse().unwrap(),
                );
            }
            3 => {
                h.append("x-session-id", "session-one".parse().unwrap());
            }
            4 => {
                h.remove("x-session-id");
            }
            5 => {
                h.remove("x-chat-id");
            }
            6 => {
                h.insert("x-call-id", "call-one".parse().unwrap());
            }
            7 => {
                h.insert("x-session-id", "../other".parse().unwrap());
            }
            _ => {
                h.append("x-chat-id", "other-chat".parse().unwrap());
            }
        }
        assert!((400..500).contains(&admit(&setup, h).await), "case {case}");
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        assert_eq!(bootstrap::read(&setup.directory), state);
    }
    assert_eq!(setup.api.received_requests().await.unwrap().len(), 1);
    setup.close().await;
    f.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_nonce_inactive_resource_frozen_binding_and_revoked_grant_cannot_issue() {
    let f = fixture::Fixture::start().await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    setup.uncertain(&f).await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let state = bootstrap::read(&setup.directory);
    resource(
        &setup,
        "wrong-nonce",
        &uuid::Uuid::new_v4().to_string(),
        "active",
    )
    .await;
    resource(
        &setup,
        "ended-session",
        state["nonce"].as_str().unwrap(),
        "completed",
    )
    .await;
    for id in ["wrong-nonce", "ended-session", "unavailable"] {
        assert!((400..500).contains(&admit(&setup, headers(&setup, id)).await));
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        assert_eq!(bootstrap::read(&setup.directory), state);
    }
    assert_eq!(admit(&setup, headers(&setup, "session-one")).await, 200);
    let state = bootstrap::read(&setup.directory);
    resource(
        &setup,
        "other-session",
        state["nonce"].as_str().unwrap(),
        "active",
    )
    .await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let count = setup.api.received_requests().await.unwrap().len();
    assert!((400..500).contains(&admit(&setup, headers(&setup, "other-session")).await));
    assert_eq!(setup.api.received_requests().await.unwrap().len(), count);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    assert_eq!(bootstrap::read(&setup.directory), state);
    // Protected lookup of the original issuance receipt, then explicit revoke.
    let issued = f
        .server
        .service
        .issue_verified_provider_task_grant(
            f.owner.credential_id.clone(),
            state["definition"]["request_id"].as_str().unwrap().into(),
            GrantDefinition {
                task_id: f.task.id.clone(),
                binding: InvocationBinding {
                    kind: InvocationKind::ProviderChat,
                    adapter: "vapi-chat-session".into(),
                    external_id: "session-one".into(),
                },
                operations: f.task.scope.operations.clone(),
                expires_at: state["definition"]["expires_at"].as_i64().unwrap(),
            },
        )
        .await
        .unwrap();
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), issued.grant.id.clone())
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!((400..500).contains(&admit(&setup, headers(&setup, "session-one")).await));
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    assert_eq!(bootstrap::read(&setup.directory), state);
    setup.close().await;
    f.close().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_tasks_in_one_installation_have_distinct_setup_authority() {
    let f = fixture::Fixture::start().await;
    let second = provider::second_task(&f).await;
    let one = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    let two = bootstrap::Setup::for_task(&f, &second, "bootstrap-two", 500, Duration::ZERO).await;
    one.uncertain(&f).await;
    two.uncertain(&f).await;
    assert_ne!(one.secret(), two.secret());
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!((400..500).contains(&admit(&two, headers(&one, "session-one")).await));
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let (a, b) = tokio::join!(
        admit(&one, headers(&one, "session-one")),
        admit(&two, headers(&two, &two.resource_id))
    );
    assert_eq!((a, b), (200, 200));
    assert_eq!(
        bootstrap::read(&one.directory)["definition"]["task_id"],
        f.task.id
    );
    assert_eq!(
        bootstrap::read(&two.directory)["definition"]["task_id"],
        second.id
    );
    one.close().await;
    two.close().await;
    f.close().await;
}
