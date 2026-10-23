//! Independent Python driver submits/reconciles a prepared task chat via public APIs.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_host_api.rs"]
mod api;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/codex_child.rs"]
mod owned_runtime;
#[path = "support/vapi_host_tools.rs"]
mod tools;
use connect_client::tasks::*;
use connect_client::ConnectClient;
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_server::{
    cli::init::private_write,
    vapi_bootstrap::{self, BootstrapAuth, Definition},
};
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};
use std::{
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::process::Command;
use wiremock::{
    matchers::{header, method, path},
    Mock, Request, ResponseTemplate,
};

struct Driver<'a> {
    root: &'a Path,
    directory: &'a Path,
    task: &'a str,
    api: &'a str,
}
impl Driver<'_> {
    async fn run(&self, action: &str, output: &Path, label: &str) -> bool {
        let log = self.directory.join(format!("{label}.log"));
        let mut command = Command::new("python3");
        command.env_clear().env("PYTHONDONTWRITEBYTECODE", "1");
        for key in ["PATH", "HOME", "TMPDIR", "USER", "LANG"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("VAPI_PRIVATE_KEY", "fake-private-key")
            .arg(self.root.join("scripts/live/task-chat.py"))
            .arg(action)
            .arg("--allow-live-provider")
            .arg("--config")
            .arg(self.directory.join("native.toml"))
            .arg("--setup")
            .arg(self.directory.join("setup"))
            .arg("--task-id")
            .arg(self.task)
            .arg("--api-base")
            .arg(self.api)
            .arg("--output")
            .arg(output)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&log).unwrap())
            .stderr(
                std::fs::File::create(self.directory.join(format!("{label}-error.log"))).unwrap(),
            );
        if action == "capture" {
            command
                .arg("--prompt")
                .arg(self.directory.join("prompt.txt"));
        } else {
            command.arg("--chat-id").arg("rehearsal-chat");
        }
        if let Some(runtime) = std::env::var_os("CONNECT_TEST_RUNTIME_ROOT") {
            command.arg("--runtime-root").arg(runtime);
        } else {
            command
                .arg("--executable")
                .arg(env!("CARGO_BIN_EXE_connect"));
        }
        let mut process = owned_runtime::OwnedRuntime::spawn(&mut command).unwrap();
        let status = tokio::time::timeout(Duration::from_secs(120), process.wait())
            .await
            .unwrap()
            .unwrap();
        let text = std::fs::read_to_string(log).unwrap();
        assert!(!text.contains("fake-private-key"));
        status.success()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prepared_chat_uses_actual_task_sdk_and_preserves_one_provider_turn() {
    journey(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncertain_chat_uses_get_only_recovery_and_cancelled_task_cannot_call_provider() {
    journey(true).await;
}

async fn journey(lose_receipt: bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    std::fs::create_dir_all(root.join(".local")).unwrap();
    let directory = tempfile::Builder::new()
        .prefix("task-chat-rehearsal-")
        .tempdir_in(root.join(".local"))
        .unwrap()
        .keep();
    eprintln!("Private chat rehearsal evidence: {}", directory.display());
    let mut h = native_tasks::Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = h.create("chat-rehearsal").await;
    h.owner
        .send("prior-source", &h.cid, "TASK-CHAT-PRIOR-SOURCE")
        .await
        .unwrap();
    let grant = grants::issue(
        &h,
        &task,
        "chat-driver",
        vec![TaskPermission::History],
        chrono::Utc::now().timestamp() + 300,
    )
    .await;
    let config = h.owner_config.clone().with_task_grant(&grant).unwrap();
    private_write(
        &directory.join("native.toml"),
        toml::to_string(&config).unwrap().as_bytes(),
    )
    .unwrap();
    private_write(
        &directory.join("prompt.txt"),
        b"Recall the earlier decision through Connect and explain it.",
    )
    .unwrap();
    let api = api::Api::start(200).await;
    let base = api.server.uri();
    let url = base.clone();
    let provider =
        tokio::task::spawn_blocking(move || Arc::new(Vapi::new("fake-private-key", &url).unwrap()))
            .await
            .unwrap();
    let setup = directory.join("setup");
    // This isolated fixture owns one auxiliary endpoint; it is not a capacity test.
    let endpoint = connect_mcp::RunningMcp::start_local_auxiliary(
        h.server.service.clone(),
        Arc::new(BootstrapAuth::new(
            h.server.service.clone(),
            provider.clone(),
            setup.clone(),
        )),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    vapi_bootstrap::prepare(
        h.server.service.clone(),
        Definition {
            credential_id: h.owner_config.credential_id.clone(),
            request_id: "chat-setup".into(),
            task_id: task.id.clone(),
            assistant_id: "assistant-one".into(),
            origin: OriginKind::ChatSession,
            operations: vec![TaskPermission::History, TaskPermission::Send],
            expires_at: chrono::Utc::now().timestamp() + 300,
        },
        &format!("http://{}/mcp", endpoint.address),
        &setup,
    )
    .await
    .unwrap();
    assert_eq!(
        vapi_bootstrap::create_session(h.server.service.clone(), &provider, &setup)
            .await
            .unwrap(),
        "installed-session"
    );
    // Actual independent MCP proves this invocation's context is the task's context.
    let tool = tools::chat(&api.body()).await;
    let sent = tool
        .call_tool(
            CallToolRequestParams::new("connect_send").with_arguments(
                json!({
                    "request_id":"mcp-source", "step_id":"prior-fact", "message": connect_protocol::messages::MessageDraft::text_all(("TASK-CHAT-MCP-SOURCE").to_string())})
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(sent.is_error, Some(true));
    tool.cancel().await.unwrap();
    let receipt = Arc::new(Mutex::new(Value::Null));
    let saved = receipt.clone();
    Mock::given(method("POST")).and(path("/chat"))
        .and(header("Authorization", "Bearer fake-private-key"))
        .respond_with(move |request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["sessionId"], "installed-session");
            assert!(body.get("assistantId").is_none());
            assert_eq!(body["stream"], false);
            let value = json!({"id":"rehearsal-chat", "sessionId":body["sessionId"],
                "name":body["name"], "input":body["input"],
                "output":[{"role":"assistant","content":"Fixture response; not model inference."}]});
            *saved.lock().unwrap() = value.clone();
            ResponseTemplate::new(if lose_receipt { 500 } else { 200 }).set_body_json(value)
        }).mount(&api.server).await;
    let saved = receipt.clone();
    Mock::given(method("GET"))
        .and(path("/chat/rehearsal-chat"))
        .and(header("Authorization", "Bearer fake-private-key"))
        .respond_with(move |_: &Request| {
            ResponseTemplate::new(200).set_body_json(saved.lock().unwrap().clone())
        })
        .mount(&api.server)
        .await;
    let driver = Driver {
        root: &root,
        directory: &directory,
        task: &task.id,
        api: &base,
    };
    // The independent driver may outlast QUIC's idle lifetime. Its later
    // cancellation is a new owner action, using the retained task and a fresh
    // authenticated connection rather than relying on an idle fixture client.
    h.owner.close().await;
    let output = directory.join("capture");
    assert_eq!(driver.run("capture", &output, "first").await, !lose_receipt);
    let original: Value =
        serde_json::from_slice(&std::fs::read(output.join("intent.json")).unwrap()).unwrap();
    if lose_receipt {
        assert_eq!(original["state"], "dispatched");
        assert!(driver.run("reconcile", &output, "recover").await);
    }
    let final_intent: Value =
        serde_json::from_slice(&std::fs::read(output.join("intent.json")).unwrap()).unwrap();
    assert_eq!(final_intent["state"], "captured");
    assert_eq!(final_intent["name"], original["name"]);
    assert_eq!(final_intent["chat_id"], "rehearsal-chat");
    let after = std::fs::read_dir(&output)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("after.json"))
        .find(|p| p.exists())
        .unwrap();
    let context = std::fs::read_to_string(after).unwrap();
    assert!(context.contains("TASK-CHAT-PRIOR-SOURCE"));
    assert!(context.contains("TASK-CHAT-MCP-SOURCE"));
    assert!(!driver.run("capture", &output, "repeat-denied").await);
    let requests = api.server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method == "POST" && r.url.path() == "/chat")
            .count(),
        1
    );
    assert_eq!(api.count(), 1);
    h.owner = ConnectClient::open(h.owner_config.clone()).await.unwrap();
    h.owner.negotiate_task(&task.id).await.unwrap();
    h.owner
        .task(TaskCommand::Cancel {
            request_id: "cancel-driver-task".into(),
            task_id: task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    assert!(
        !driver
            .run("capture", &directory.join("cancelled"), "cancelled")
            .await
    );
    assert_eq!(
        api.server.received_requests().await.unwrap().len(),
        requests.len()
    );
    endpoint.shutdown().await.unwrap();
    h.owner.close().await;
    h.peer.close().await;
    h.server.shutdown().await.unwrap();
}
