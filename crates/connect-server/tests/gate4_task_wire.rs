//! Shape, routing and wait failures through independent physical clients.
#![allow(clippy::unwrap_used)]
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ClientError, ConnectClient};
use connect_protocol::{Operation, Request, Response, REQUEST_TYPE};
use native_tasks::Harness;
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn cached_task_support_cannot_borrow_a_revoked_backing_credential() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let create = TaskCommand::Create {
        request_id: "revoke-task".into(),
        definition: h.definition(),
    };
    let TaskOutcome::Record(task) = h.owner.task(create.clone()).await.unwrap() else {
        panic!("task")
    };
    let actor = h
        .server
        .store
        .revalidate(&h.owner_config.credential_id)
        .unwrap();
    h.server
        .store
        .revoke(&actor, &h.owner_config.credential_id)
        .unwrap();
    for command in [
        create,
        TaskCommand::Invoke {
            request_id: "revoked-send".into(),
            task_id: task.id.clone(),
            step_id: "send".into(),
            operation: Box::new(Operation::Send {
                request_id: "revoked-send".into(),
                conversation_id: h.cid.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(
                    "Must not reach the peer.",
                ),
            }),
            reply_to: None,
            wait_for_reply: false,
        },
    ] {
        let error = h.owner.task(command).await.unwrap_err();
        assert!(matches!(error,ClientError::Application(e) if e.code=="unauthorized"));
    }
    assert!(h
        .peer
        .history(&h.cid, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    h.close().await;
}

#[tokio::test]
async fn caller_claimed_authority_is_rejected_before_admission_on_the_wire() {
    let mut h = Harness::start().await;
    let mut peer = support::Peer::quic(&h.owner_config).await;
    assert!(peer.authenticate(&h.owner_config.credential).await);
    for field in ["owner_id", "credential_id", "binding"] {
        let create = TaskCommand::Create {
            request_id: format!("claim-{field}"),
            definition: h.definition(),
        };
        let mut payload = serde_json::to_value(Request::new(Operation::Task {
            request: Box::new(TaskRequest::new(create.clone())),
        }))
        .unwrap();
        payload["request"][field] = json!("attacker-selected");
        let response = peer
            .exchange(UctpEnvelope::new(
                MessageType::Unknown(REQUEST_TYPE.into()),
                payload,
            ))
            .await;
        assert_eq!(
            response
                .decode_payload::<Response>()
                .unwrap()
                .result
                .unwrap_err()
                .code,
            "invalid-request"
        );
        h.owner
            .negotiate_tasks("connect:agent@local", Some(&h.cid))
            .await
            .unwrap();
        let TaskOutcome::Record(record) = h.owner.task(create).await.unwrap() else {
            panic!("record")
        };
        assert_eq!(record.owner_id, h.owner_config.identity_id);
    }
    peer.close().await;
    h.close().await;
}

#[tokio::test]
async fn another_conversation_and_nested_task_command_cannot_escape_task_scope() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = h.create("bound-task").await;
    let other = h
        .owner
        .create("other-conversation", vec![h.peer_id.clone()])
        .await
        .unwrap();
    for (id, operation) in [
        (
            "cross-conversation",
            Operation::Send {
                request_id: "cross-conversation".into(),
                conversation_id: other.id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all("must not send"),
            },
        ),
        (
            "nested",
            Operation::Task {
                request: Box::new(TaskRequest::new(TaskCommand::Cancel {
                    request_id: "nested".into(),
                    task_id: task.id.clone(),
                    reason: None,
                })),
            },
        ),
    ] {
        let error = h
            .owner
            .task(TaskCommand::Invoke {
                request_id: id.into(),
                task_id: task.id.clone(),
                step_id: id.into(),
                operation: Box::new(operation),
                reply_to: None,
                wait_for_reply: false,
            })
            .await
            .unwrap_err();
        assert!(matches!(error,ClientError::Application(e) if e.code=="forbidden"));
    }
    assert!(h
        .owner
        .history(&other.id, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    let TaskOutcome::Record(current) = h
        .owner
        .task(TaskCommand::Get { task_id: task.id })
        .await
        .unwrap()
    else {
        panic!("task")
    };
    assert_eq!(current.state, TaskState::Ready);
    h.close().await;
}

#[tokio::test]
async fn valid_task_wait_has_its_wait_budget_plus_the_exchange_deadline() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = h.create("wait-task").await;
    let TaskOutcome::Inbox { next, .. } = h
        .owner
        .task(TaskCommand::Inbox {
            task_id: task.id.clone(),
            after: None,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap()
    else {
        panic!("inbox")
    };
    let mut config = h.owner_config.clone();
    config.timeout_ms = 1000;
    let mut client = ConnectClient::open(config).await.unwrap();
    client
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.task(TaskCommand::Wait {
            task_id: task.id,
            after: next.clone(),
            limit: 100,
            wait_ms: 1500,
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        matches!(result,TaskOutcome::Inbox { events, next: actual } if events.is_empty() && actual==next)
    );
    client
        .send("after-wait", &h.cid, "The same peer remains usable.")
        .await
        .unwrap();
    client.close().await;
    h.close().await;
}
