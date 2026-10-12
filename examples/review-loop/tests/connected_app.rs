//! Packaged two-way business journey, duplicate handling and crash recovery.
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
fn draft() -> Draft {
    Draft {
        id: "incoming-01".into(),
        title: "Community update".into(),
        body: "A real draft submitted by another app.".into(),
    }
}
fn submission(request: &str) -> String {
    serde_json::to_string(&Request::SubmitDraft {
        request_id: request.into(),
        draft: draft(),
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
        panic!("create")
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
async fn approve(f: &Fixture, question: &connect_client::Message, id: &str) {
    f.request(
        "agent",
        task(TaskCommand::Reply {
            request_id: id.into(),
            conversation_id: question.conversation_id.clone(),
            reply_to: question.id.clone(),
            body: serde_json::to_string(&Request::ReviewDecision {
                draft_id: "draft-002".into(),
                decision: Decision::Approved,
                note: "Ready for publication.".into(),
            })
            .unwrap(),
        }),
    )
    .await;
}
#[tokio::test]
async fn two_way_business_policy_binding_and_semantic_duplicates() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None);
    let question = f.question().await;
    let q: serde_json::Value = serde_json::from_str(&question.body).unwrap();
    assert_eq!(q["draft"]["id"], "draft-002");
    assert!(!q["draft"]["body"].as_str().unwrap().is_empty());
    // Plain text shaped like a decision does not select a task.
    let fake = serde_json::to_string(&Request::ReviewDecision {
        draft_id: "draft-002".into(),
        decision: Decision::Approved,
        note: "Unbound".into(),
    })
    .unwrap();
    send(&f, "agent", &question.conversation_id, "unbound", fake).await;
    let incoming = create(&f, "agent", "independent-contact").await;
    send(&f, "agent", &incoming, "submit-a", submission("intake-1")).await;
    approve(&f, &question, "approve-1").await;
    let first = f.settled(1, 1).await;
    assert_eq!(first.unassigned_events, 1);
    assert_eq!(f.business_payload("review")["decision"], "approved");
    assert_eq!(f.business_payload("intake")["draft"]["body"], draft().body);
    assert_eq!(f.business_payload("intake")["status"], "received");
    assert_eq!(
        first
            .results
            .iter()
            .find(|r| r.kind == "intake")
            .unwrap()
            .conversation_id,
        incoming
    );
    assert_eq!(
        first
            .results
            .iter()
            .find(|r| r.kind == "review")
            .unwrap()
            .conversation_id,
        question.conversation_id
    );
    // Same semantic submission under another wire ID is one ticket/response.
    send(&f, "agent", &incoming, "submit-b", submission("intake-1")).await;
    approve(&f, &question, "approve-2").await;
    let outside = create(&f, "outside", "denied-contact").await;
    send(
        &f,
        "outside",
        &outside,
        "outside-submit",
        submission("not-allowed"),
    )
    .await;
    let mut changed = draft();
    changed.body = "Changed input under the original semantic ID".into();
    send(
        &f,
        "agent",
        &incoming,
        "submit-c",
        serde_json::to_string(&Request::SubmitDraft {
            request_id: "intake-1".into(),
            draft: changed,
        })
        .unwrap(),
    )
    .await;
    // Fence subsequent work with a server read and wait until all distinct messages are observed.
    let stats = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let s = f.settled(1, 1).await;
            if s.denied_events == 1 && s.conflict_events == 1 {
                break s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(stats.results.len(), 2);
    assert_eq!(
        f.history("agent", &incoming)
            .await
            .iter()
            .filter(|m| m.body.contains("\"draft_received\""))
            .count(),
        1
    );
    assert_eq!(
        f.history("agent", &question.conversation_id)
            .await
            .iter()
            .filter(|m| m.body.contains("\"review_recorded\""))
            .count(),
        1
    );
    assert!(f
        .history("outside", &outside)
        .await
        .iter()
        .all(|m| m.sender == f.identity("outside")));
    f.finish(&mut app).await;
}
#[tokio::test]
async fn intake_ack_and_local_transaction_crashes_recover_original_ticket() {
    for fault in [
        "after-intake-task-ack",
        "after-business",
        "after-business-commit",
        "after-response-ack",
    ] {
        let mut f = Fixture::start().await;
        let mut app = f.spawn(Some(fault));
        let _ = f.question().await;
        let cid = create(&f, "agent", "incoming").await;
        send(&f, "agent", &cid, "submit", submission("recover-1")).await;
        exited(&mut app, 86).await;
        f.restart_server().await;
        app = f.spawn(None);
        let stats = f.settled(0, 1).await;
        assert_eq!(stats.results.len(), 1);
        assert_eq!(stats.results[0].conversation_id, cid);
        assert_eq!(
            f.business_payload("intake")["draft"],
            serde_json::to_value(draft()).unwrap()
        );
        send(&f, "agent", &cid, "retry", submission("recover-1")).await;
        let _ = f.settled(0, 1).await;
        assert_eq!(
            f.history("agent", &cid)
                .await
                .iter()
                .filter(|m| m.body.contains("\"draft_received\""))
                .count(),
            1
        );
        f.finish(&mut app).await;
    }
}
#[tokio::test]
async fn consumed_reply_ack_crash_resumes_one_review_and_response() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(Some("after-consume-ack"));
    let question = f.question().await;
    approve(&f, &question, "approve").await;
    exited(&mut app, 86).await;
    assert_eq!(
        connect_review_loop::connected_app::AppStore::inspect(&f.state())
            .unwrap()
            .reviews,
        0
    );
    f.restart_server().await;
    app = f.spawn(None);
    let _ = f.settled(1, 0).await;
    assert_eq!(f.business_payload("review")["decision"], "approved");
    let c = rusqlite::Connection::open_with_flags(
        f.state(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let original: String = c
        .query_row(
            "SELECT outcome FROM app_commands WHERE key='outgoing-question'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&original).unwrap()["data"]["data"]["status"],
        "pending"
    );
    let observations: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM app_observations WHERE command_key='outgoing-question'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(observations, 1);
    drop(c);
    assert_eq!(
        f.history("agent", &question.conversation_id)
            .await
            .iter()
            .filter(|m| m.body.contains("\"review_recorded\""))
            .count(),
        1
    );
    f.finish(&mut app).await;
}
