//! Offline task follow-through through the public SDK, worker and pinned Vapi SDK.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/provider_task_flow.rs"]
mod provider_task_flow;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use provider_task_flow::{mount_reply, question, reply, task};

use connect_server::providers::Worker;
use connect_store::{Route, Scope};
use native_tasks::Harness;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";
const FACT: &str = "Private conversation reference: the draft is cedar.";

#[tokio::test(flavor = "multi_thread")]
async fn task_grants_bound_provider_context_and_replies_across_native_and_http() {
    // Provider construction may synchronously discover platform proxy settings.
    // Complete installation setup before opening peers with bounded idle leases.
    let mock = MockServer::start().await;
    Mock::given(method("GET")).and(path(format!("/assistant/{ASSISTANT}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":ASSISTANT,"model":{"messages":[{"role":"system","content":"History: {{connect_context}}"}]}
        }))).expect(2).mount(&mock).await;
    mount_reply(&mock, FACT, "ordinary-chat", "Prior ordinary response").await;
    mount_reply(
        &mock,
        "Narrow task question",
        "narrow-chat",
        "Narrow task answer",
    )
    .await;
    mount_reply(
        &mock,
        "Recall task question",
        "recall-chat",
        "Recall task answer",
    )
    .await;
    let driver = Arc::new(Vapi::new("offline-fixture", &mock.uri()).unwrap());
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let delegate = h
        .server
        .store
        .issue(
            &h.peer_id,
            Scope::messaging(vec![h.owner_config.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    h.server
        .store
        .configure_route(
            &h.owner_config.credential_id,
            &Route {
                id: "task-vapi".into(),
                identity_id: h.peer_id.clone(),
                credential_id: delegate.id,
                conversation_id: h.cid.clone(),
                connector: "vapi".into(),
                target: ASSISTANT.into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let worker = Worker::with_drivers(h.server.service.clone(), vec![driver]).unwrap();
    h.owner.send("ordinary-seed", &h.cid, FACT).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if h.owner
                .provider_status(&h.cid)
                .await
                .unwrap()
                .iter()
                .any(|j| j.state == "completed")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let narrow_task = task(&mut h, "narrow-task").await;
    let recall_task = task(&mut h, "recall-task").await;
    let expiry = chrono::Utc::now().timestamp() + 180;
    let narrow = grants::issue(
        &h,
        &narrow_task,
        "narrow-grant",
        vec![TaskPermission::Send, TaskPermission::Inbox],
        expiry,
    )
    .await;
    let full = grants::issue(
        &h,
        &recall_task,
        "recall-grant",
        vec![
            TaskPermission::Send,
            TaskPermission::Inbox,
            TaskPermission::History,
        ],
        expiry,
    )
    .await;
    let mut a = ConnectClient::open(h.owner_config.clone().with_task_grant(&narrow).unwrap())
        .await
        .unwrap();
    let mut b = ConnectClient::open(h.owner_config.clone().with_task_grant(&full).unwrap())
        .await
        .unwrap();
    a.negotiate_task(&narrow_task.id).await.unwrap();
    b.negotiate_task(&recall_task.id).await.unwrap();
    let ask_a = question(&narrow_task, "ask-narrow", "Narrow task question");
    let ask_b = question(&recall_task, "ask-recall", "Recall task question");
    for (client, request) in [(&mut a, &ask_a), (&mut b, &ask_b)] {
        // The provider may already have completed when a local ACK arrives.
        assert!(matches!(
            client.task(request.clone()).await.unwrap(),
            TaskOutcome::Tool(TaskToolResult::Pending { .. } | TaskToolResult::Completed { .. })
        ));
    }
    // A successful provider turn must precede reply-inbox assertions. This keeps
    // a driver/network failure from being mistaken for missing task routing.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let jobs = h.owner.provider_status(&h.cid).await.unwrap();
            if jobs.len() == 3 && jobs.iter().all(|job| job.state == "completed") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let requests = mock.received_requests().await.unwrap();
    let body = |input: &str| -> Value {
        let request = requests
            .iter()
            .find(|r| r.method == "POST" && r.body_json::<Value>().unwrap()["input"] == input)
            .unwrap();
        request.body_json().unwrap()
    };
    let narrow_body = body("Narrow task question");
    assert!(narrow_body.get("assistantOverrides").is_none());
    assert!(narrow_body.get("previousChatId").is_none());
    let recall_body = body("Recall task question");
    assert!(recall_body.get("previousChatId").is_none());
    let context = recall_body["assistantOverrides"]["variableValues"]["connect_context"]
        .as_str()
        .unwrap();
    assert!(context.contains(FACT) && context.contains("conversation-journal:"));
    assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 3);
    let reply_a = reply(&mut a, &narrow_task).await;
    let reply_b = reply(&mut b, &recall_task).await;
    assert_eq!(reply_a.task_id.as_deref(), Some(narrow_task.id.as_str()));
    assert_eq!(reply_b.task_id.as_deref(), Some(recall_task.id.as_str()));
    assert_ne!(reply_a.event_id, reply_b.event_id);
    for (client, record, grant, request, expected) in [
        (&mut a, &narrow_task, &narrow, &ask_a, &reply_a),
        (&mut b, &recall_task, &full, &ask_b, &reply_b),
    ] {
        provider_task_flow::assert_receipt(&h, client, record, grant, request, expected).await;
    }
    assert_eq!(
        grants::code(
            a.task(TaskCommand::Recall {
                task_id: narrow_task.id,
                after: None,
                limit: 100,
            })
            .await
            .unwrap_err()
        ),
        "forbidden"
    );
    let TaskOutcome::Recall { sources, .. } = b
        .task(TaskCommand::Recall {
            task_id: recall_task.id,
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("recall")
    };
    assert!(sources.iter().any(|s| s.body.as_deref() == Some(FACT)));
    assert_eq!(
        h.owner
            .history(&h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        6
    );
    // Replay must not even create a second queued job, regardless of scheduling.
    let jobs = h.owner.provider_status(&h.cid).await.unwrap();
    assert_eq!(jobs.len(), 3);
    assert!(jobs.iter().all(|job| job.state == "completed"));
    assert_eq!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        3
    );
    worker.shutdown().await.unwrap();
    a.close().await;
    b.close().await;
    h.close().await;
}
