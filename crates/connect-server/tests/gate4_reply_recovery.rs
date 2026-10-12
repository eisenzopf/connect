//! Missed notifications cannot lose or implicitly assign a durable reply.
#![allow(clippy::unwrap_used)]
#[path = "support/native_tasks.rs"]
mod native_tasks;
use connect_client::{tasks::*, ClientError, ConnectClient};
use native_tasks::{record, Harness};

async fn negotiate(h: &mut Harness) {
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    h.peer
        .negotiate_tasks("connect:reviews@local", Some(&h.cid))
        .await
        .unwrap();
}
async fn waiting(h: &mut Harness, id: &str) -> TaskRecord {
    let mut task = h.create(id).await;
    for state in [TaskState::Running, TaskState::Waiting] {
        task = record(
            h.owner
                .task(TaskCommand::Transition {
                    request_id: format!("{id}-{state:?}"),
                    task_id: task.id.clone(),
                    expected_revision: task.revision,
                    state,
                    reason: None,
                })
                .await
                .unwrap(),
        );
    }
    task
}
async fn inbox(
    client: &mut ConnectClient,
    task: &str,
    after: Option<String>,
) -> (Vec<TaskEvent>, Option<String>) {
    let TaskOutcome::Inbox { events, next } = client
        .task(TaskCommand::Inbox {
            task_id: task.into(),
            after,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap()
    else {
        panic!("task inbox")
    };
    (events, next)
}
fn forbidden(error: ClientError) {
    assert!(matches!(error, ClientError::Application(error) if error.code == "forbidden"));
}
fn assignment(task: &TaskRecord, event: &str, request: &str) -> TaskCommand {
    TaskCommand::AssignReply {
        request_id: request.into(),
        task_id: task.id.clone(),
        event_id: event.into(),
        reply_to: None,
    }
}

#[tokio::test]
async fn unassigned_reply_and_assignment_survive_restart_with_exact_task_catch_up() {
    let mut h = Harness::start().await;
    negotiate(&mut h).await;
    let one = waiting(&mut h, "task-one").await;
    let two = waiting(&mut h, "task-two").await;
    let (_, cursor_one) = inbox(&mut h.owner, &one.id, None).await;
    let (_, cursor_two) = inbox(&mut h.owner, &two.id, None).await;
    h.peer
        .send(
            "incoming",
            &h.cid,
            "The person has replied, without a question binding.",
        )
        .await
        .unwrap();
    // No notification is read, and no participant matching selects a task.
    h = h.restart().await;
    negotiate(&mut h).await;
    let list = TaskCommand::Replies {
        conversation_id: h.cid.clone(),
        after: None,
        limit: 100,
    };
    let TaskOutcome::Replies { items, .. } = h.owner.task(list.clone()).await.unwrap() else {
        panic!("replies")
    };
    assert_eq!(items.len(), 1);
    let reply = &items[0];
    assert!(reply.task_id.is_none() && reply.assigned_by.is_none() && !reply.consumed);
    assert!(inbox(&mut h.owner, &one.id, cursor_one.clone())
        .await
        .0
        .is_empty());
    assert!(inbox(&mut h.owner, &two.id, cursor_two.clone())
        .await
        .0
        .is_empty());
    let assign = assignment(&one, &reply.event_id, "assign-original");
    forbidden(h.peer.task(assign.clone()).await.unwrap_err());
    let accepted = h.owner.task(assign.clone()).await.unwrap();
    assert_eq!(record(accepted.clone()).state, TaskState::Ready);
    forbidden(
        h.owner
            .task(assignment(&two, &reply.event_id, "steal"))
            .await
            .unwrap_err(),
    );
    // A saved cursor and logical mutation are sufficient after a second restart.
    h = h.restart().await;
    negotiate(&mut h).await;
    assert_eq!(h.owner.task(assign.clone()).await.unwrap(), accepted);
    let (events, next) = inbox(&mut h.owner, &one.id, cursor_one).await;
    let replies: Vec<_> = events
        .iter()
        .filter_map(|event| match &event.data {
            TaskEventData::Reply(reply) => Some(reply),
            _ => None,
        })
        .collect();
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0].event_id, reply.event_id);
    assert_eq!(replies[0].task_id.as_deref(), Some(one.id.as_str()));
    assert_eq!(
        replies[0].assigned_by.as_deref(),
        Some(h.owner_config.identity_id.as_str())
    );
    assert!(inbox(&mut h.owner, &one.id, next).await.0.is_empty());
    assert!(inbox(&mut h.owner, &two.id, cursor_two).await.0.is_empty());
    assert_eq!(
        record(
            h.owner
                .task(TaskCommand::Get { task_id: two.id })
                .await
                .unwrap()
        )
        .state,
        TaskState::Waiting
    );
    let consume = TaskCommand::ConsumeReply {
        request_id: "consume-original".into(),
        task_id: one.id.clone(),
        event_id: reply.event_id.clone(),
        expected_revision: record(accepted).revision,
    };
    let consumed = h.owner.task(consume.clone()).await.unwrap();
    h = h.restart().await;
    negotiate(&mut h).await;
    assert_eq!(h.owner.task(consume).await.unwrap(), consumed);
    let TaskOutcome::Replies { items, .. } = h.owner.task(list).await.unwrap() else {
        panic!("replies")
    };
    assert_eq!(items.len(), 1);
    assert!(items[0].consumed);
    // A retained receipt is not authority to continue under a cancelled task.
    h.owner
        .task(TaskCommand::Cancel {
            request_id: "cancel".into(),
            task_id: one.id,
            reason: None,
        })
        .await
        .unwrap();
    forbidden(h.owner.task(assign).await.unwrap_err());
    h.close().await;
}
