//! Normal MCP can explicitly release reply capacity using existing owner CAS.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
#[path = "support/agent_task_fixture.rs"]
mod tracked;
use connect_client::tasks::*;
use connect_store::Scope;
use serde_json::{json, Value};

async fn reply(client: &mcp::Client, task: &TaskRecord, question: &str, index: u32) -> Value {
    mcp::call(client, "connect_reply", reply_args(task, question, index))
        .await
        .structured_content
        .unwrap()
}
fn reply_args(task: &TaskRecord, question: &str, index: u32) -> Value {
    json!({"request_id":format!("reply-{index}"),"conversation_id":task.conversation_id,
        "reply_to":question,"body":format!("Review answer {index}")})
}
fn accepted(value: Value) -> TaskReply {
    let TaskOutcome::ReplyAccepted { reply, .. } =
        tracked::task(serde_json::from_value(value).unwrap())
    else {
        panic!("accepted attributed reply")
    };
    reply
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_mcp_consumption_frees_capacity_and_replays_once_across_restart() {
    let f = agent::Fixture::start().await;
    let owner = tracked::owner(&f);
    let mut native = f.agent_client().await;
    let cid = native
        .create("consume-capacity-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let client = mcp::client(f.server.mcp_address.unwrap(), &owner.credential).await;
    let peer = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let task = tracked::create(&client, &cid, "consume-capacity-task").await;
    let question = tracked::question(&client, &task, "consume-capacity-question").await;
    let first = accepted(reply(&peer, &task, &question, 0).await);
    assert!(!first.consumed);
    mcp::outcome(
        mcp::call(
            &client,
            "connect_task_send",
            json!({"task_id":task.id,
        "request_id":"consume-followup","step_id":"consume-followup","conversation_id":cid,
        "message": connect_protocol::messages::MessageDraft::text_all(("I have handled this answer.").to_string()),"reply_to":first.event_id}),
        )
        .await,
    );
    for index in 1..MAX_TASK_INBOX_EVENTS {
        assert_eq!(
            accepted(reply(&peer, &task, &question, index).await)
                .task_id
                .as_deref(),
            Some(task.id.as_str())
        );
    }
    let journal = tracked::journal(&client, &task).await;
    assert_eq!(
        journal
            .iter()
            .filter(|event| matches!(&event.data,
        TaskEventData::Reply(reply) if !reply.consumed))
            .count(),
        MAX_TASK_INBOX_EVENTS as usize
    );
    let overflow = mcp::call(
        &peer,
        "connect_reply",
        reply_args(&task, &question, MAX_TASK_INBOX_EVENTS),
    )
    .await;
    assert_eq!(mcp::error(overflow).code, "capacity-exceeded");

    let current = tracked::status(&client, &task).await;
    let stale = json!({"request_id":"consume-stale-revision","task_id":task.id,
        "event_id":first.event_id,"expected_revision":0});
    assert_eq!(
        mcp::error(mcp::call(&client, tracked::CONSUME, stale).await).code,
        "task-state-conflict"
    );
    assert_eq!(tracked::status(&client, &task).await, current);
    let args = json!({"request_id":"consume-one-reply","task_id":task.id,
        "event_id":first.event_id,"expected_revision":current.revision});
    let consumed = mcp::outcome(mcp::call(&client, tracked::CONSUME, args.clone()).await);
    assert_eq!(
        tracked::record(consumed.clone()).revision,
        current.revision + 1
    );
    let admitted = reply(&peer, &task, &question, MAX_TASK_INBOX_EVENTS).await;
    assert_eq!(
        accepted(admitted.clone()).task_id.as_deref(),
        Some(task.id.as_str())
    );
    assert_eq!(
        mcp::outcome(mcp::call(&client, tracked::CONSUME, args.clone()).await),
        consumed
    );
    let mut conflict = args.clone();
    conflict["event_id"] = json!(accepted(admitted.clone()).event_id);
    assert_eq!(
        mcp::error(mcp::call(&client, tracked::CONSUME, conflict).await).code,
        "request-conflict"
    );
    let live = tracked::status(&client, &task).await;
    assert_eq!(
        mcp::error(
            mcp::call(
                &client,
                tracked::CONSUME,
                json!({
        "request_id":"consume-again","task_id":task.id,"event_id":first.event_id,
        "expected_revision":live.revision})
            )
            .await
        )
        .code,
        "reply-consumed"
    );

    native.close().await;
    peer.cancel().await.unwrap();
    client.cancel().await.unwrap();
    let f = f.restart().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &owner.credential).await;
    let peer = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    assert_eq!(
        mcp::outcome(mcp::call(&client, tracked::CONSUME, args.clone()).await),
        consumed
    );
    assert_eq!(
        reply(&peer, &task, &question, MAX_TASK_INBOX_EVENTS).await,
        admitted
    );
    let journal = tracked::journal(&client, &task).await;
    assert_eq!(
        journal
            .iter()
            .filter(|event| matches!(&event.data,
        TaskEventData::Reply(reply) if reply.consumed && reply.event_id == first.event_id))
            .count(),
        1
    );
    tracked::record(mcp::outcome(
        mcp::call(
            &client,
            "connect_task_cancel",
            json!({"task_id":task.id,"request_id":"consume-task-cancel"}),
        )
        .await,
    ));
    assert_eq!(
        mcp::error(mcp::call(&client, tracked::CONSUME, args).await).code,
        "forbidden"
    );
    peer.cancel().await.unwrap();
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_consumption_enforces_closed_schema_current_owner_scope_and_task_binding() {
    let f = agent::Fixture::start().await;
    let owner = tracked::owner(&f);
    let mut native = f.agent_client().await;
    let cid = native
        .create("consume-auth-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let client = mcp::client(f.server.mcp_address.unwrap(), &owner.credential).await;
    let peer = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let task = tracked::create(&client, &cid, "consume-auth-task").await;
    let question = tracked::question(&client, &task, "consume-auth-question").await;
    let first = accepted(reply(&peer, &task, &question, 0).await);
    let current = tracked::status(&client, &task).await;
    let args = json!({"request_id":"consume-auth-reply","task_id":task.id,
        "event_id":first.event_id,"expected_revision":current.revision});

    for (name, value) in [
        ("actor_id", json!(f.agent.identity_id)),
        ("credential_id", json!(owner.credential_id)),
    ] {
        let mut forged = args.clone();
        forged[name] = value;
        mcp::invalid(&client, tracked::CONSUME, forged).await;
    }
    for name in ["request_id", "task_id", "event_id", "expected_revision"] {
        let mut missing = args.clone();
        missing.as_object_mut().unwrap().remove(name);
        mcp::invalid(&client, tracked::CONSUME, missing).await;
    }
    let no_scope = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    assert!(no_scope
        .list_tools(None)
        .await
        .unwrap()
        .tools
        .iter()
        .all(|tool| tool.name != tracked::CONSUME));
    mcp::invalid(&no_scope, tracked::CONSUME, args.clone()).await;
    let wrong_task = tracked::create(&client, &cid, "other-consume-task").await;
    let mut wrong = args.clone();
    wrong["task_id"] = json!(wrong_task.id);
    assert_eq!(
        mcp::error(mcp::call(&client, tracked::CONSUME, wrong).await).code,
        "forbidden"
    );
    let foreign = f.issue_as_operator(
        &f.foreign,
        Scope {
            operations: vec!["task_consume_reply".into()],
            recipients: vec!["*".into()],
        },
    );
    let foreign_client = mcp::client(f.server.mcp_address.unwrap(), &foreign.credential).await;
    assert_eq!(
        mcp::error(mcp::call(&foreign_client, tracked::CONSUME, args.clone()).await).code,
        "forbidden"
    );
    assert_eq!(tracked::status(&client, &task).await, current);

    let accepted = mcp::outcome(mcp::call(&client, tracked::CONSUME, args.clone()).await);
    assert_eq!(tracked::record(accepted).revision, current.revision + 1);
    f.revoke(&owner.credential_id);
    // A new valid owner can inspect retained work but cannot replay its original
    // consumption mutation through a revoked task backing credential.
    let replacement = tracked::owner(&f);
    let replacement_client =
        mcp::client(f.server.mcp_address.unwrap(), &replacement.credential).await;
    assert!(!tracked::journal(&replacement_client, &task)
        .await
        .is_empty());
    assert_eq!(
        mcp::error(mcp::call(&replacement_client, tracked::CONSUME, args).await).code,
        "unauthorized"
    );
    native.close().await;
    replacement_client.cancel().await.unwrap();
    foreign_client.cancel().await.unwrap();
    no_scope.cancel().await.unwrap();
    peer.cancel().await.unwrap();
    client.cancel().await.unwrap();
    f.close().await;
}
