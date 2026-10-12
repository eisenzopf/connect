//! Ordinary context defaults stay bounded without changing native/scoped contracts.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
#[path = "support/mcp_fixture.rs"]
mod scoped;
use connect_protocol::{tasks::TaskPermission, Operation, Outcome};
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_context_omitted_limit_matches_bounded_thirty_and_rejects_invalid_limits() {
    let f = agent::Fixture::start().await;
    let client = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let catalog = client.list_tools(None).await.unwrap();
    let context_schema = &catalog
        .tools
        .iter()
        .find(|tool| tool.name == "connect_context_view")
        .unwrap()
        .input_schema;
    assert!(!context_schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!("limit")));
    assert_eq!(context_schema["properties"]["limit"]["default"], 30);
    let history_schema = &catalog
        .tools
        .iter()
        .find(|tool| tool.name == "connect_history")
        .unwrap()
        .input_schema;
    assert!(history_schema["required"]
        .as_array()
        .unwrap()
        .contains(&json!("limit")));

    let mut peer = f.peer_client().await;
    let cid = peer
        .create("context-default-room", vec![f.agent.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let mut sources = Vec::new();
    for index in 0..35 {
        sources.push(
            peer.send(
                &format!("context-fact-{index}"),
                &cid,
                &format!("Review fact {index}: source attribution remains available."),
            )
            .await
            .unwrap(),
        );
    }
    let implicit = mcp::outcome(
        mcp::call(
            &client,
            "connect_context_view",
            json!({"conversation_id":cid}),
        )
        .await,
    );
    let explicit = mcp::outcome(
        mcp::call(
            &client,
            "connect_context_view",
            json!({"conversation_id":cid,"limit":30}),
        )
        .await,
    );
    assert_eq!(implicit, explicit);
    let mut native = f.agent_client().await;
    assert_eq!(
        native
            .request(Operation::ContextView {
                conversation_id: cid.clone(),
                limit: 30,
            })
            .await
            .unwrap(),
        implicit
    );
    let Outcome::ContextView(context) = implicit else {
        panic!("expected bounded native context outcome")
    };
    assert_eq!(context.items.len(), 30);
    assert_eq!(context.omitted, 5);
    assert!(context
        .items
        .iter()
        .all(|item| item.actor_id == f.peer.identity_id));
    assert!(context
        .items
        .iter()
        .any(|item| item.source_id == sources[34].message_id));
    assert!(context
        .items
        .iter()
        .all(|item| item.source_id != sources[0].message_id));
    for (limit, expected_items) in [(1, 1), (100, 35)] {
        let result = mcp::outcome(
            mcp::call(
                &client,
                "connect_context_view",
                json!({"conversation_id":cid,"limit":limit}),
            )
            .await,
        );
        assert!(matches!(result, Outcome::ContextView(view) if view.items.len() == expected_items));
    }
    for limit in [json!(0), json!(101), json!(-1), json!(null), json!("30")] {
        mcp::invalid(
            &client,
            "connect_context_view",
            json!({"conversation_id":cid,"limit":limit}),
        )
        .await;
    }
    // The convenience belongs only to ordinary MCP, not the native wire format.
    assert!(serde_json::from_value::<Operation>(json!({
        "op":"context_view","conversation_id":cid
    }))
    .is_err());
    native.close().await;
    peer.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scoped_context_still_requires_its_limit_and_logical_step() {
    let f = scoped::Fixture::start_permitted(true, vec![TaskPermission::ContextView]).await;
    let client = mcp::client(f.server.mcp_address.unwrap(), f.issued.expose_secret()).await;
    let catalog = client.list_tools(None).await.unwrap();
    let schema = &catalog
        .tools
        .iter()
        .find(|tool| tool.name == "connect_context_view")
        .unwrap()
        .input_schema;
    let required = schema["required"].as_array().unwrap();
    for name in ["limit", "request_id", "step_id"] {
        assert!(required.contains(&json!(name)));
    }
    mcp::invalid(
        &client,
        "connect_context_view",
        json!({"request_id":"scoped-context","step_id":"context-step"}),
    )
    .await;
    let result = mcp::call(
        &client,
        "connect_context_view",
        json!({"request_id":"scoped-context","step_id":"context-step","limit":30}),
    )
    .await;
    assert_ne!(result.is_error, Some(true));
    let value = result.structured_content.unwrap();
    assert_eq!(value["kind"], "tool");
    assert_eq!(value["data"]["status"], "completed");
    assert_eq!(value["data"]["outcome"]["kind"], "context_view");
    assert_eq!(
        value["data"]["outcome"]["data"]["conversation_id"],
        f.task.conversation_id
    );
    client.cancel().await.unwrap();
    f.close().await;
}
