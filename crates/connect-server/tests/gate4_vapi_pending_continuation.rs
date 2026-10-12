//! Provider pending effect and disconnected-host recovery through actual MCP.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_task_runtime.rs"]
mod api;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/vapi_task_tools.rs"]
mod tools;
use connect_client::tasks::*;
use connect_server::providers::Worker;
use connect_store::{Route, Scope, Store};
use serde_json::json;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn slow_provider_effect_remains_pending_then_reconnect_catches_up_exact_reply_once() {
    let api = api::Fixture::start().await;
    let mut h = native_tasks::Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    h.owner
        .send(
            "old-reference",
            &h.cid,
            "Earlier draft: cedar, decision sapphire-nine.",
        )
        .await
        .unwrap();
    let other = h
        .owner
        .create("other-conversation", vec![h.peer_id.clone()])
        .await
        .unwrap();
    h.owner
        .send("other-reference", &other.id, "UNRELATED-CONTEXT-CANARY")
        .await
        .unwrap();
    let mut definition = h.definition();
    definition
        .scope
        .operations
        .extend([TaskPermission::Inbox, TaskPermission::RequestStatus]);
    let task = native_tasks::record(
        h.owner
            .task(TaskCommand::Create {
                request_id: "pending-provider-task".into(),
                definition,
            })
            .await
            .unwrap(),
    );
    let delegate = h
        .server
        .store
        .issue(
            &h.peer_id,
            Scope::messaging(vec![h.owner_config.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    h.server
        .store
        .configure_route(
            &h.owner_config.credential_id,
            &Route {
                id: "slow-task-vapi".into(),
                identity_id: h.peer_id.clone(),
                credential_id: delegate.id,
                conversation_id: h.cid.clone(),
                connector: "vapi".into(),
                target: api::ASSISTANT.into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let worker =
        Worker::with_drivers(h.server.service.clone(), vec![api.provider.clone()]).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let endpoint = tools::Endpoint::start(
        &h.server,
        &h.owner_config,
        &task,
        &api,
        &temp.path().join("pending-session"),
        task.scope.operations.clone(),
    )
    .await;
    let client = endpoint.client().await;
    // Business rejection is journaled as a failed effect; it must not send
    // a message or create a provider job. Matching error replay is stable.
    let before = h.owner.history(&h.cid, None, 100).await.unwrap();
    let denied_args = json!({"request_id":"denied-send","step_id":"invalid","body":""});
    let denied = tools::call(&client, "connect_send", denied_args.clone()).await;
    assert_eq!(denied.is_error, Some(true));
    assert_eq!(
        denied.structured_content.as_ref().unwrap()["code"],
        "invalid-request"
    );
    assert_eq!(h.owner.history(&h.cid, None, 100).await.unwrap(), before);
    let (events, _) = tools::inbox(
        tools::call(
            &client,
            "connect_task_wait",
            json!({"limit":100,"wait_ms":0}),
        )
        .await,
    );
    let rejected: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.data {
            TaskEventData::Effect(effect) if effect.state == EffectState::Failed => Some(effect),
            _ => None,
        })
        .collect();
    assert_eq!(rejected.len(), 1);
    assert_eq!(rejected[0].request_id, "denied-send");
    let frozen = Store::inspect_snapshot(h.server.store.path()).unwrap();
    assert_eq!(
        tools::call(&client, "connect_send", denied_args)
            .await
            .structured_content,
        denied.structured_content
    );
    assert_eq!(
        Store::inspect_snapshot(h.server.store.path()).unwrap(),
        frozen
    );
    assert!(api.requests().is_empty());
    let question = json!({"request_id":"slow-mcp-question","step_id":"question","body":"Choose the draft decision","wait_for_reply":true});
    let started = Instant::now();
    let TaskOutcome::Tool(TaskToolResult::Pending {
        task_id,
        effect_id: Some(effect),
    }) = tools::outcome(tools::call(&client, "connect_send", question.clone()).await)
    else {
        panic!("blocked provider must remain pending")
    };
    assert_eq!(task_id, task.id);
    assert!(started.elapsed() < Duration::from_secs(10));
    api.posted_chat().await;
    let (_, cursor) = tools::inbox(
        tools::call(
            &client,
            "connect_task_wait",
            json!({"limit":100,"wait_ms":0}),
        )
        .await,
    );
    let started = Instant::now();
    let pending = tools::call(
        &client,
        "connect_task_wait",
        json!({"after":cursor,"limit":100,"wait_ms":10000}),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(started.elapsed() >= Duration::from_secs(7));
    assert_eq!(pending.meta.as_ref().unwrap()["connect"]["pending"], true);
    let (events, next) = tools::inbox(pending);
    assert!(events.is_empty());
    assert_eq!(next, cursor);
    client.cancel().await.unwrap();
    // The model/host is disconnected when the real provider worker accepts the
    // reply. Catch-up relies on the retained journal cursor, not notifications.
    api.release_chat();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let jobs = h.owner.provider_status(&h.cid).await.unwrap();
            if jobs.len() == 1 && jobs[0].state == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let client = endpoint.client().await;
    let (events, _) = tools::inbox(
        tools::call(
            &client,
            "connect_task_wait",
            json!({"after":cursor,"limit":100,"wait_ms":0}),
        )
        .await,
    );
    let replies: Vec<_> = events
        .iter()
        .filter_map(|e| {
            if let TaskEventData::Reply(r) = &e.data {
                Some(r)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].task_id.as_deref(), Some(task.id.as_str()));
    let TaskOutcome::Tool(TaskToolResult::Completed { task_id, outcome }) =
        tools::outcome(tools::call(&client, "connect_send", question).await)
    else {
        panic!("completed receipt")
    };
    assert_eq!(task_id, task.id);
    let connect_protocol::Outcome::Accepted { message_id, .. } = *outcome else {
        panic!("accepted message")
    };
    assert_eq!(replies[0].reply_to.as_deref(), Some(message_id.as_str()));
    let TaskOutcome::Effect(record) = tools::outcome(
        tools::call(
            &client,
            "connect_effect_status",
            json!({"effect_id":effect}),
        )
        .await,
    ) else {
        panic!("effect")
    };
    assert_eq!(record.state, EffectState::Succeeded);
    assert_eq!(record.task_id, task.id);
    let TaskOutcome::Record(record) =
        tools::outcome(tools::call(&client, "connect_task_status", json!({})).await)
    else {
        panic!("task")
    };
    assert_eq!(record.state, TaskState::Ready);
    let sources =
        tools::sources(tools::call(&client, "connect_recall", json!({"limit":100})).await);
    let reply = sources
        .iter()
        .find(|s| s.body.as_deref() == Some("Slow provider answer: use sapphire-nine."))
        .unwrap();
    assert!(reply.provenance.contains("conversation-journal:"));
    assert_eq!(reply.state, "available");
    assert!(!serde_json::to_string(&sources)
        .unwrap()
        .contains("UNRELATED-CONTEXT-CANARY"));
    let requests = api.requests();
    assert_eq!(requests.len(), 1);
    let context = requests[0]["assistantOverrides"]["variableValues"]["connect_context"]
        .as_str()
        .unwrap();
    assert!(context.contains("Earlier draft"));
    assert!(!context.contains("UNRELATED-CONTEXT-CANARY"));
    assert!(requests[0].get("previousChatId").is_none());
    assert_eq!(h.owner.provider_status(&h.cid).await.unwrap().len(), 1);
    client.cancel().await.unwrap();
    endpoint.close().await;
    worker.shutdown().await.unwrap();
    h.close().await;
}
