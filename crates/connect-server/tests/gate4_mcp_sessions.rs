//! Successful MCP session controls retain the parent task and conversation.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_client::{ConnectClient, Session};
use connect_store::Scope;
use mcp::{call, success};
use serde_json::{json, Value};

fn session(outcome: Value) -> Session {
    serde_json::from_value(outcome["data"]["outcome"]["data"].clone()).unwrap()
}

async fn peer(f: &fixture::Fixture) -> ConnectClient {
    // Trusted fixture issuance; both participants then use public interfaces.
    let mut config = f.peer.clone();
    let credential = f
        .server
        .store
        .issue(
            &config.identity_id,
            Scope {
                operations: ["session_invite", "session_get", "history"]
                    .map(String::from)
                    .to_vec(),
                recipients: vec![f.owner.identity_id.clone()],
            },
            chrono::Utc::now().timestamp() + 180,
        )
        .unwrap();
    config.credential_id = credential.id;
    config.credential = credential.token;
    ConnectClient::open(config).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn successful_session_tools_preserve_receipts_and_post_session_task_context() {
    let f = fixture::Fixture::start_permitted(true, mcp::permissions()).await;
    let client = mcp::client(&f).await;
    let mut peer = peer(&f).await;
    let cid = &f.task.conversation_id;

    // The other participant initiates contact; the assistant accepts through MCP.
    let inbound = peer
        .session_invite("inbound", cid, &f.owner.identity_id, vec!["audio".into()])
        .await
        .unwrap();
    let accept = json!({"request_id":"accept","step_id":"voice","session_id":inbound.id});
    let accepted = success(call(&client, "connect_session_accept", accept.clone()).await);
    assert_eq!(session(accepted.clone()).state, "accepted");
    assert_eq!(
        success(call(&client, "connect_session_accept", accept).await),
        accepted
    );
    let read = success(
        call(
            &client,
            "connect_session_get",
            json!({"request_id":"read","step_id":"voice","session_id":inbound.id}),
        )
        .await,
    );
    assert_eq!(
        session(read),
        peer.session_get(cid, &inbound.id).await.unwrap()
    );
    let end = json!({"request_id":"end","step_id":"voice","session_id":inbound.id,"reason":"finished this leg"});
    let ended = success(call(&client, "connect_session_end", end.clone()).await);
    assert_eq!(session(ended.clone()).state, "ended");
    assert_eq!(
        success(call(&client, "connect_session_end", end).await),
        ended
    );
    assert_eq!(
        peer.session_get(cid, &inbound.id)
            .await
            .unwrap()
            .ended_reason
            .as_deref(),
        Some("finished this leg")
    );

    let rejected = peer
        .session_invite(
            "reject-inbound",
            cid,
            &f.owner.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    let reject = json!({"request_id":"reject","step_id":"voice","session_id":rejected.id,"reason":"unavailable"});
    let receipt = success(call(&client, "connect_session_reject", reject.clone()).await);
    assert_eq!(session(receipt.clone()).state, "rejected");
    assert_eq!(
        success(call(&client, "connect_session_reject", reject).await),
        receipt
    );

    // An outbound invitation and its cancellation also preserve exact receipts.
    let invite = json!({"request_id":"invite","step_id":"outbound","participant_id":f.peer.identity_id,"media":["audio"]});
    let receipt = success(call(&client, "connect_session_invite", invite.clone()).await);
    let outbound = session(receipt.clone());
    assert_eq!(outbound.state, "invited");
    assert_eq!(outbound.conversation_id, *cid);
    assert_eq!(
        success(call(&client, "connect_session_invite", invite).await),
        receipt
    );
    let cancel = json!({"request_id":"cancel-leg","step_id":"outbound","session_id":outbound.id});
    let receipt = success(call(&client, "connect_session_cancel", cancel.clone()).await);
    assert_eq!(session(receipt.clone()).state, "cancelled");
    assert_eq!(
        success(call(&client, "connect_session_cancel", cancel).await),
        receipt
    );

    let status = success(call(&client, "connect_task_status", json!({})).await);
    assert_eq!(status["data"]["id"], f.task.id);
    assert_eq!(status["data"]["conversation_id"], *cid);
    // Invoking a tool starts the task; ending its bounded legs does not finish it.
    assert_eq!(status["data"]["state"], "running");
    success(call(&client, "connect_send", json!({"request_id":"continue","step_id":"follow-up","message": connect_protocol::messages::MessageDraft::text_all(("Continue after the bounded session.").to_string())})).await);
    let history = peer.history(cid, None, 100).await.unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].body,
        "Continue after the bounded session."
    );
    let recalled = success(call(&client, "connect_recall", json!({"limit":100})).await);
    assert!(recalled["data"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| { source["body"] == "Continue after the bounded session." }));
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}
