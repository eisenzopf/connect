//! Protected recovery uses the saved intent and never repeats provider creation.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_bootstrap.rs"]
mod bootstrap;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/paused_vapi_lookup.rs"]
mod paused;
#[path = "support/vapi_bindings.rs"]
mod provider;
use connect_client::{tasks::*, ConnectClient};
use connect_mcp::VapiOrigin;
use connect_server::vapi_bootstrap::{create_session, reconcile_created};
use connect_store::Store;
use serde_json::json;
use std::time::Duration;
use wiremock::{
    matchers::{method, path},
    Mock, ResponseTemplate,
};

#[tokio::test]
async fn lost_session_receipt_recovers_exact_resource_and_first_mcp_effect_once() {
    let f = fixture::Fixture::start_permitted(
        false,
        vec![TaskPermission::History, TaskPermission::Send],
    )
    .await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    setup.uncertain(&f).await;
    let original = bootstrap::read(&setup.directory);
    for _ in 0..2 {
        assert_eq!(
            reconcile_created(
                f.server.service.clone(),
                &setup.provider,
                &setup.directory,
                "session-one"
            )
            .await
            .unwrap(),
            "session-one"
        );
    }
    let recovered = bootstrap::read(&setup.directory);
    assert_eq!(recovered["external_id"], "session-one");
    for field in ["nonce", "secret", "definition", "mcp_url", "posted"] {
        assert_eq!(recovered[field], original[field]);
    }
    assert_eq!(
        create_session(f.server.service.clone(), &setup.provider, &setup.directory)
            .await
            .unwrap(),
        "session-one"
    );
    let http = provider::http(provider::headers(VapiOrigin::ChatSession, "session-one")).await;
    let body = provider::mutation("recovered-session-send");
    let mut first = None;
    for _ in 0..2 {
        let response = http
            .post(format!("http://{}/mcp", setup.endpoint.address))
            .header("Accept", "application/json, text/event-stream")
            .bearer_auth(setup.secret())
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let result: serde_json::Value = response.json().await.unwrap();
        assert_ne!(result["result"]["isError"], true);
        assert_eq!(
            result["result"]["structuredContent"]["data"]["task_id"],
            f.task.id
        );
        if let Some(saved) = &first {
            assert_eq!(&result, saved);
        } else {
            first = Some(result);
        }
    }
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    assert_eq!(
        peer.history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    peer.close().await;
    assert_eq!(
        setup
            .api
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
    setup.close().await;
    f.close().await;
}

#[tokio::test]
async fn unstaged_invalid_nonce_inactive_and_conflicting_resource_cannot_recover() {
    let f = fixture::Fixture::start().await;
    let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let original = bootstrap::read(&setup.directory);
    assert!(reconcile_created(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory,
        "session-one"
    )
    .await
    .is_err());
    assert!(setup.api.received_requests().await.unwrap().is_empty());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    assert_eq!(bootstrap::read(&setup.directory), original);
    setup.uncertain(&f).await;
    let original = bootstrap::read(&setup.directory);
    for (id, nonce, status) in [
        ("wrong-nonce", uuid::Uuid::new_v4().to_string(), "active"),
        (
            "ended-session",
            original["nonce"].as_str().unwrap().into(),
            "completed",
        ),
        (
            "other-session",
            original["nonce"].as_str().unwrap().into(),
            "active",
        ),
    ] {
        Mock::given(method("GET")).and(path(format!("/session/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":id,"assistantId":"assistant-one","status":status,"metadata":{"variableValues":{"connect_invocation_id":nonce}}})))
            .mount(&setup.api).await;
    }
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    for id in ["wrong-nonce", "ended-session", "unavailable", "../invalid"] {
        assert!(reconcile_created(
            f.server.service.clone(),
            &setup.provider,
            &setup.directory,
            id
        )
        .await
        .is_err());
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        assert_eq!(bootstrap::read(&setup.directory), original);
    }
    reconcile_created(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory,
        "session-one",
    )
    .await
    .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let frozen = bootstrap::read(&setup.directory);
    let requests = setup.api.received_requests().await.unwrap().len();
    assert!(reconcile_created(
        f.server.service.clone(),
        &setup.provider,
        &setup.directory,
        "other-session"
    )
    .await
    .is_err());
    assert_eq!(setup.api.received_requests().await.unwrap().len(), requests);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    assert_eq!(bootstrap::read(&setup.directory), frozen);
    setup.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_authority_after_paused_recovery_lookup_denies_issuance() {
    for cancel in [false, true] {
        let f = fixture::Fixture::start().await;
        let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
        setup.uncertain(&f).await;
        let (barrier,sdk)=paused::PausedLookup::with_response("/session/session-one",json!({"id":"session-one","assistantId":"assistant-one","status":"active","metadata":{"variableValues":{"connect_invocation_id":setup.state["nonce"]}}})).await;
        let (service, directory) = (f.server.service.clone(), setup.directory.clone());
        let recovering = tokio::spawn(async move {
            reconcile_created(service, &sdk, &directory, "session-one").await
        });
        barrier.wait_for_lookup().await;
        if cancel {
            let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
            owner
                .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
                .await
                .unwrap();
            owner
                .task(TaskCommand::Cancel {
                    request_id: "cancel-recovery".into(),
                    task_id: f.task.id.clone(),
                    reason: None,
                })
                .await
                .unwrap();
            owner.close().await;
        } else {
            f.revoke_parent();
        }
        let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
        assert!(!recovering.is_finished());
        barrier.release();
        assert!(recovering.await.unwrap().is_err());
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        // Retaining a proved observation does not register or authorize it.
        assert_eq!(
            bootstrap::read(&setup.directory)["external_id"],
            "session-one"
        );
        barrier.close().await;
        setup.close().await;
        f.close().await;
    }
}
