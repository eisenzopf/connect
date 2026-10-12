//! Ordinary agent access retains current identity authorization on every request.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{tasks::*, ClientError, ConnectClient};
use connect_protocol::{Operation, Outcome};
use mcp::{call, outcome};
use serde_json::json;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revocation_denies_existing_mcp_native_replay_and_new_work_without_affecting_peer() {
    let f = agent::Fixture::start().await;
    let http = mcp::http().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut native = f.agent_client().await;
    let conversation = native
        .create("revocation-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    let args = json!({"request_id":"accepted-before-revoke","conversation_id":conversation.id,"body":"Keep exactly this accepted message."});
    let accepted = outcome(call(&client, "connect_send", args.clone()).await);
    assert!(matches!(accepted, Outcome::Accepted { .. }));
    f.revoke(&f.agent.credential_id);
    let url = format!("http://{}/mcp", f.server.mcp_address.unwrap());
    for body in [
        mcp::initialize(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"connect_send","arguments":args}}),
    ] {
        assert_eq!(
            http.post(&url)
                .bearer_auth(&f.agent.credential)
                .header("accept", "application/json, text/event-stream")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    for operation in [
        Operation::Send {
            request_id: "accepted-before-revoke".into(),
            conversation_id: conversation.id.clone(),
            body: "Keep exactly this accepted message.".into(),
        },
        Operation::Send {
            request_id: "new-after-revoke".into(),
            conversation_id: conversation.id.clone(),
            body: "Must not be written.".into(),
        },
    ] {
        assert!(matches!(native.request(operation).await,
            Err(ClientError::Application(error)) if error.code == "unauthorized"));
    }
    let mut peer = f.peer_client().await;
    assert_eq!(
        peer.history(&conversation.id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    peer.send(
        "unaffected-peer",
        &conversation.id,
        "The peer remains independently authorized.",
    )
    .await
    .unwrap();
    assert_eq!(
        peer.history(&conversation.id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        2
    );
    peer.close().await;
    native.close().await;
    let _ = client.cancel().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_normal_credential_expiry_denies_discovery_and_accepted_receipt_replay() {
    let f = agent::Fixture::start().await;
    let http = mcp::http().await;
    let mut creator = f.agent_client().await;
    let conversation = creator
        .create("expiry-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    creator.close().await;
    let expires_at = chrono::Utc::now().timestamp() + 4;
    let config = f.short_credential(expires_at);
    let client = mcp::client_with_http(
        f.server.mcp_address.unwrap(),
        &config.credential,
        http.clone(),
    )
    .await;
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let args = json!({"request_id":"accepted-before-expiry","conversation_id":conversation.id,"body":"Accepted under the normal identity credential."});
    outcome(call(&client, "connect_send", args.clone()).await);
    while chrono::Utc::now().timestamp() < expires_at {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let url = format!("http://{}/mcp", f.server.mcp_address.unwrap());
    for body in [
        mcp::initialize(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"connect_send","arguments":args}}),
    ] {
        assert_eq!(
            http.post(&url)
                .bearer_auth(&config.credential)
                .header("accept", "application/json, text/event-stream")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
    }
    // The public SDK checks the trusted auth.session expiry before sending.
    // This is a definite authentication denial, not a transport timeout.
    assert!(matches!(
        native
            .send(
                "accepted-before-expiry",
                &conversation.id,
                "Accepted under the normal identity credential."
            )
            .await,
        Err(ClientError::Authentication)
    ));
    let mut peer = f.peer_client().await;
    assert_eq!(
        peer.history(&conversation.id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    peer.close().await;
    native.close().await;
    let _ = client.cancel().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_credential_cannot_select_foreign_actor_conversation_task_or_provider_authority() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let http = mcp::http().await;
    let mut foreign = f.foreign_client().await;
    let room = foreign
        .create("private-foreign-room", vec![])
        .await
        .unwrap();
    foreign
        .send("foreign-secret", &room.id, "Foreign context must not leak.")
        .await
        .unwrap();
    foreign
        .negotiate_tasks("connect:operator@local", Some(&room.id))
        .await
        .unwrap();
    let TaskOutcome::Record(task) = foreign
        .task(TaskCommand::Create {
            request_id: "foreign-task".into(),
            definition: TaskDefinition {
                conversation_id: room.id.clone(),
                scope: TaskScope {
                    operations: vec![TaskPermission::Send, TaskPermission::History],
                    recipients: vec![f.foreign.identity_id.clone()],
                    sms_enrollments: vec![],
                },
                expires_at: chrono::Utc::now().timestamp() + 600,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap()
    else {
        panic!("foreign task")
    };
    let denied = call(
        &client,
        "connect_history",
        json!({"conversation_id":room.id,"limit":100}),
    )
    .await;
    assert_eq!(mcp::error(denied).code, "forbidden");
    let denied = call(&client, "connect_task_status", json!({"task_id":task.id})).await;
    assert_eq!(mcp::error(denied).code, "forbidden");
    let denied = call(
        &client,
        "connect_task_cancel",
        json!({
            "request_id":"foreign-cancel","task_id":task.id,"reason":"Do not touch foreign work"
        }),
    )
    .await;
    assert_eq!(mcp::error(denied).code, "forbidden");
    let TaskOutcome::Record(unchanged) = foreign
        .task(TaskCommand::Get {
            task_id: task.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("foreign task remains inspectable by its owner")
    };
    assert_eq!(unchanged.state, TaskState::Ready);
    let mut native = f.agent_client().await;
    let allowed = native
        .create("allowed-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap();
    let denied = call(
        &client,
        "connect_task_create",
        json!({
            "request_id":"foreign-ceiling","conversation_id":allowed.id,
            "operations":["send"],"recipients":[f.foreign.identity_id]
        }),
    )
    .await;
    assert_eq!(mcp::error(denied).code, "forbidden");
    mcp::invalid(
        &client,
        "connect_task_create",
        json!({
            "request_id":"admin-ceiling","conversation_id":allowed.id,
            "operations":["admin"],"recipients":[f.peer.identity_id]
        }),
    )
    .await;
    for (key, value) in [
        ("owner_id", json!(f.foreign.identity_id)),
        ("credential_id", json!(f.foreign.credential_id)),
        (
            "binding",
            json!({"kind":"provider_call","external_id":"claimed-call"}),
        ),
        ("task_id", json!(task.id)),
    ] {
        let mut args = json!({"request_id":"malformed","conversation_id":allowed.id,"body":"Must not be sent."});
        args[key] = value;
        mcp::invalid(&client, "connect_send", args).await;
    }
    mcp::invalid(
        &client,
        "connect_send",
        json!({"request_id":"bad-selector","conversation_id":17,"body":"Must not be sent."}),
    )
    .await;
    for header in ["x-call-id", "x-chat-id", "x-session-id"] {
        let response = http
            .post(format!("http://{}/mcp", f.server.mcp_address.unwrap()))
            .bearer_auth(&f.agent.credential)
            .header(header, "forged-provider-resource")
            .header("accept", "application/json, text/event-stream")
            .json(&mcp::initialize())
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            401,
            "{header} must not supply provider authority"
        );
    }
    let catalog = client.list_tools(None).await.unwrap();
    for name in [
        "connect_issue",
        "connect_create_identity",
        "connect_task_grant",
        "connect_provider_config",
    ] {
        assert!(catalog.tools.iter().all(|tool| tool.name != name));
    }
    let response = call(
        &client,
        "connect_context_view",
        json!({"conversation_id":allowed.id,"limit":100}),
    )
    .await;
    let encoded = serde_json::to_string(&response).unwrap();
    for secret in [
        &f.agent.credential,
        &f.peer.credential,
        &f.foreign.credential,
        &f.operator_secret,
    ] {
        assert!(!encoded.contains(secret));
    }
    assert!(!encoded.contains("Foreign context must not leak."));
    outcome(response);
    assert!(native
        .history(&allowed.id, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    native.close().await;
    foreign.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_inspection_reports_current_named_access_without_secret_or_actor_selectors() {
    let f = agent::Fixture::start().await;
    let http = mcp::http().await;
    let client = mcp::client_with_http(
        f.server.mcp_address.unwrap(),
        &f.agent.credential,
        http.clone(),
    )
    .await;
    let result = call(&client, "connect_inspect", json!({})).await;
    assert_ne!(result.is_error, Some(true));
    let metadata = result.structured_content.unwrap();
    assert_eq!(metadata["kind"], "identity_access");
    let data = &metadata["data"];
    assert_eq!(data["access_mode"], "identity");
    assert_eq!(data["identity_id"], f.agent.identity_id);
    assert_eq!(data["credential_id"], f.agent.credential_id);
    assert_eq!(data["expires_at"], f.expires_at);
    let operations = data["operations"].as_array().unwrap();
    assert!(operations.iter().any(|operation| operation == "send"));
    assert!(operations
        .iter()
        .all(|operation| !matches!(operation.as_str(), Some("*" | "admin" | "task_grant"))));
    let tools = data["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool == "connect_send"));
    assert!(tools.iter().any(|tool| tool == "connect_create"));
    assert!(tools.iter().all(|tool| !matches!(
        tool.as_str(),
        Some("connect_issue" | "connect_create_identity" | "connect_provider_config")
    )));
    let encoded = serde_json::to_string(&metadata).unwrap();
    for secret in [
        &f.agent.credential,
        &f.peer.credential,
        &f.foreign.credential,
        &f.operator_secret,
    ] {
        assert!(!encoded.contains(secret));
    }
    assert!(!encoded.contains(f.server.store.path().to_str().unwrap()));
    mcp::invalid(
        &client,
        "connect_inspect",
        json!({"identity_id":f.foreign.identity_id}),
    )
    .await;
    f.revoke(&f.agent.credential_id);
    let response = http
        .post(format!("http://{}/mcp", f.server.mcp_address.unwrap()))
        .bearer_auth(&f.agent.credential)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"connect_inspect","arguments":{}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let _ = client.cancel().await;
    f.close().await;
}
