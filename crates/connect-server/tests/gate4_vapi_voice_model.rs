//! Saved-model lookup cannot stage a call with lost authority or bad provider data.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/paused_vapi_lookup.rs"]
mod paused;
mod support;
use connect_client::{tasks::TaskCommand, ConnectClient};
use connect_connectors::Vapi;
use connect_server::vapi_bootstrap;
use connect_store::Store;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";

fn posted(directory: &Path) -> bool {
    let state: Value =
        serde_json::from_slice(&std::fs::read(directory.join("intent.json")).unwrap()).unwrap();
    state["posted"].as_bool().unwrap()
}

async fn session(f: &mut media::Fixture) -> String {
    call_setup::route(f);
    let (_, config) = f
        .grant("model-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let id = f.session(&mut client, "model-session").await.id;
    client.close().await;
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn failed_or_invalid_saved_model_cannot_stage_dispatch_or_echo_provider_data() {
    let fake = gate3_support::FakeVapi::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    let sid = session(&mut f).await;
    let expected =
        f.h.server
            .store
            .authorize_vapi_task_session_setup(&f.h.owner_config.credential_id, &f.task.id, &sid)
            .unwrap();
    let api = MockServer::start().await;
    let sdk = Arc::new(Vapi::new("private-model-key", &api.uri()).unwrap());
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("model-setup");
    f.h.server
        .prepare_vapi_task_session(&sid, call_setup::definition(&f), &directory, sdk.clone())
        .await
        .unwrap();
    for (status, body, stage) in [
        (
            503,
            json!({"error":"private-provider-content-sentinel"}),
            "model-lookup",
        ),
        (
            200,
            json!({"id":"wrong-assistant","model":{"provider":"openai"}}),
            "assistant",
        ),
        (200, json!({"id":ASSISTANT}), "model-shape"),
        (200, json!({"id":ASSISTANT,"model":{}}), "model-shape"),
        (
            200,
            json!({"id":ASSISTANT,"model":{"provider":"openai","tools":null}}),
            "model-shape",
        ),
    ] {
        let response = Mock::given(method("GET"))
            .and(path(format!("/assistant/{ASSISTANT}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .expect(1)
            .mount_as_scoped(&api)
            .await;
        let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
        let error = vapi_bootstrap::stage_call(
            f.h.server.service.clone(),
            &directory,
            &f.h.owner_config.credential_id,
            &expected,
            &sdk,
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.to_string(), stage);
        assert!(!error
            .to_string()
            .contains("private-provider-content-sentinel"));
        assert!(!posted(&directory));
        assert_eq!(
            Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
            before
        );
        drop(response);
    }
    assert!(api
        .received_requests()
        .await
        .unwrap()
        .iter()
        .all(|r| r.method.as_str() == "GET"));
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_cancellation_during_saved_model_lookup_prevents_dispatch_intent() {
    let fake = gate3_support::FakeVapi::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    let sid = session(&mut f).await;
    let (barrier, sdk) = paused::PausedLookup::with_response(
        &format!("/assistant/{ASSISTANT}"),
        json!({"id":ASSISTANT,"model":{"provider":"openai","tools":[]}}),
    )
    .await;
    let sdk = Arc::new(sdk);
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("cancel-model");
    f.h.server
        .prepare_vapi_task_session(&sid, call_setup::definition(&f), &directory, sdk.clone())
        .await
        .unwrap();
    let expected =
        f.h.server
            .store
            .authorize_vapi_task_session_setup(&f.h.owner_config.credential_id, &f.task.id, &sid)
            .unwrap();
    let service = f.h.server.service.clone();
    let owned_directory = directory.clone();
    let caller = f.h.owner_config.credential_id.clone();
    let staging = tokio::spawn(async move {
        vapi_bootstrap::stage_call(service, &owned_directory, &caller, &expected, &sdk).await
    });
    barrier.wait_for_lookup().await;
    f.h.owner
        .task(TaskCommand::Cancel {
            request_id: "cancel-model-read".into(),
            task_id: f.task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    let after = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    barrier.release();
    assert_eq!(
        staging.await.unwrap().err().unwrap().to_string(),
        "current-authority"
    );
    assert!(!posted(&directory));
    assert_eq!(
        Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        after
    );
    barrier.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_saved_model_lookup_is_bounded_without_staging_a_call() {
    let fake = gate3_support::FakeVapi::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    let sid = session(&mut f).await;
    let api = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/assistant/{ASSISTANT}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(8))
                .set_body_json(json!({"id":ASSISTANT,"model":{"provider":"openai"}})),
        )
        .expect(1)
        .mount(&api)
        .await;
    let sdk = Arc::new(Vapi::new("private-model-key", &api.uri()).unwrap());
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("slow-model");
    f.h.server
        .prepare_vapi_task_session(&sid, call_setup::definition(&f), &directory, sdk.clone())
        .await
        .unwrap();
    let expected =
        f.h.server
            .store
            .authorize_vapi_task_session_setup(&f.h.owner_config.credential_id, &f.task.id, &sid)
            .unwrap();
    let started = tokio::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(7),
        vapi_bootstrap::stage_call(
            f.h.server.service.clone(),
            &directory,
            &f.h.owner_config.credential_id,
            &expected,
            &sdk,
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.err().unwrap().to_string(), "model-deadline");
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(!posted(&directory));
    f.close().await;
}
