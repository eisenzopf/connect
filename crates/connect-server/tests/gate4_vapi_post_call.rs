//! A bounded voice leg ends; its task resumes in chat after a real server restart.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_task_runtime.rs"]
mod api;
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
#[path = "support/vapi_task_tools.rs"]
mod tools;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use connect_protocol::Outcome;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn voice_question_and_decision_survive_restart_then_chat_replies_once_in_same_task() {
    let mut fake = gate3_support::FakeVapi::start().await;
    let sdk = Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap());
    let api = api::Fixture::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    let mut scope = f.task.scope.clone();
    scope
        .operations
        .extend([TaskPermission::Inbox, TaskPermission::RequestStatus]);
    f.task = native_tasks::record(
        f.h.owner
            .task(TaskCommand::Create {
                request_id: "voice-continuation-task".into(),
                definition: TaskDefinition {
                    conversation_id: f.h.cid.clone(),
                    scope,
                    expires_at: f.task.expires_at,
                    initiating_event_id: None,
                },
            })
            .await
            .unwrap(),
    );
    let sibling = f.h.create("unrelated-task-in-same-conversation").await;
    call_setup::route(&f);
    f.h.owner
        .send(
            "old-goal",
            &f.h.cid,
            "Earlier draft goal: choose a release color.",
        )
        .await
        .unwrap();
    let (_, config) = f
        .grant("continuation-native", 120, media::Fixture::permissions())
        .await;
    let mut native = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut native, "continuation-voice").await;
    let temp = tempfile::tempdir().unwrap();
    let mut definition = call_setup::definition(&f);
    definition
        .operations
        .extend([TaskPermission::Inbox, TaskPermission::RequestStatus]);
    let address =
        f.h.server
            .prepare_vapi_task_session(&session.id, definition, &temp.path().join("voice"), sdk)
            .await
            .unwrap();
    let mut audio = native
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
        .unwrap();
    let post = fake.next("create-call").await;
    let voice = call_setup::mcp(&post["body"]).await;
    let recalled =
        tools::sources(tools::call(&voice, "connect_recall", json!({"limit":100})).await);
    assert!(recalled
        .iter()
        .any(|s| s.body.as_deref() == Some("Earlier draft goal: choose a release color.")));
    let question_args = json!({
        "request_id":"voice-question", "step_id":"approval",
        "body":"Which channel should I use after the call?", "wait_for_reply":true
    });
    let TaskOutcome::Tool(TaskToolResult::Pending {
        task_id,
        effect_id: Some(effect_id),
    }) = tools::outcome(tools::call(&voice, "connect_send", question_args).await)
    else {
        panic!("an unanswered question must remain pending")
    };
    assert_eq!(task_id, f.task.id);
    let (events, cursor) = tools::inbox(
        tools::call(
            &voice,
            "connect_task_wait",
            json!({"limit":100,"wait_ms":0}),
        )
        .await,
    );
    // Catch-up exposes bounded metadata. An explicitly permitted status read
    // retrieves the committed send receipt; its requested answer is still pending.
    assert!(events.iter().any(|event| matches!(&event.data,
        TaskEventData::Effect(effect) if effect.id == effect_id
            && effect.state == EffectState::Succeeded && effect.outcome.is_none()
    )));
    let TaskOutcome::Effect(effect) = tools::outcome(
        tools::call(
            &voice,
            "connect_effect_status",
            json!({"effect_id":effect_id}),
        )
        .await,
    ) else {
        panic!("effect status")
    };
    assert_eq!(effect.task_id, f.task.id);
    assert_eq!(effect.state, EffectState::Succeeded);
    let Outcome::Accepted {
        message_id: question,
        ..
    } = *effect.outcome.unwrap()
    else {
        panic!("persisted question receipt")
    };
    assert!(cursor.is_some());
    let pending = tools::call(
        &voice,
        "connect_task_wait",
        json!({"after":cursor,"limit":100,"wait_ms":300}),
    )
    .await;
    assert_eq!(pending.meta.as_ref().unwrap()["connect"]["pending"], true);
    assert!(tools::inbox(pending).0.is_empty());
    fake.next("add-message").await;
    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    fake.next("audio").await;
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = f.h.owner.context_view(&f.h.cid, 100).await.unwrap();
            if view
                .items
                .iter()
                .any(|s| s.kind == "voice_outcome" && s.source_id.contains(&session.id))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // The provider ended the call. Its MCP authority cannot outlive that leg.
    // Owner inspection captures the retained journal projection for comparison
    // with a fresh, explicitly bound chat invocation after restart.
    let TaskOutcome::Recall { sources, .. } =
        f.h.owner
            .task(TaskCommand::Recall {
                task_id: f.task.id.clone(),
                after: None,
                limit: 100,
            })
            .await
            .unwrap()
    else {
        panic!("owner source inspection")
    };
    let decision = sources
        .iter()
        .find(|s| {
            s.body
                .as_deref()
                .is_some_and(|b| b.contains("sapphire-nine"))
        })
        .unwrap()
        .clone();
    assert!(decision.provenance.contains("conversation-journal:"));
    assert!(!serde_json::to_string(&sources)
        .unwrap()
        .contains("must-not-enter-context"));
    native.close_native_audio(&audio).await.unwrap();
    native.close().await;
    voice.cancel().await.unwrap();
    let media::Fixture {
        h,
        task,
        peer_config: _,
    } = f;
    let mut h = h.restart().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    h.owner
        .negotiate_tasks("connect:agent@local", Some(&h.cid))
        .await
        .unwrap();
    h.peer
        .negotiate_tasks("connect:reviews@local", Some(&h.cid))
        .await
        .unwrap();
    let waiting = native_tasks::record(
        h.owner
            .task(TaskCommand::Get {
                task_id: task.id.clone(),
            })
            .await
            .unwrap(),
    );
    assert_eq!(waiting.state, TaskState::Waiting);
    assert_eq!(waiting.conversation_id, h.cid);
    // An explicit recipient reply finds only the persisted question's task.
    let reply = h
        .peer
        .task(TaskCommand::Reply {
            request_id: "late-human-reply".into(),
            conversation_id: h.cid.clone(),
            reply_to: question.clone(),
            body: "Continue by message with a blue draft.".into(),
        })
        .await
        .unwrap();
    let TaskOutcome::ReplyAccepted {
        message_id: reply_message,
        reply,
        ..
    } = reply
    else {
        panic!("reply")
    };
    assert_eq!(
        native_tasks::record(
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
    let reply_id = reply.event_id;
    let endpoint = tools::Endpoint::start(
        &h.server,
        &h.owner_config,
        &task,
        &api,
        &temp.path().join("chat"),
        vec![
            TaskPermission::History,
            TaskPermission::Inbox,
            TaskPermission::Send,
        ],
    )
    .await;
    let chat = endpoint.client().await;
    assert!(chat
        .list_tools(None)
        .await
        .unwrap()
        .tools
        .iter()
        .all(|tool| {
            !tool.name.starts_with("connect_session_")
                && !tool.name.starts_with("connect_call_")
                && !tool.name.starts_with("connect_sip_")
        }));
    let (events, next) = tools::inbox(
        tools::call(
            &chat,
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
    assert_eq!(replies[0].event_id, reply_id);
    assert_eq!(replies[0].task_id.as_deref(), Some(task.id.as_str()));
    assert_eq!(replies[0].reply_to.as_deref(), Some(question.as_str()));
    assert!(tools::inbox(
        tools::call(
            &chat,
            "connect_task_wait",
            json!({"after":next,"limit":100,"wait_ms":0})
        )
        .await
    )
    .0
    .is_empty());
    let sources = tools::sources(tools::call(&chat, "connect_recall", json!({"limit":100})).await);
    assert!(sources.iter().any(|s| s == &decision));
    let answered = sources
        .iter()
        .find(|s| s.source_id == reply_message)
        .unwrap();
    assert_eq!(answered.state, "available");
    assert!(answered.provenance.contains("conversation-journal:"));
    let body = format!(
        "{} {}",
        decision.body.as_ref().unwrap(),
        answered.body.as_ref().unwrap()
    );
    let args = json!({"request_id":"post-call-followup","step_id":"draft","body":body,"reply_to":reply_id});
    let first = tools::outcome(tools::call(&chat, "connect_send", args.clone()).await);
    assert_eq!(
        tools::outcome(tools::call(&chat, "connect_send", args).await),
        first
    );
    let TaskOutcome::Tool(TaskToolResult::Completed { task_id, .. }) = first else {
        panic!("follow-up")
    };
    assert_eq!(task_id, task.id);
    assert_eq!(
        h.peer
            .history(&h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.body == body)
            .count(),
        1
    );
    assert_eq!(
        native_tasks::record(
            h.owner
                .task(TaskCommand::Get { task_id: task.id })
                .await
                .unwrap()
        )
        .conversation_id,
        h.cid
    );
    fake.assert_no_message("create-call").await;
    chat.cancel().await.unwrap();
    endpoint.close().await;
    h.close().await;
}
