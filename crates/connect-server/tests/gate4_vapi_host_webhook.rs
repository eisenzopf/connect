//! One installed task host owns an independent, signed-callback-only SMS listener.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_host_api.rs"]
mod api;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/vapi_host_process.rs"]
mod process;
#[path = "support/vapi_host_tools.rs"]
mod tools;
use base64::Engine;
use connect_client::tasks::TaskPermission;
use connect_connectors::vapi_origin::OriginKind;
use connect_server::{cli::init::private_write, vapi_bootstrap::Definition};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_task_host_keeps_sms_callbacks_separate_from_control_and_requires_signature() {
    let api = api::Api::start(200).await;
    let mut h = native_tasks::Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = h.create("webhook-owned-task").await;
    let definition = Definition {
        credential_id: h.owner_config.credential_id.clone(),
        request_id: "webhook-vapi-install".into(),
        task_id: task.id,
        assistant_id: "assistant-one".into(),
        origin: OriginKind::ChatSession,
        operations: vec![TaskPermission::Send, TaskPermission::History],
        expires_at: chrono::Utc::now().timestamp() + 120,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bind = listener.local_addr().unwrap();
    drop(listener);
    let state = h.stop_for_host().await;
    private_write(
        &state.directory.join("host-webhook-bind.txt"),
        bind.to_string().as_bytes(),
    )
    .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
    let encoded = base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
    private_write(
        &state.directory.join("host-telnyx-public-key.txt"),
        encoded.as_bytes(),
    )
    .unwrap();
    let callback = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = callback.local_addr().unwrap();
    drop(callback);
    let input = state.directory.join("webhook-host.json");
    private_write(&input,serde_json::to_vec(&json!({"profile":"connect-vapi-task-host/1",
        "definition":definition,"state_directory":"vapi-private","callback_bind":address.to_string()})).unwrap().as_slice()).unwrap();
    let host = process::Process::start(&state, &input, &api.server.uri(), "webhook").await;
    let client = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        client
            .get(format!("http://{bind}/v1/task-access"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        client
            .post(format!("http://{bind}/webhooks/telnyx"))
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(api.count(), 1);
    let mcp = tools::chat(&api.body()).await;
    let catalog = mcp.list_all_tools().await.unwrap();
    assert!(catalog.iter().any(|tool| tool.name == "connect_send"));
    mcp.cancel().await.unwrap();
    host.stop(&[&state.owner.credential]).await;
    assert!(tokio::net::TcpStream::connect(bind).await.is_err());
}
