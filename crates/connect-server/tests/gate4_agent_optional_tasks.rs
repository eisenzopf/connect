//! Optional work tracking and narrow delegation do not burden normal communication.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{tasks::*, ClientError, ConnectClient};
use connect_protocol::{Operation, Outcome};
use connect_store::Scope;
use serde_json::json;

fn task(outcome: Outcome) -> TaskRecord {
    let Outcome::Task(outcome) = outcome else {
        panic!("native task outcome expected")
    };
    let TaskOutcome::Record(record) = *outcome else {
        panic!("task record expected")
    };
    *record
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn optional_task_defaults_and_explicit_reply_work_without_invocation_grants() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut native = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let cid = native
        .create("optional-task-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let args = json!({"request_id":"optional-tracking","conversation_id":cid});
    let record = task(mcp::outcome(
        mcp::call(&client, "connect_task_create", args.clone()).await,
    ));
    assert_eq!(record.owner_id, f.agent.identity_id);
    assert_eq!(record.conversation_id, cid);
    assert_eq!(record.scope.recipients, vec![f.peer.identity_id.clone()]);
    assert!(record.scope.operations.contains(&TaskPermission::Send));
    assert!(record
        .scope
        .operations
        .contains(&TaskPermission::TaskCancel));
    assert!(record.scope.sms_enrollments.is_empty());
    assert!(record.expires_at <= f.expires_at);
    assert!(record.expires_at > chrono::Utc::now().timestamp() + 300);
    assert_eq!(
        task(mcp::outcome(
            mcp::call(&client, "connect_task_create", args).await
        )),
        record
    );
    native.negotiate_task(&record.id).await.unwrap();
    let question_args = json!({"request_id":"optional-question","task_id":record.id,
        "step_id":"question-step","conversation_id":cid,"message": connect_protocol::messages::MessageDraft::text_all("Can you confirm the Thursday review?"),
        "wait_for_reply":true});
    let question =
        mcp::outcome(mcp::call(&client, "connect_task_send", question_args.clone()).await);
    let Outcome::Task(question) = question else {
        panic!("tracked question")
    };
    let command = TaskCommand::Invoke {
        request_id: "optional-question".into(),
        task_id: record.id.clone(),
        step_id: "question-step".into(),
        operation: Box::new(Operation::Send {
            request_id: "optional-question".into(),
            conversation_id: cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all(
                "Can you confirm the Thursday review?",
            ),
        }),
        reply_to: None,
        wait_for_reply: true,
    };
    assert_eq!(native.task(command).await.unwrap(), *question);
    assert!(matches!(
        *question,
        TaskOutcome::Tool(TaskToolResult::Pending { .. })
    ));
    mcp::invalid(
        &client,
        "connect_task_create",
        json!({"request_id":"unpaired-scope",
        "conversation_id":cid,"operations":["send"]}),
    )
    .await;
    mcp::invalid(
        &client,
        "connect_task_create",
        json!({"request_id":"unpaired-recipient",
        "conversation_id":cid,"recipients":[f.peer.identity_id]}),
    )
    .await;
    let question_message = peer
        .history(&cid, None, 100)
        .await
        .unwrap()
        .messages
        .into_iter()
        .find(|message| message.body == "Can you confirm the Thursday review?")
        .unwrap();
    // The recipient needs ordinary send membership only, with no task privilege.
    let send_only =
        f.issue_as_operator(&f.peer, Scope::messaging(vec![f.agent.identity_id.clone()]));
    let recipient = mcp::client(f.server.mcp_address.unwrap(), &send_only.credential).await;
    let catalog = recipient.list_tools(None).await.unwrap();
    assert!(catalog
        .tools
        .iter()
        .any(|tool| tool.name == "connect_reply"));
    assert!(catalog
        .tools
        .iter()
        .all(|tool| tool.name != "connect_task_create"));
    let reply = json!({"request_id":"recipient-confirmation","conversation_id":cid,
        "reply_to":question_message.id,"body":"Thursday is confirmed."});
    let accepted = mcp::outcome(mcp::call(&recipient, "connect_reply", reply.clone()).await);
    let Outcome::Task(outcome) = &accepted else {
        panic!("reply outcome")
    };
    assert!(
        matches!(outcome.as_ref(), TaskOutcome::ReplyAccepted { reply, .. }
        if reply.task_id.as_deref() == Some(record.id.as_str()) && !reply.consumed)
    );
    assert_eq!(
        mcp::outcome(mcp::call(&recipient, "connect_reply", reply).await),
        accepted
    );
    let unknown = mcp::call(
        &recipient,
        "connect_reply",
        json!({"request_id":"unknown-reply",
        "conversation_id":cid,"reply_to":"not-a-delivered-question","body":"Do not assign."}),
    )
    .await;
    assert_eq!(mcp::error(unknown).code, "forbidden");
    let other = peer
        .create("other-reply-room", vec![f.agent.identity_id.clone()])
        .await
        .unwrap();
    assert_eq!(mcp::error(mcp::call(&recipient, "connect_reply", json!({
        "request_id":"wrong-room-reply","conversation_id":other.id,"reply_to":question_message.id,
        "body":"Do not assign across conversations."
    })).await).code, "forbidden");
    let waited = mcp::outcome(
        mcp::call(
            &client,
            "connect_task_wait",
            json!({
                "task_id":record.id,"limit":100,"wait_ms":0
            }),
        )
        .await,
    );
    let Outcome::Task(waited) = waited else {
        panic!("task wait outcome")
    };
    let TaskOutcome::Inbox { events, .. } = *waited else {
        panic!("task inbox")
    };
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(&event.data, TaskEventData::Reply(reply)
        if reply.reply_to.as_deref() == Some(question_message.id.as_str())))
            .count(),
        1
    );
    let cancel =
        json!({"request_id":"optional-cancel","task_id":record.id,"reason":"Stop tracked work"});
    assert_eq!(
        task(mcp::outcome(
            mcp::call(&client, "connect_task_cancel", cancel.clone()).await
        ))
        .state,
        TaskState::Cancelled
    );
    assert_eq!(
        task(mcp::outcome(
            mcp::call(&client, "connect_task_cancel", cancel).await
        ))
        .state,
        TaskState::Cancelled
    );
    // Optional task cancellation leaves ordinary conversation communication available.
    mcp::outcome(
        mcp::call(
            &client,
            "connect_send",
            json!({"request_id":"after-task-cancel",
                "conversation_id":cid,"message": connect_protocol::messages::MessageDraft::text_all("Continue communicating without tracked work.")}),
        )
        .await,
    );
    assert_eq!(agent::history(&mut peer, &cid).await.len(), 3);
    native.close().await;
    peer.close().await;
    recipient.cancel().await.unwrap();
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn operator_replaces_a_revoked_agent_key_for_retained_inspection_without_restoring_old_effects(
) {
    let f = agent::Fixture::start().await;
    let mut native = f.agent_client().await;
    let cid = native
        .create("operator-review-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    native
        .negotiate_tasks("connect:agent@local", Some(&cid))
        .await
        .unwrap();
    let TaskOutcome::Record(record) = native
        .task(TaskCommand::Create {
            request_id: "operator-review-task".into(),
            definition: TaskDefinition {
                conversation_id: cid.clone(),
                scope: TaskScope {
                    operations: vec![TaskPermission::Send, TaskPermission::History],
                    recipients: vec![f.peer.identity_id.clone()],
                    sms_enrollments: vec![],
                },
                expires_at: chrono::Utc::now().timestamp() + 600,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap()
    else {
        panic!("tracked task")
    };
    let effect_command = TaskCommand::Invoke {
        request_id: "inspectable-effect".into(),
        task_id: record.id.clone(),
        step_id: "inspectable-step".into(),
        operation: Box::new(Operation::Send {
            request_id: "inspectable-effect".into(),
            conversation_id: cid.clone(),
            message: connect_protocol::messages::MessageDraft::text_all(
                "Retain this agent activity after key revocation.",
            ),
        }),
        reply_to: None,
        wait_for_reply: false,
    };
    assert!(matches!(
        native.task(effect_command.clone()).await.unwrap(),
        TaskOutcome::Tool(TaskToolResult::Completed { .. })
    ));
    let TaskOutcome::Inbox { events, .. } = native
        .task(TaskCommand::Inbox {
            task_id: record.id.clone(),
            after: None,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap()
    else {
        panic!("effect journal")
    };
    let effect_id = events
        .iter()
        .find_map(|event| {
            if let TaskEventData::Effect(effect) = &event.data {
                Some(effect.id.clone())
            } else {
                None
            }
        })
        .unwrap();
    f.revoke(&f.agent.credential_id);
    assert!(
        matches!(native.task(TaskCommand::Get { task_id: record.id.clone() }).await,
        Err(ClientError::Application(error)) if error.code == "unauthorized")
    );
    // A different operator identity cannot bypass conversation or task membership.
    let mut operator_config = f.foreign.clone();
    operator_config.credential = f.operator_secret.clone();
    operator_config.credential_id = f.admin().credential_id.clone();
    let mut operator = ConnectClient::open(operator_config).await.unwrap();
    assert!(matches!(operator.history(&cid, None, 100).await,
        Err(ClientError::Application(error)) if error.code == "forbidden"));
    // Existing protected issuance lets the owner inspect as this separate agent identity.
    let inspection = f.issue_as_operator(&f.agent, agent::scope(vec![f.peer.identity_id.clone()]));
    assert_ne!(inspection.credential, f.agent.credential);
    assert!(!f
        .server
        .store
        .authenticate(&inspection.credential)
        .unwrap()
        .permits("admin"));
    let client = mcp::client(f.server.mcp_address.unwrap(), &inspection.credential).await;
    let retained = mcp::outcome(
        mcp::call(
            &client,
            "connect_history",
            json!({"conversation_id":cid,"limit":100}),
        )
        .await,
    );
    assert!(
        matches!(retained, Outcome::History { messages, .. } if messages.len() == 1
        && messages[0].sender == f.agent.identity_id
        && messages[0].body == "Retain this agent activity after key revocation.")
    );
    assert_eq!(
        task(mcp::outcome(
            mcp::call(&client, "connect_task_status", json!({"task_id":record.id})).await
        ))
        .id,
        record.id
    );
    let retained_effect = mcp::outcome(
        mcp::call(
            &client,
            "connect_effect_status",
            json!({
                "task_id":record.id,"effect_id":effect_id
            }),
        )
        .await,
    );
    let Outcome::Task(retained_effect) = retained_effect else {
        panic!("retained effect")
    };
    assert!(matches!(*retained_effect, TaskOutcome::Effect(effect)
        if effect.id == effect_id && effect.request_id == "inspectable-effect"
        && effect.outcome.is_some()));
    let mut replacement = ConnectClient::open(inspection).await.unwrap();
    replacement.negotiate_task(&record.id).await.unwrap();
    assert!(matches!(replacement.task(effect_command).await,
        Err(ClientError::Application(error)) if error.code == "unauthorized"));
    assert_eq!(task(mcp::outcome(mcp::call(&client, "connect_task_cancel", json!({
        "request_id":"operator-cancel","task_id":record.id,"reason":"Owner stopped the agent"
    })).await)).state, TaskState::Cancelled);
    assert_eq!(agent::history(&mut replacement, &cid).await.len(), 1);
    replacement.close().await;
    operator.close().await;
    native.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_narrow_mcp_grant_keeps_its_bound_schema_and_cannot_create_conversations() {
    let f = fixture::Fixture::start_independent_permitted(vec![
        TaskPermission::History,
        TaskPermission::Send,
    ])
    .await;
    let client = mcp::client(f.server.mcp_address.unwrap(), f.issued.expose_secret()).await;
    let tools = client.list_tools(None).await.unwrap();
    assert!(tools.tools.iter().any(|tool| tool.name == "connect_send"));
    assert!(tools.tools.iter().all(|tool| tool.name != "connect_create"));
    assert!(tools
        .tools
        .iter()
        .all(|tool| tool.name != "connect_task_consume_reply"));
    mcp::invalid(
        &client,
        "connect_task_consume_reply",
        json!({"request_id":"narrow-consume","task_id":f.task.id,
            "event_id":"not-a-current-reply","expected_revision":0}),
    )
    .await;
    mcp::invalid(
        &client,
        "connect_send",
        json!({"request_id":"not-narrow","message": connect_protocol::messages::MessageDraft::text_all("Missing step")}),
    )
    .await;
    mcp::invalid(
        &client,
        "connect_history",
        json!({"conversation_id":"foreign-selector","limit":100}),
    )
    .await;
    let args = json!({"request_id":"explicit-narrow-send","step_id":"explicit-step","message": connect_protocol::messages::MessageDraft::text_all("Narrow mode remains available.")});
    let result = mcp::call(&client, "connect_send", args.clone()).await;
    assert_ne!(result.is_error, Some(true));
    let retained: TaskOutcome = serde_json::from_value(result.structured_content.unwrap()).unwrap();
    assert!(
        matches!(retained, TaskOutcome::Tool(TaskToolResult::Completed { task_id, .. }) if task_id == f.task.id)
    );
    let replay = mcp::call(&client, "connect_send", args).await;
    assert_ne!(replay.is_error, Some(true));
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    assert_eq!(
        agent::history(&mut peer, &f.task.conversation_id)
            .await
            .len(),
        1
    );
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}
