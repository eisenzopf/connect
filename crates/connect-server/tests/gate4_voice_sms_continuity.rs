//! One task carries a sourced voice decision into reviewed SMS after restart.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/voice_sms_fixture.rs"]
mod journey;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/signed_sms.rs"]
mod signed_sms;
#[allow(dead_code)]
#[path = "support/task_sms.rs"]
mod task_sms;
#[path = "support/voice_decision.rs"]
mod voice_decision;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Telnyx;
use connect_protocol::Outcome;
use journey::{followup, replies};
use native_tasks::record;
use serde_json::json;
use signed_sms::{incoming, Callbacks};
use std::sync::Arc;
use task_sms::{dispatch, PHONE, SENDER};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_decision_survives_restart_and_stop_then_one_sms_reply_returns_to_its_task() {
    let _key = signed_sms::FixtureKey::install();
    let mut fake = gate3_support::FakeVapi::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    let (enrollment, sibling) = journey::prepare(&mut f).await;
    let draft = journey::draft(&mut f).await;
    let directory = tempfile::tempdir().unwrap();
    let recorded = voice_decision::capture(
        &mut f,
        &mut fake,
        &directory.path().join("voice"),
        &draft.id,
    )
    .await;
    f.h.owner
        .artifact_delete("delete-obsolete-draft", &f.h.cid, &draft.id)
        .await
        .unwrap();
    let media::Fixture {
        h,
        task,
        peer_config: _,
    } = f;
    let mut h = h.restart().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    let grant = grants::issue(
        &h,
        &task,
        "post-voice-sms",
        vec![
            TaskPermission::Send,
            TaskPermission::History,
            TaskPermission::Inbox,
            TaskPermission::RequestStatus,
        ],
        chrono::Utc::now().timestamp() + 300,
    )
    .await;
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
        panic!("post-call recall")
    };
    let source = sources
        .iter()
        .find(|s| s.source_id == recorded.source_id && s.body == recorded.body)
        .unwrap();
    assert_eq!(source, &recorded);
    journey::unavailable(&sources, &draft.id);
    assert_eq!(
        grants::code(
            h.owner
                .artifact_get(&h.cid, &draft.id, 0, 32768)
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    assert!(!serde_json::to_string(&sources)
        .unwrap()
        .contains(voice_decision::CONTRAST));
    let body = format!(
        "Following voice source {}: {} Please confirm this draft decision.",
        source.source_id,
        source.body.as_ref().unwrap()
    );
    assert!(body.contains("sapphire-nine"));
    enrollment.enabled(&h, true);
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(body_partial_json(
            json!({"from":SENDER,"to":PHONE,"text":body}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"post-voice-sms"}})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let driver = Arc::new(Telnyx::new("offline-fixture", &mock.uri()).unwrap());
    let callbacks = Callbacks::start(&h.server).await;
    let now = chrono::Utc::now().timestamp();
    let blocked = followup(&task, "voice-followup-before-stop", &body);
    task_sms::queue(&mut client, blocked.clone()).await;
    callbacks
        .accepted(incoming("post-voice-stop", "STOP", now, PHONE, SENDER))
        .await;
    let jobs = dispatch(&mut h, driver.clone(), 1).await;
    assert_eq!(jobs[0].sms_decision.as_deref(), Some("opted-out"));
    assert_eq!(jobs[0].state, "cancelled");
    callbacks
        .accepted(incoming(
            "post-voice-start",
            "START",
            now + 1,
            PHONE,
            SENDER,
        ))
        .await;
    assert_eq!(
        grants::code(client.task(blocked.clone()).await.unwrap_err()),
        "provider-failed"
    );
    let fresh = followup(&task, "voice-followup-after-start", &body);
    let TaskOutcome::Tool(TaskToolResult::Pending {
        effect_id: Some(effect_id),
        ..
    }) = client.task(fresh.clone()).await.unwrap()
    else {
        panic!("pending SMS")
    };
    let jobs = dispatch(&mut h, driver.clone(), 2).await;
    assert_eq!(jobs.iter().filter(|j| j.state == "accepted").count(), 1);
    callbacks.accepted(json!({"id":"post-voice-delivered","event_type":"message.finalized","payload":{"id":"post-voice-sms","to":[{"status":"delivered"}]}})).await;
    assert!(matches!(
        client.task(fresh.clone()).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Pending { .. })
    ));
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
        message_id: question,
        ..
    } = *effect.outcome.unwrap()
    else {
        panic!("question receipt")
    };
    let before_reply = replies(&mut h.owner, &h.cid).await;
    let answer = incoming(
        "post-voice-answer",
        "Confirmed sapphire-nine from our call.",
        now + 2,
        PHONE,
        SENDER,
    );
    callbacks.accepted(answer.clone()).await;
    callbacks.accepted(answer).await;
    let incoming = replies(&mut h.owner, &h.cid).await;
    // Voice observations are context, not implicit answers to an SMS question.
    let new_replies: Vec<_> = incoming
        .iter()
        .filter(|r| !before_reply.iter().any(|old| old.event_id == r.event_id))
        .collect();
    assert_eq!(new_replies.len(), 1, "Duplicate SMS yields one new reply");
    let answer = new_replies[0];
    assert!(
        answer.task_id.is_none(),
        "Voice or phone identity cannot assign the SMS reply"
    );
    let assign = TaskCommand::AssignReply {
        request_id: "bind-post-voice-answer".into(),
        task_id: task.id.clone(),
        event_id: answer.event_id.clone(),
        reply_to: Some(question),
    };
    h.owner.task(assign.clone()).await.unwrap();
    client.close().await;
    drop(callbacks);
    h = h.restart().await;
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    h.owner.task(assign).await.unwrap();
    let mut client = ConnectClient::open(h.owner_config.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&task.id).await.unwrap();
    assert!(matches!(
        client.task(fresh.clone()).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    assert!(matches!(
        client.task(fresh).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    assert_eq!(
        grants::code(client.task(blocked).await.unwrap_err()),
        "provider-failed"
    );
    let TaskOutcome::Recall { sources, .. } = client
        .task(TaskCommand::Recall {
            task_id: task.id.clone(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("final recall")
    };
    assert!(sources.iter().any(|s| s == &recorded));
    journey::unavailable(&sources, &draft.id);
    assert!(sources.iter().any(|s| s.body.as_deref()
        == Some("Confirmed sapphire-nine from our call.")
        && s.state == "available"));
    assert!(!serde_json::to_string(&sources)
        .unwrap()
        .contains(voice_decision::CONTRAST));
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
    assert_eq!(
        replies(&mut h.owner, &h.cid)
            .await
            .iter()
            .filter(|r| r.task_id.as_deref() == Some(task.id.as_str()))
            .count(),
        1
    );
    assert_eq!(
        h.peer
            .history(&h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.body == body)
            .count(),
        2,
        "One cancelled intent and one fresh intent remain in conversation history"
    );
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
    fake.assert_no_message("create-call").await;
    client.close().await;
    h.close().await;
}
