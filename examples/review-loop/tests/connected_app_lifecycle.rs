//! One cancelled task does not remove app presence; physical recovery keeps IDs.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/connected_app.rs"]
mod support;
use connect_client::{
    tasks::{TaskCommand, TaskRequest},
    Operation, Outcome,
};
use connect_review_loop::connected_app::{Decision, Draft, Request};
use support::{exited, Fixture};
fn task(command: TaskCommand) -> Operation {
    Operation::Task {
        request: Box::new(TaskRequest::new(command)),
    }
}
fn submission(id: &str) -> String {
    serde_json::to_string(&Request::SubmitDraft {
        request_id: id.into(),
        draft: Draft {
            id: "draft-new".into(),
            title: "Intake".into(),
            body: "Actual incoming content".into(),
        },
    })
    .unwrap()
}
async fn create(f: &Fixture, who: &str, id: &str) -> String {
    let Outcome::Conversation(c) = f
        .request(
            who,
            Operation::Create {
                request_id: id.into(),
                participants: vec![f.identity("reviews")],
            },
        )
        .await
    else {
        panic!("conversation")
    };
    c.id
}
async fn send(f: &Fixture, who: &str, cid: &str, id: &str, body: String) {
    f.request(
        who,
        Operation::Send {
            request_id: id.into(),
            conversation_id: cid.into(),
            body,
        },
    )
    .await;
}
async fn approve(f: &Fixture, q: &connect_client::Message, id: &str) {
    f.request(
        "agent",
        task(TaskCommand::Reply {
            request_id: id.into(),
            conversation_id: q.conversation_id.clone(),
            reply_to: q.id.clone(),
            body: serde_json::to_string(&Request::ReviewDecision {
                draft_id: "draft-002".into(),
                decision: Decision::Approved,
                note: "Approved".into(),
            })
            .unwrap(),
        }),
    )
    .await;
}
#[tokio::test]
async fn outgoing_task_cancellation_preserves_incoming_app_presence_after_restart() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None);
    let question = f.question().await;
    let stats = f.settled(0, 0).await;
    let task_id = stats.outgoing_task_id.unwrap();
    f.request(
        "reviews",
        task(TaskCommand::Cancel {
            request_id: "cancel-outgoing-only".into(),
            task_id: task_id.clone(),
            reason: Some("Review no longer needed".into()),
        }),
    )
    .await;
    let cid = create(&f, "agent", "after-cancel-contact").await;
    send(
        &f,
        "agent",
        &cid,
        "intake-after-cancel",
        submission("still-present-1"),
    )
    .await;
    approve(&f, &question, "late-decision").await;
    let first = f.settled(0, 1).await;
    assert_eq!(first.outgoing_state.as_deref(), Some("cancelled"));
    assert_eq!(first.results[0].conversation_id, cid);
    assert_ne!(first.results[0].task_id, task_id);
    support::stop(&mut app).await;
    f.restart_server().await;
    app = f.spawn(None);
    send(
        &f,
        "agent",
        &cid,
        "second-intake",
        submission("still-present-2"),
    )
    .await;
    let next = f.settled(0, 2).await;
    assert_eq!(next.outgoing_task_id.as_deref(), Some(task_id.as_str()));
    assert_eq!(next.outgoing_state.as_deref(), Some("cancelled"));
    assert!(next.results.iter().all(|r| r.conversation_id == cid));
    assert_eq!(
        f.history("agent", &question.conversation_id)
            .await
            .iter()
            .filter(|m| m.body.contains("\"review_request\""))
            .count(),
        1
    );
    f.finish(&mut app).await;
}

#[tokio::test]
async fn running_app_reconnects_original_question_and_reply_after_server_restart() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None);
    let question = f.question().await;
    f.restart_server().await;
    approve(&f, &question, "after-live-restart").await;
    let stats = f.settled(1, 0).await;
    assert_eq!(stats.results[0].conversation_id, question.conversation_id);
    let history = f.history("agent", &question.conversation_id).await;
    assert_eq!(
        history
            .iter()
            .filter(|m| m.body.contains("\"review_request\""))
            .count(),
        1
    );
    assert_eq!(
        history
            .iter()
            .filter(|m| m.body.contains("\"review_recorded\""))
            .count(),
        1
    );
    f.finish(&mut app).await;
}
#[tokio::test]
async fn revoked_app_credential_cannot_replay_consumed_reply_into_business_effect() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(Some("after-consume-ack"));
    let question = f.question().await;
    approve(&f, &question, "before-revoke").await;
    exited(&mut app, 86).await;
    let before = connect_review_loop::connected_app::AppStore::inspect(&f.state()).unwrap();
    assert_eq!(before.reviews, 0);
    f.revoke_app_credential().await;
    app = f.spawn(None);
    exited(&mut app, 1).await;
    let after = connect_review_loop::connected_app::AppStore::inspect(&f.state()).unwrap();
    assert_eq!(after.reviews, 0);
    assert_eq!(after.pending_commands, before.pending_commands);
    assert_eq!(after.results.len(), 0);
    assert!(f
        .history("agent", &question.conversation_id)
        .await
        .iter()
        .all(|m| !m.body.contains("\"review_recorded\"")));
    f.finish(&mut app).await;
}
