//! Real HTTP proof of binding, expiry and backing-credential revocation.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_mcp::{FixedBindingAuth, RunningMcp};
use connect_protocol::tasks::{GrantDefinition, InvocationKind, TaskPermission};
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_binding_duplicate_authorization_and_revoked_parent_are_denied() {
    let f = fixture::Fixture::start().await;
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        Arc::new(FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap()),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let http = client().await;
    let mut other = f.binding.clone();
    other.external_id = "different-installed-host".into();
    f.server
        .service
        .register_task_binding(
            f.owner.credential_id.clone(),
            f.task.id.clone(),
            other.clone(),
        )
        .await
        .unwrap();
    let issued = f
        .server
        .service
        .issue_task_grant(
            f.owner.credential_id.clone(),
            "wrong-binding".into(),
            GrantDefinition {
                task_id: f.task.id.clone(),
                binding: other,
                operations: vec![TaskPermission::History],
                expires_at: chrono::Utc::now().timestamp() + 180,
            },
        )
        .await
        .unwrap();
    let request = || {
        http.post(format!("http://{}/mcp", listener.address))
            .header("accept", "application/json, text/event-stream")
            .json(&initialize())
    };
    assert_eq!(
        request()
            .bearer_auth(issued.expose_secret())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        request()
            .header(
                "authorization",
                format!("Bearer {}", f.issued.expose_secret())
            )
            .header(
                "authorization",
                format!("Bearer {}", issued.expose_secret())
            )
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let response = request()
        .bearer_auth(f.issued.expose_secret())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(!body.contains(f.issued.expose_secret()));
    assert!(!body.contains(&f.owner.credential));
    f.revoke_parent();
    assert_eq!(
        request()
            .bearer_auth(f.issued.expose_secret())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let mut provider = f.binding.clone();
    provider.kind = InvocationKind::ProviderCall;
    assert!(FixedBindingAuth::new(f.server.service.clone(), provider).is_err());
    listener.shutdown().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_bearer_cannot_reinitialize_or_discover() {
    let f = fixture::Fixture::start_configured(true).await;
    let http = client().await;
    let expires = chrono::Utc::now().timestamp() + 2;
    let issued = f
        .server
        .service
        .issue_task_grant(
            f.owner.credential_id.clone(),
            "short-lived".into(),
            GrantDefinition {
                task_id: f.task.id.clone(),
                binding: f.binding.clone(),
                operations: vec![TaskPermission::History],
                expires_at: expires,
            },
        )
        .await
        .unwrap();
    let request = || {
        http.post(format!("http://{}/mcp", f.server.mcp_address.unwrap()))
            .header("accept", "application/json, text/event-stream")
            .bearer_auth(issued.expose_secret())
    };
    assert_eq!(
        request().json(&initialize()).send().await.unwrap().status(),
        200
    );
    while chrono::Utc::now().timestamp() < expires {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    for body in [
        initialize(),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    ] {
        assert_eq!(request().json(&body).send().await.unwrap().status(), 401);
    }
    f.close().await;
}

async fn client() -> reqwest_mcp::Client {
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
fn initialize() -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"authority-test","version":"1"}}})
}
