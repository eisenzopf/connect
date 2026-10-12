//! One ordinary agent credential communicates through MCP and native UCTP/QUIC.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_protocol::{Conversation, Operation, Outcome};
use mcp::{call, outcome};
use serde_json::json;

fn conversation(value: Outcome) -> Conversation {
    let Outcome::Conversation(value) = value else {
        panic!("expected native conversation outcome")
    };
    value
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_reusable_credential_has_native_receipts_in_two_conversations_after_restart() {
    let f = agent::Fixture::start().await;
    assert!(f.expires_at > chrono::Utc::now().timestamp() + 300);
    assert!(!f.agent.credential.starts_with("ctg1"));
    let original_token = f.agent.credential.clone();
    let client = mcp::client(f.server.mcp_address.unwrap(), &original_token).await;
    let mut native = f.agent_client().await;
    let mut conversations = Vec::new();
    for request_id in ["redwood-room", "courier-room"] {
        let op = Operation::Create {
            request_id: request_id.into(),
            participants: vec![f.peer.identity_id.clone()],
        };
        let args = json!({"request_id":request_id,"participants":[f.peer.identity_id]});
        let result = outcome(call(&client, "connect_create", args).await);
        assert_eq!(native.request(op).await.unwrap(), result);
        conversations.push(conversation(result));
    }
    assert_ne!(conversations[0].id, conversations[1].id);
    let intents = [
        (
            "redwood-window",
            "Redwood delivery is Friday at 11:45; code MESA-8421.",
        ),
        (
            "courier-decision",
            "Courier pickup is Tuesday at 09:30; code LAKE-6172.",
        ),
    ];
    let mut receipts = Vec::new();
    for (conversation, (request_id, body)) in conversations.iter().zip(intents) {
        let result = outcome(
            call(
                &client,
                "connect_send",
                json!({"request_id":request_id,"conversation_id":conversation.id,"body":body}),
            )
            .await,
        );
        assert_eq!(
            native
                .request(Operation::Send {
                    request_id: request_id.into(),
                    conversation_id: conversation.id.clone(),
                    body: body.into(),
                })
                .await
                .unwrap(),
            result
        );
        let Outcome::Accepted { message_id, .. } = &result else {
            panic!("expected durable acceptance")
        };
        let context = outcome(
            call(
                &client,
                "connect_context_view",
                json!({"conversation_id":conversation.id,"limit":100}),
            )
            .await,
        );
        let Outcome::ContextView(context) = context else {
            panic!("expected source-backed context")
        };
        assert!(context
            .items
            .iter()
            .any(|item| item.source_id == *message_id
                && item.actor_id == f.agent.identity_id
                && item.body == body));
        assert_eq!(context.conversation_id, conversation.id);
        assert!(context.items.iter().all(|item| item.body == body));
        receipts.push(result);
    }
    let conflict = call(
        &client,
        "connect_send",
        json!({
            "request_id":"redwood-window","conversation_id":conversations[1].id,
            "body":"A retry must not move the original message into another conversation."
        }),
    )
    .await;
    assert_eq!(mcp::error(conflict).code, "conflict");
    native.close().await;
    client.cancel().await.unwrap();
    let f = f.restart().await;
    assert_eq!(
        f.agent.credential, original_token,
        "restart must not renew the agent key"
    );
    let client = mcp::client(f.server.mcp_address.unwrap(), &original_token).await;
    let mut peer = f.peer_client().await;
    for ((conversation, (request_id, body)), receipt) in
        conversations.iter().zip(intents).zip(receipts)
    {
        assert_eq!(
            outcome(
                call(
                    &client,
                    "connect_send",
                    json!({
                        "request_id":request_id,"conversation_id":conversation.id,"body":body
                    })
                )
                .await
            ),
            receipt
        );
        let recovered = outcome(
            call(
                &client,
                "connect_request_status",
                json!({"request_id":request_id}),
            )
            .await,
        );
        assert!(
            matches!(recovered, Outcome::RequestStatus { state, outcome: Some(saved) }
            if state == "accepted" && *saved == receipt)
        );
        let history = peer.history(&conversation.id, None, 100).await.unwrap();
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].body, body);
    }
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn independent_incoming_contact_is_visible_and_answered_without_a_task_grant() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut peer = f.peer_client().await;
    let conversation = peer
        .create("independent-incoming", vec![f.agent.identity_id.clone()])
        .await
        .unwrap();
    let received = peer
        .send(
            "contact-fact",
            &conversation.id,
            "Use Friday 11:45 and the source code MESA-8421.",
        )
        .await
        .unwrap();
    let inbox = outcome(call(&client, "connect_inbox", json!({"limit":100,"wait_ms":0})).await);
    let Outcome::Inbox { events, cursor } = inbox else {
        panic!("expected agent inbox")
    };
    assert!(events
        .iter()
        .any(|event| event.conversation_id == conversation.id
            && event
                .message
                .as_ref()
                .is_some_and(|message| message.id == received.message_id)));
    let context = outcome(
        call(
            &client,
            "connect_context_view",
            json!({
                "conversation_id":conversation.id,"limit":100
            }),
        )
        .await,
    );
    let Outcome::ContextView(context) = context else {
        panic!("expected retained incoming context")
    };
    let source = context
        .items
        .iter()
        .find(|source| source.source_id == received.message_id)
        .unwrap();
    assert_eq!(source.actor_id, f.peer.identity_id);
    assert!(source.body.contains("Friday 11:45") && source.body.contains("MESA-8421"));
    let answer = format!(
        "Confirmed Friday 11:45 with MESA-8421. Source: {}.",
        source.source_id
    );
    let receipt = outcome(
        call(
            &client,
            "connect_send",
            json!({
                "request_id":"incoming-answer","conversation_id":conversation.id,"body":answer
            }),
        )
        .await,
    );
    assert!(matches!(receipt, Outcome::Accepted { .. }));
    let caught_up = outcome(
        call(
            &client,
            "connect_inbox",
            json!({
                "cursor":cursor,"limit":100,"wait_ms":0
            }),
        )
        .await,
    );
    assert!(matches!(caught_up, Outcome::Inbox { .. }));
    let history = peer.history(&conversation.id, None, 100).await.unwrap();
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|message| message.body == answer)
            .count(),
        1
    );
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_history_recovers_old_context_beyond_the_recent_projection() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let mut peer = f.peer_client().await;
    let conversation = peer
        .create("retained-history", vec![f.agent.identity_id.clone()])
        .await
        .unwrap();
    let old = peer
        .send(
            "old-decision",
            &conversation.id,
            "The approved finish is cobalt-7391.",
        )
        .await
        .unwrap();
    for index in 0..80 {
        peer.send(
            &format!("later-{index}"),
            &conversation.id,
            &format!("Later update {index}: {}", "x".repeat(1200)),
        )
        .await
        .unwrap();
    }
    let recent = outcome(
        call(
            &client,
            "connect_context_view",
            json!({
                "conversation_id":conversation.id,"limit":100
            }),
        )
        .await,
    );
    let Outcome::ContextView(recent) = recent else {
        panic!("expected bounded context")
    };
    assert!(recent.omitted > 0);
    assert!(recent
        .items
        .iter()
        .all(|item| item.source_id != old.message_id));
    let page = outcome(
        call(
            &client,
            "connect_history",
            json!({
                "conversation_id":conversation.id,"limit":100
            }),
        )
        .await,
    );
    let Outcome::History { messages, .. } = page else {
        panic!("expected retained history")
    };
    let source = messages
        .iter()
        .find(|message| message.id == old.message_id)
        .unwrap();
    assert_eq!(source.body, "The approved finish is cobalt-7391.");
    assert_eq!(source.sender, f.peer.identity_id);
    let answer = format!("Use cobalt-7391, based on source {}.", source.id);
    outcome(
        call(
            &client,
            "connect_send",
            json!({
                "request_id":"retained-answer","conversation_id":conversation.id,"body":answer
            }),
        )
        .await,
    );
    assert_eq!(
        agent::history(&mut peer, &conversation.id)
            .await
            .iter()
            .filter(|message| message.body == answer)
            .count(),
        1
    );
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}
