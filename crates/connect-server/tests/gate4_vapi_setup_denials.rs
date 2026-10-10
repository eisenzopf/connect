//! Unauthorized setup never reaches the provider; failed lookup never registers.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/vapi_setup.rs"]
mod setup;
use connect_connectors::vapi_origin::OriginKind;
use connect_protocol::tasks::TaskPermission;
use connect_server::vapi_task_setup;
use connect_store::Store;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn current_preflight_denies_before_private_sdk_lookup() {
    let (api, sdk) =
        setup::private_api(OriginKind::Call, setup::resource(OriginKind::Call), 200, 0).await;
    let f = fixture::Fixture::start().await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let mut request = setup::setup(&f, OriginKind::Call);
    request.operations.push(TaskPermission::Send);
    assert_eq!(
        vapi_task_setup::issue(f.server.service.clone(), &sdk, request)
            .await
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    f.revoke_parent();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(vapi_task_setup::issue(
        f.server.service.clone(),
        &sdk,
        setup::setup(&f, OriginKind::Call)
    )
    .await
    .is_err());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    assert!(api.received_requests().await.unwrap().is_empty());
    api.verify().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_lookup_failure_mismatch_and_inactive_resource_leave_no_binding() {
    for (kind, response, status, code) in [
        (
            OriginKind::Call,
            json!({"error":"private-body"}),
            503,
            "provider-origin-unavailable",
        ),
        (
            OriginKind::Call,
            json!({"id":"resource-one","assistantId":"other","status":"in-progress"}),
            200,
            "provider-origin-denied",
        ),
        (
            OriginKind::Call,
            json!({"id":"resource-one","assistantId":"assistant-one","status":"ended"}),
            200,
            "provider-origin-denied",
        ),
        (
            OriginKind::Chat,
            json!({"id":"resource-one","assistantId":"assistant-one","sessionId":"another-session"}),
            200,
            "provider-origin-denied",
        ),
        (
            OriginKind::ChatSession,
            json!({"id":"resource-one","assistantId":"assistant-one","status":"completed"}),
            200,
            "provider-origin-denied",
        ),
    ] {
        let (api, sdk) = setup::private_api(kind, response, status, 1).await;
        let f = fixture::Fixture::start().await;
        let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
        let error = vapi_task_setup::issue(f.server.service.clone(), &sdk, setup::setup(&f, kind))
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, code);
        assert!(!format!("{error:?}").contains("private-body"));
        assert!(!format!("{error:?}").contains("private-test-key"));
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        api.verify().await;
        f.close().await;
    }
}
