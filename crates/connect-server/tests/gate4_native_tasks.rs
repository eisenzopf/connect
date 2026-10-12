//! Owner task policy and sender replies over actual native UCTP/QUIC.
#![allow(clippy::unwrap_used)]
#[path = "support/native_tasks.rs"]
mod native_tasks;
use connect_client::{tasks::*, ClientError, ConnectClient};
use connect_protocol::{Operation, Outcome};
use native_tasks::{record, Harness};

fn code(error: ClientError) -> String {
    let ClientError::Application(error) = error else {
        panic!("expected application denial: {error}")
    };
    error.code
}

#[tokio::test]
async fn native_task_profile_is_explicit_and_does_not_expand_messaging_authority() {
    let mut h = Harness::start().await;
    assert_eq!(
        code(
            h.owner
                .task(TaskCommand::Get {
                    task_id: "unknown".into()
                })
                .await
                .unwrap_err()
        ),
        "unsupported-profile"
    );
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    h.peer
        .negotiate_tasks("connect:reviews@local", Some(&h.cid))
        .await
        .unwrap();
    let definition = h.definition();
    assert_eq!(
        code(
            h.peer
                .task(TaskCommand::Create {
                    request_id: "denied-create".into(),
                    definition: definition.clone()
                })
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    let mut request = TaskRequest::new(TaskCommand::Create {
        request_id: "unsupported-create".into(),
        definition,
    });
    request.task_profile = "connect-task/999".into();
    assert_eq!(
        code(
            h.owner
                .request(Operation::Task {
                    request: Box::new(request)
                })
                .await
                .unwrap_err()
        ),
        "unsupported-profile"
    );
    let task = h.create("supported-create").await;
    assert_eq!(task.owner_id, h.owner_config.identity_id);
    assert_eq!(
        code(
            h.peer
                .task(TaskCommand::Get { task_id: task.id })
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    h.peer
        .send(
            "ordinary-send",
            &h.cid,
            "Old communication permission still works.",
        )
        .await
        .unwrap();
    h.close().await;
}

#[tokio::test]
async fn native_question_reply_recall_and_terminal_receipts_keep_the_same_conversation() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    h.peer
        .negotiate_tasks("connect:reviews@local", Some(&h.cid))
        .await
        .unwrap();
    let definition = h.definition();
    let create = TaskCommand::Create {
        request_id: "task-create".into(),
        definition,
    };
    let first = h.owner.task(create.clone()).await.unwrap();
    assert_eq!(h.owner.task(create).await.unwrap(), first);
    let mut task = record(first);
    task = record(
        h.owner
            .task(TaskCommand::Transition {
                request_id: "start".into(),
                task_id: task.id.clone(),
                expected_revision: task.revision,
                state: TaskState::Running,
                reason: None,
            })
            .await
            .unwrap(),
    );
    let question = TaskCommand::Invoke {
        request_id: "question".into(),
        task_id: task.id.clone(),
        step_id: "review".into(),
        operation: Box::new(Operation::Send {
            request_id: "question".into(),
            conversation_id: h.cid.clone(),
            body: "Approve the violet draft?".into(),
        }),
        reply_to: None,
        wait_for_reply: true,
    };
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect_id),
        ..
    }) = h.owner.task(question.clone()).await.unwrap()
    else {
        panic!("pending question")
    };
    let TaskOutcome::Effect(effect) = h
        .owner
        .task(TaskCommand::EffectGet {
            task_id: task.id.clone(),
            effect_id,
        })
        .await
        .unwrap()
    else {
        panic!("effect")
    };
    let Outcome::Accepted { message_id, .. } = *effect.outcome.unwrap() else {
        panic!("message")
    };
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
    let (waiting, answering) = tokio::join!(
        h.owner.task(TaskCommand::Wait {
            task_id: task.id.clone(),
            after: next,
            limit: 100,
            wait_ms: 1000
        }),
        h.peer.task(TaskCommand::Reply {
            request_id: "answer".into(),
            conversation_id: h.cid.clone(),
            reply_to: message_id,
            body: "Approved: violet.".into()
        })
    );
    let TaskOutcome::ReplyAccepted { reply, .. } = answering.unwrap() else {
        panic!("reply")
    };
    assert_eq!(reply.task_id.as_deref(), Some(task.id.as_str()));
    assert!(
        matches!(waiting.unwrap(),TaskOutcome::Inbox { events, .. } if events.iter().any(|e| matches!(&e.data,TaskEventData::Reply(r) if r.event_id == reply.event_id)))
    );
    assert!(matches!(
        h.owner.task(question).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    task = record(
        h.owner
            .task(TaskCommand::Get {
                task_id: task.id.clone(),
            })
            .await
            .unwrap(),
    );
    task = record(
        h.owner
            .task(TaskCommand::ConsumeReply {
                request_id: "consume".into(),
                task_id: task.id.clone(),
                event_id: reply.event_id,
                expected_revision: task.revision,
            })
            .await
            .unwrap(),
    );
    let TaskOutcome::Recall { sources, .. } = h
        .owner
        .task(TaskCommand::Recall {
            task_id: task.id.clone(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("recall")
    };
    assert!(sources
        .iter()
        .any(|s| s.body.as_deref() == Some("Approved: violet.")));
    assert_eq!(task.state, TaskState::Ready);
    // Receiving/consuming an answer does not claim that the application resumed
    // its work. The runner explicitly transitions ready -> running first.
    task = record(
        h.owner
            .task(TaskCommand::Transition {
                request_id: "resume".into(),
                task_id: task.id.clone(),
                expected_revision: task.revision,
                state: TaskState::Running,
                reason: None,
            })
            .await
            .unwrap(),
    );
    let complete = TaskCommand::Transition {
        request_id: "complete".into(),
        task_id: task.id.clone(),
        expected_revision: task.revision,
        state: TaskState::Completed,
        reason: None,
    };
    let terminal = h.owner.task(complete.clone()).await.unwrap();
    assert_eq!(record(terminal.clone()).state, TaskState::Completed);
    assert_eq!(h.owner.task(complete).await.unwrap(), terminal);
    let history = h.owner.history(&h.cid, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.body == "Approve the violet draft?")
            .count(),
        1
    );
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.body == "Approved: violet.")
            .count(),
        1
    );
    h.close().await;
}

#[tokio::test]
async fn native_claim_assignment_cancel_and_reconnect_use_retained_records() {
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let task = h.create("assign-task").await;
    let TaskOutcome::Claim(Some(claim)) = h
        .owner
        .task(TaskCommand::Claim {
            request_id: "claim".into(),
            task_id: task.id.clone(),
            step_id: "review".into(),
            worker_id: "worker".into(),
            expected_revision: None,
        })
        .await
        .unwrap()
    else {
        panic!("claim")
    };
    assert_eq!(
        h.owner
            .task(TaskCommand::ClaimGet {
                task_id: task.id.clone(),
                step_id: "review".into()
            })
            .await
            .unwrap(),
        TaskOutcome::Claim(Some(claim.clone()))
    );
    h.owner
        .task(TaskCommand::ReleaseClaim {
            request_id: "release".into(),
            task_id: task.id.clone(),
            step_id: "review".into(),
            worker_id: "worker".into(),
            expected_revision: claim.revision,
        })
        .await
        .unwrap();
    h.peer
        .send("unsolicited", &h.cid, "A reply without task selection.")
        .await
        .unwrap();
    let TaskOutcome::Replies { items, .. } = h
        .owner
        .task(TaskCommand::Replies {
            conversation_id: h.cid.clone(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("replies")
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].task_id.is_none());
    let assignment = TaskCommand::AssignReply {
        request_id: "assign".into(),
        task_id: task.id.clone(),
        event_id: items[0].event_id.clone(),
        reply_to: None,
    };
    let assigned = h.owner.task(assignment.clone()).await.unwrap();
    assert_eq!(h.owner.task(assignment).await.unwrap(), assigned);
    let cancel = TaskCommand::Cancel {
        request_id: "cancel".into(),
        task_id: task.id.clone(),
        reason: Some("owner cancelled".into()),
    };
    let terminal = h.owner.task(cancel.clone()).await.unwrap();
    assert_eq!(record(terminal.clone()).state, TaskState::Cancelled);
    let mut replacement = ConnectClient::open(h.owner_config.clone()).await.unwrap();
    replacement
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    assert_eq!(replacement.task(cancel).await.unwrap(), terminal);
    assert_eq!(
        record(
            replacement
                .task(TaskCommand::Get { task_id: task.id })
                .await
                .unwrap()
        )
        .state,
        TaskState::Cancelled
    );
    replacement
        .send(
            "same-cid",
            &h.cid,
            "Conversation continues after cancellation.",
        )
        .await
        .unwrap();
    replacement.close().await;
    h.close().await;
}
