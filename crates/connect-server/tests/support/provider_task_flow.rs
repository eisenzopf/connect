//! Public-client assertions for the offline provider task worker qualification.
#![allow(clippy::unwrap_used)]
use super::{
    grants,
    native_tasks::{record, Harness},
};
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::{Operation, Outcome};
use serde_json::json;
use std::time::Duration;
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};
pub(crate) async fn mount_reply(server: &MockServer, input: &str, id: &str, answer: &str) {
    Mock::given(method("POST"))
        .and(path("/chat"))
        .and(body_partial_json(json!({"input":input})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":id,"output":[{"role":"assistant","content":answer}]
        })))
        .expect(1)
        .mount(server)
        .await;
}

pub(crate) async fn task(h: &mut Harness, id: &str) -> TaskRecord {
    let mut definition = h.definition();
    definition.scope.operations.push(TaskPermission::Inbox);
    record(
        h.owner
            .task(TaskCommand::Create {
                request_id: id.into(),
                definition,
            })
            .await
            .unwrap(),
    )
}

pub(crate) fn question(task: &TaskRecord, id: &str, body: &str) -> TaskCommand {
    TaskCommand::Invoke {
        request_id: id.into(),
        task_id: task.id.clone(),
        step_id: id.into(),
        operation: Box::new(Operation::Send {
            request_id: id.into(),
            conversation_id: task.conversation_id.clone(),
            message: connect_protocol::messages::MessageDraft::text_all(body),
        }),
        reply_to: None,
        wait_for_reply: true,
    }
}

pub(crate) async fn reply(client: &mut ConnectClient, task: &TaskRecord) -> TaskReply {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut after = None;
        loop {
            let TaskOutcome::Inbox { events, next } = client
                .task(TaskCommand::Wait {
                    task_id: task.id.clone(),
                    after: after.clone(),
                    limit: 100,
                    wait_ms: 500,
                })
                .await
                .unwrap()
            else {
                panic!("inbox")
            };
            for event in events {
                if let TaskEventData::Reply(reply) = event.data {
                    return reply;
                }
            }
            after = next;
        }
    })
    .await
    .unwrap()
}

pub(crate) async fn assert_receipt(
    h: &Harness,
    client: &mut ConnectClient,
    task: &TaskRecord,
    grant: &ProtectedTaskGrant,
    request: &TaskCommand,
    expected: &TaskReply,
) {
    let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) =
        client.task(request.clone()).await.unwrap()
    else {
        panic!("completed communication effect")
    };
    let Outcome::Accepted { message_id, .. } = outcome.as_ref() else {
        panic!("accepted")
    };
    assert_eq!(expected.reply_to.as_deref(), Some(message_id.as_str()));
    let response = grants::operation(
        h,
        grant.expose_secret(),
        Operation::Task {
            request: Box::new(TaskRequest::new(TaskCommand::Inbox {
                task_id: task.id.clone(),
                after: None,
                limit: 100,
                wait_ms: 0,
            })),
        },
    )
    .await;
    let Outcome::Task(outcome) = response.result.unwrap() else {
        panic!("task response")
    };
    let TaskOutcome::Inbox { events, .. } = *outcome else {
        panic!("inbox")
    };
    let replies: Vec<_> = events
        .into_iter()
        .filter_map(|e| match e.data {
            TaskEventData::Reply(reply) => Some(reply),
            _ => None,
        })
        .collect();
    assert_eq!(replies, vec![(*expected).clone()]);
    assert_eq!(
        record(
            client
                .task(TaskCommand::Get {
                    task_id: task.id.clone()
                })
                .await
                .unwrap()
        )
        .state,
        TaskState::Ready
    );
}
