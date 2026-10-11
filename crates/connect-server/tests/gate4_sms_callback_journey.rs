//! Signed SMS controls and ambiguous replies retain task authority after restart.
#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/signed_sms.rs"]
mod signed_sms;
#[allow(dead_code)] // Shared enrollment helpers also exercise route/lease failure cases.
#[path = "support/task_sms.rs"]
mod task_sms;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Telnyx;
use connect_protocol::Outcome;
use native_tasks::{record, Harness};
use serde_json::json;
use signed_sms::{incoming, Callbacks};
use std::sync::Arc;
use task_sms::{dispatch, question, queue, Enrollment, PHONE, SENDER};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

async fn replies(h: &mut Harness) -> Vec<TaskReply> {
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
    items
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn signed_controls_and_late_sms_reply_preserve_one_task_without_reviving_cancelled_sends() {
    let _key = signed_sms::FixtureKey::install();
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(body_partial_json(
            json!({"from":SENDER,"to":PHONE,"text":"Requested draft review: fresh"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"review-sms"}})))
        .expect(1)
        .mount(&mock)
        .await;
    let driver = Arc::new(Telnyx::new("offline-fixture", &mock.uri()).unwrap());
    let mut h = Harness::start().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let enrollment = Enrollment::install(&h);
    let mut definition = h.definition();
    definition
        .scope
        .operations
        .extend([TaskPermission::Inbox, TaskPermission::RequestStatus]);
    definition.scope.sms_enrollments = vec![enrollment.reference()];
    let task = record(
        h.owner
            .task(TaskCommand::Create {
                request_id: "sms-journey".into(),
                definition,
            })
            .await
            .unwrap(),
    );
    let sibling = h.create("second-work-same-conversation").await;
    let permissions = task.scope.operations.clone();
    let grant = grants::issue(
        &h,
        &task,
        "sms-journey-grant",
        permissions,
        chrono::Utc::now().timestamp() + 300,
    )
    .await;
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let callbacks = Callbacks::start(&h.server).await;
    let now = chrono::Utc::now().timestamp();
    let blocked = question(&task, &h.cid, "before-stop");
    queue(&mut client, blocked.clone()).await;
    let stop = incoming("stop", "STOP", now, PHONE, SENDER);
    let before = connect_store::Store::inspect_snapshot(h.server.store.path()).unwrap();
    assert_eq!(
        callbacks.post(stop.clone(), true).await,
        reqwest::StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(h.server.store.path()).unwrap(),
        before
    );
    callbacks.accepted(stop.clone()).await;
    callbacks.accepted(stop.clone()).await;
    callbacks
        .accepted(incoming("help", "HELP", now + 1, PHONE, SENDER))
        .await;
    let jobs = dispatch(&mut h, driver.clone(), 1).await;
    assert_eq!(jobs[0].state, "cancelled");
    assert_eq!(jobs[0].sms_decision.as_deref(), Some("opted-out"));
    assert!(
        mock.received_requests().await.unwrap().is_empty(),
        "Connect must not send provider-owned keyword acknowledgements"
    );
    assert!(
        replies(&mut h).await.is_empty(),
        "Controls cannot become task replies"
    );
    client.close().await;
    drop(callbacks);
    h = h.restart().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let callbacks = Callbacks::start(&h.server).await;
    callbacks
        .accepted(incoming("start", "START", now + 2, PHONE, SENDER))
        .await;
    callbacks.accepted(stop).await; // A delayed duplicate STOP cannot undo newer START.
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    assert_eq!(
        grants::code(client.task(blocked.clone()).await.unwrap_err()),
        "provider-failed"
    );
    let mut fresh = question(&task, &h.cid, "fresh");
    if let TaskCommand::Invoke { wait_for_reply, .. } = &mut fresh {
        *wait_for_reply = true;
    }
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect_id),
        ..
    }) = client.task(fresh.clone()).await.unwrap()
    else {
        panic!("pending question")
    };
    let jobs = dispatch(&mut h, driver.clone(), 2).await;
    assert_eq!(jobs.iter().filter(|j| j.state == "accepted").count(), 1);
    let delivery = json!({"id":"delivery", "event_type":"message.finalized", "payload":{"id":"review-sms", "to":[{"status":"delivered"}]}});
    callbacks.accepted(delivery.clone()).await;
    callbacks.accepted(delivery).await;
    assert!(
        matches!(
            client.task(fresh.clone()).await.unwrap(),
            TaskOutcome::Tool(TaskToolResult::Pending { .. })
        ),
        "Delivery is not the requested human answer"
    );
    let TaskOutcome::Effect(effect) = client
        .task(TaskCommand::EffectGet {
            task_id: task.id.clone(),
            effect_id,
        })
        .await
        .unwrap()
    else {
        panic!("effect")
    };
    let Outcome::Accepted {
        message_id: question_id,
        ..
    } = *effect.outcome.unwrap()
    else {
        panic!("question receipt")
    };
    let answer = incoming(
        "answer",
        "Approve sapphire-nine for the requested draft.",
        now + 3,
        PHONE,
        SENDER,
    );
    callbacks.accepted(answer.clone()).await;
    callbacks.accepted(answer).await;
    let unassigned = replies(&mut h).await;
    assert_eq!(unassigned.len(), 1);
    assert!(
        unassigned[0].task_id.is_none(),
        "A phone number cannot choose between tasks"
    );
    let assignment = TaskCommand::AssignReply {
        request_id: "assign-sms-answer".into(),
        task_id: task.id.clone(),
        event_id: unassigned[0].event_id.clone(),
        reply_to: Some(question_id),
    };
    let accepted = h.owner.task(assignment.clone()).await.unwrap();
    client.close().await;
    drop(callbacks);
    h = h.restart().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    assert_eq!(h.owner.task(assignment).await.unwrap(), accepted);
    let assigned = replies(&mut h).await;
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].task_id.as_deref(), Some(task.id.as_str()));
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    let TaskOutcome::Recall { sources, .. } = client
        .task(TaskCommand::Recall {
            task_id: task.id.clone(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("sources")
    };
    let source = sources
        .iter()
        .find(|s| s.body.as_deref() == Some("Approve sapphire-nine for the requested draft."))
        .unwrap();
    assert_eq!(source.state, "available");
    assert!(source.provenance.contains("conversation-journal:"));
    assert_eq!(
        record(
            h.owner
                .task(TaskCommand::Get {
                    task_id: sibling.id
                })
                .await
                .unwrap()
        )
        .state,
        sibling.state
    );
    assert!(matches!(
        client.task(fresh.clone()).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    assert_eq!(
        grants::code(client.task(blocked).await.unwrap_err()),
        "provider-failed"
    );
    h.owner
        .task(TaskCommand::Cancel {
            request_id: "end-review".into(),
            task_id: task.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    let callbacks = Callbacks::start(&h.server).await;
    callbacks
        .accepted(incoming("start-again", "START", now + 4, PHONE, SENDER))
        .await;
    assert_eq!(
        grants::code(client.task(fresh).await.unwrap_err()),
        "forbidden",
        "START cannot revive cancelled task replay"
    );
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        h.owner
            .provider_status(&h.cid)
            .await
            .unwrap()
            .iter()
            .filter(|j| j.state == "delivered")
            .count(),
        1
    );
    client.close().await;
    drop(callbacks);
    h.close().await;
}
