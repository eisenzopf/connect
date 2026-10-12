//! Public callers distinguish a received journal event from nearby message IDs.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_client::ConnectClient;
use connect_protocol::tasks::{TaskCommand, TaskOutcome, TaskPermission};
use mcp::{call, client, success};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn received_reply_event_routes_one_follow_up_while_other_ids_are_denied() {
    let f = fixture::Fixture::start_permitted(
        true,
        vec![
            TaskPermission::Send,
            TaskPermission::History,
            TaskPermission::Inbox,
        ],
    )
    .await;
    let mcp = client(&f).await;
    let tools = mcp.list_all_tools().await.unwrap();
    let send = tools
        .iter()
        .find(|tool| tool.name == "connect_send")
        .unwrap();
    assert!(send.input_schema["properties"]["reply_to"]["description"]
        .as_str()
        .unwrap()
        .contains("journal event ID"));
    success(
        call(
            &mcp,
            "connect_send",
            json!({
                "request_id":"question", "step_id":"review", "body":"Approve the draft?",
                "wait_for_reply":true
            }),
        )
        .await,
    );
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    let cid = &f.task.conversation_id;
    let question = peer.history(cid, None, 10).await.unwrap().messages[0]
        .id
        .clone();
    peer.negotiate_tasks("connect:reviews@local", Some(cid))
        .await
        .unwrap();
    let TaskOutcome::ReplyAccepted {
        message_id, reply, ..
    } = peer
        .task(TaskCommand::Reply {
            request_id: "human-answer".into(),
            conversation_id: cid.clone(),
            reply_to: question.clone(),
            body: "Approved with indigo-42".into(),
        })
        .await
        .unwrap()
    else {
        panic!("expected attributed reply")
    };
    let page = success(call(&mcp, "connect_task_wait", json!({"limit":100,"wait_ms":0})).await);
    let event = page["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["data"]["kind"] == "reply")
        .unwrap();
    assert_eq!(event["data"]["data"]["event_id"], reply.event_id);
    assert_eq!(event["data"]["data"]["task_id"], f.task.id);
    let follow_up = |reference: &str| {
        json!({
            "request_id":"follow-up", "step_id":"decision", "body":"Confirmed indigo-42",
            "reply_to":reference, "wait_for_reply":false
        })
    };
    for wrong in [
        question,
        message_id,
        event["id"].as_str().unwrap().to_owned(),
    ] {
        assert_ne!(wrong, reply.event_id);
        let result = call(&mcp, "connect_send", follow_up(&wrong)).await;
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result.structured_content.unwrap()["code"], "forbidden");
    }
    assert_eq!(peer.history(cid, None, 10).await.unwrap().messages.len(), 2);
    let intended = follow_up(&reply.event_id);
    let accepted = success(call(&mcp, "connect_send", intended.clone()).await);
    assert_eq!(accepted["data"]["status"], "completed");
    assert_eq!(
        success(call(&mcp, "connect_send", intended).await),
        accepted
    );
    let history = peer.history(cid, None, 10).await.unwrap();
    assert_eq!(history.messages.len(), 3);
    assert_eq!(history.messages[2].body, "Confirmed indigo-42");
    peer.close().await;
    mcp.cancel().await.unwrap();
    f.close().await;
}
