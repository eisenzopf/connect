//! Installed CLI composition retains context, effects and recovery intent.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_host_api.rs"]
mod api;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/vapi_host_process.rs"]
mod process;
#[path = "support/vapi_host_tools.rs"]
mod tools;
use connect_client::ConnectClient;
use connect_server::cli::init::{private_replace, private_write};
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};

async fn setup() -> (native_tasks::StoppedHost, std::path::PathBuf, Value) {
    let mut h = native_tasks::Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = h.create("installed-vapi-task").await;
    h.owner
        .send("older-source", &h.cid, "RETAINED-INSTALLED-CONTEXT")
        .await
        .unwrap();
    let state = h.stop_for_host().await;
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = reservation.local_addr().unwrap();
    drop(reservation);
    let value = json!({"profile":"connect-vapi-task-host/1","state_directory":"vapi-private",
        "callback_bind":bind.to_string(),"definition":{"credential_id":state.owner.credential_id,
        "request_id":"installed-chat-grant","task_id":task.id,"assistant_id":"assistant-one",
        "origin":"chat_session","operations":["send","history"],
        "expires_at":chrono::Utc::now().timestamp()+300}});
    let input = state.directory.join("task-host.json");
    private_write(&input, &serde_json::to_vec(&value).unwrap()).unwrap();
    (state, input, value)
}
fn intent(state: &native_tasks::StoppedHost) -> Value {
    serde_json::from_slice(
        &std::fs::read(state.directory.join("vapi-private/intent.json")).unwrap(),
    )
    .unwrap()
}
fn send() -> CallToolRequestParams {
    CallToolRequestParams::new("connect_send").with_arguments(
        json!({"request_id":"installed-once",
        "step_id":"follow-up","body":"Installed assistant follow-up"})
        .as_object()
        .unwrap()
        .clone(),
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_restart_reuses_one_creation_and_one_visible_mcp_effect() {
    let api = api::Api::start(200).await;
    let (state, input, mut manifest) = setup().await;
    let host = process::Process::start(&state, &input, &api.server.uri(), "first").await;
    assert_eq!(host.ready["provider_resource"], "installed-session");
    let client = tools::chat(&api.body()).await;
    let recall = client
        .call_tool(
            CallToolRequestParams::new("connect_recall")
                .with_arguments(json!({"limit":100}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(recall.is_error, Some(true));
    assert!(recall
        .structured_content
        .unwrap()
        .to_string()
        .contains("RETAINED-INSTALLED-CONTEXT"));
    let first = client.call_tool(send()).await.unwrap();
    assert_ne!(first.is_error, Some(true));
    client.cancel().await.unwrap();
    let original = intent(&state);
    let secret = original["secret"].as_str().unwrap();
    host.stop(&[secret, &state.owner.credential]).await;
    let host = process::Process::start(&state, &input, &api.server.uri(), "restarted").await;
    assert_eq!(api.count(), 1);
    assert_eq!(intent(&state), original);
    let client = tools::chat(&api.body()).await;
    let replay = client.call_tool(send()).await.unwrap();
    assert_eq!(replay.structured_content, first.structured_content);
    let mut owner = ConnectClient::open(state.owner.clone()).await.unwrap();
    let history = owner.history(&state.cid, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| message.body == "Installed assistant follow-up")
            .count(),
        1
    );
    owner.close().await;
    client.cancel().await.unwrap();
    host.stop(&[secret, &state.owner.credential]).await;
    manifest["definition"]["expires_at"] =
        json!(manifest["definition"]["expires_at"].as_i64().unwrap() + 1);
    private_replace(&input, &serde_json::to_vec(&manifest).unwrap()).unwrap();
    process::Process::fails(&state, &input, &api.server.uri(), "changed-expiry").await;
    assert_eq!(api.count(), 1);
    assert_eq!(intent(&state), original);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn issued_task_owner_uses_separate_admin_without_exporting_either_credential() {
    let api = api::Api::start(200).await;
    let (state, input, mut manifest) = setup().await;
    let owner_path = state.directory.join("issued-owner.toml");
    let output = tokio::process::Command::new(process::binary())
        .env_clear()
        .args(["issue", "--config"])
        .arg(state.directory.join("operator.toml"))
        .arg("--server-config")
        .arg(state.directory.join("server.toml"))
        .arg("--identity-id")
        .arg(&state.owner.identity_id)
        .arg("--recipients")
        .arg(&state.peer_id)
        .args(["--voice", "--tasks", "--output"])
        .arg(&owner_path)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "credential issue failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let owner = connect_client::ClientConfig::load(&owner_path).unwrap();
    let operator_path = state.directory.join("operator.toml");
    let operator = connect_client::ClientConfig::load(&operator_path).unwrap();
    let database = state.directory.join("state.sqlite3");
    assert!(connect_store::Store::authorize_recovery(&database, &owner.credential).is_err());
    manifest["definition"]["credential_id"] = json!(owner.credential_id);
    private_replace(&input, &serde_json::to_vec(&manifest).unwrap()).unwrap();
    private_replace(
        &state.directory.join("host-owner.toml"),
        &std::fs::read(&owner_path).unwrap(),
    )
    .unwrap();
    process::Process::fails(&state, &input, &api.server.uri(), "admin-required").await;
    assert_eq!(api.count(), 0);
    assert!(!state.directory.join("vapi-private").exists());
    private_write(
        &state.directory.join("host-operator.toml"),
        &std::fs::read(&operator_path).unwrap(),
    )
    .unwrap();
    let host = process::Process::start(&state, &input, &api.server.uri(), "scoped-owner").await;
    let mut owner_client = ConnectClient::open(owner.clone()).await.unwrap();
    owner_client
        .negotiate_tasks("connect:agent@local", Some(&state.cid))
        .await
        .unwrap();
    let created = owner_client
        .task(connect_client::tasks::TaskCommand::Create {
            request_id: "scoped-owned-task".into(),
            definition: connect_client::tasks::TaskDefinition {
                conversation_id: state.cid.clone(),
                scope: connect_client::tasks::TaskScope {
                    operations: vec![connect_client::tasks::TaskPermission::History],
                    recipients: vec![state.peer_id.clone()],
                    sms_enrollments: Vec::new(),
                },
                expires_at: chrono::Utc::now().timestamp() + 120,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap();
    let connect_client::tasks::TaskOutcome::Record(created) = created else {
        panic!("task creation returned the wrong outcome");
    };
    assert_eq!(created.owner_id, owner.identity_id);
    let mut peer_config =
        connect_client::ClientConfig::load(&state.directory.join("agent.toml")).unwrap();
    peer_config.endpoint = owner.endpoint;
    let mut peer = ConnectClient::open(peer_config).await.unwrap();
    peer.send("scoped-peer-reply", &state.cid, "Independent peer reply")
        .await
        .unwrap();
    peer.close().await;
    let replies = owner_client
        .task(connect_client::tasks::TaskCommand::Replies {
            conversation_id: state.cid.clone(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap();
    let connect_client::tasks::TaskOutcome::Replies { items, .. } = replies else {
        panic!("reply list returned the wrong outcome");
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].task_id.is_none());
    let assigned = owner_client
        .task(connect_client::tasks::TaskCommand::AssignReply {
            request_id: "scoped-explicit-assignment".into(),
            task_id: created.id.clone(),
            event_id: items[0].event_id.clone(),
            reply_to: None,
        })
        .await
        .unwrap();
    let connect_client::tasks::TaskOutcome::Record(assigned) = assigned else {
        panic!("assignment returned the wrong outcome");
    };
    assert_eq!(assigned.id, created.id);
    assert_eq!(assigned.owner_id, owner.identity_id);
    owner_client.close().await;
    let client = tools::chat(&api.body()).await;
    assert_ne!(client.call_tool(send()).await.unwrap().is_error, Some(true));
    client.cancel().await.unwrap();
    let provider_input = api.body().to_string();
    assert!(!provider_input.contains(&owner.credential));
    assert!(!provider_input.contains(&operator.credential));
    host.stop(&[&owner.credential, &operator.credential]).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncertain_creation_requires_exact_resource_reconciliation_without_repost() {
    let api = api::Api::start(500).await;
    let (state, input, _) = setup().await;
    process::Process::fails(&state, &input, &api.server.uri(), "lost-receipt").await;
    let original = intent(&state);
    assert_eq!(original["posted"], true);
    assert!(original["external_id"].is_null());
    process::Process::fails(&state, &input, &api.server.uri(), "uncertain-restart").await;
    assert_eq!(api.count(), 1);
    assert_eq!(intent(&state), original);
    process::Process::reconcile(&state, &input, &api.server.uri(), "wrong-resource", false).await;
    assert_eq!(intent(&state), original);
    process::Process::reconcile(&state, &input, &api.server.uri(), "installed-session", true).await;
    let reconciled = intent(&state);
    assert_eq!(reconciled["external_id"], "installed-session");
    assert_eq!(reconciled["nonce"], original["nonce"]);
    assert_eq!(reconciled["secret"], original["secret"]);
    let host = process::Process::start(&state, &input, &api.server.uri(), "reconciled").await;
    let client = tools::chat(&api.body()).await;
    assert_ne!(
        client
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap()
            .is_error,
        Some(true)
    );
    client.cancel().await.unwrap();
    host.stop(&[reconciled["secret"].as_str().unwrap()]).await;
    assert_eq!(api.count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalid_owner_or_callback_fails_before_intent_or_provider_dispatch() {
    let api = api::Api::start(200).await;
    let (state, input, manifest) = setup().await;
    let before =
        connect_store::Store::inspect_snapshot(&state.directory.join("state.sqlite3")).unwrap();
    let mut wrong_owner = manifest.clone();
    wrong_owner["definition"]["credential_id"] = json!("another-owner-reference");
    let mut invalid_url = manifest.clone();
    invalid_url["callback_url"] = json!("http://public.example.invalid/mcp");
    for (label, value) in [("wrong-owner", wrong_owner), ("invalid-url", invalid_url)] {
        private_replace(&input, &serde_json::to_vec(&value).unwrap()).unwrap();
        process::Process::fails(&state, &input, &api.server.uri(), label).await;
        assert!(!state.directory.join("vapi-private").exists());
        assert_eq!(
            connect_store::Store::inspect_snapshot(&state.directory.join("state.sqlite3")).unwrap(),
            before
        );
    }
    private_replace(&input, &serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut denied = state.owner.clone();
    denied.credential = "a".repeat(64);
    private_replace(
        &state.directory.join("host-owner.toml"),
        toml::to_string(&denied).unwrap().as_bytes(),
    )
    .unwrap();
    process::Process::fails(&state, &input, &api.server.uri(), "invalid-token").await;
    assert!(!state.directory.join("vapi-private").exists());
    assert!(api.server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        connect_store::Store::inspect_snapshot(&state.directory.join("state.sqlite3")).unwrap(),
        before
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callback_expiry_retires_only_the_invocation_and_leaves_conversation_available() {
    use std::{io::ErrorKind, time::Duration};
    let api = api::Api::start(200).await;
    let (state, input, mut manifest) = setup().await;
    let deadline = chrono::Utc::now().timestamp() + 30;
    manifest["definition"]["expires_at"] = json!(deadline);
    private_replace(&input, &serde_json::to_vec(&manifest).unwrap()).unwrap();
    let host = process::Process::start(&state, &input, &api.server.uri(), "expires").await;
    let address: std::net::SocketAddr = host.ready["callback_local"]
        .as_str()
        .unwrap()
        .strip_prefix("http://")
        .unwrap()
        .strip_suffix("/mcp")
        .unwrap()
        .parse()
        .unwrap();
    let client = tools::chat(&api.body()).await;
    assert_ne!(
        client
            .call_tool(CallToolRequestParams::new("connect_task_status"))
            .await
            .unwrap()
            .is_error,
        Some(true)
    );
    let remaining = deadline
        .saturating_sub(chrono::Utc::now().timestamp())
        .max(0) as u64;
    tokio::time::sleep(Duration::from_secs(remaining)).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match tokio::net::TcpStream::connect(address).await {
                Err(error) if error.kind() == ErrorKind::ConnectionRefused => break,
                Err(error) => panic!("unexpected expiry observation: {error}"),
                Ok(socket) => drop(socket),
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert!(client
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .is_err());
    let mut owner = ConnectClient::open(state.owner.clone()).await.unwrap();
    assert!(owner
        .history(&state.cid, None, 100)
        .await
        .unwrap()
        .messages
        .iter()
        .any(|message| message.body == "RETAINED-INSTALLED-CONTEXT"));
    owner.close().await;
    let _ = client.cancel().await;
    let saved = intent(&state);
    host.stop(&[saved["secret"].as_str().unwrap()]).await;
    assert_eq!(api.count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ready_means_shutdown_signals_are_already_installed() {
    let api = api::Api::start(200).await;
    let (state, input, _) = setup().await;
    let host = process::Process::start(&state, &input, &api.server.uri(), "immediate-stop").await;
    host.stop(&[&state.owner.credential]).await;
    assert_eq!(api.count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_during_creation_keeps_uncertain_intent_and_drains_without_ready() {
    use std::time::Duration;
    let api = api::Api::with_delay(200, Duration::from_secs(5)).await;
    let (state, input, _) = setup().await;
    let host = process::Process::launch(&state, &input, &api.server.uri(), "stop-before-receipt");
    tokio::time::timeout(Duration::from_secs(10), async {
        while api.count() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    host.stop(&[&state.owner.credential]).await;
    let saved = intent(&state);
    assert_eq!(saved["posted"], true);
    assert!(saved["external_id"].is_null());
    process::Process::fails(&state, &input, &api.server.uri(), "cancelled-startup-retry").await;
    assert_eq!(api.count(), 1);
    assert_eq!(intent(&state), saved);
}
