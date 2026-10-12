//! MCP pages recover an older fact outside initial context with source proof.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_client::{tasks::*, ConnectClient};
use connect_store::Store;
use serde_json::json;

#[tokio::test]
async fn bounded_mcp_pages_retrieve_old_source_and_deny_another_tasks_cursor() {
    let f = fixture::Fixture::start_permitted(
        true,
        vec![TaskPermission::History, TaskPermission::ContextView],
    )
    .await;
    let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
    let fact = "The retained launch checksum is cobalt-7391.";
    let mut fact_id = None;
    for index in 0..180 {
        let receipt = owner
            .send(
                &format!("source-{index}"),
                &f.task.conversation_id,
                &if index == 60 {
                    fact.into()
                } else {
                    format!("Source {index}: {}", "x".repeat(1200))
                },
            )
            .await
            .unwrap();
        if index == 60 {
            fact_id = Some(receipt.message_id)
        }
    }
    let client = mcp::client(&f).await;
    let initial = mcp::success(
        mcp::call(
            &client,
            "connect_context_view",
            json!({"request_id":"initial-context","step_id":"context","limit":100}),
        )
        .await,
    );
    assert!(!serde_json::to_string(&initial).unwrap().contains(fact));
    let mut after = serde_json::Value::Null;
    let mut found = false;
    let mut pages = 0;
    for _ in 0..20 {
        let page = mcp::success(
            mcp::call(
                &client,
                "connect_recall",
                json!({"after":after,"limit":100}),
            )
            .await,
        );
        assert!(serde_json::to_vec(&page).unwrap().len() <= TASK_CONTEXT_BYTES);
        pages += 1;
        if let Some(source) = page["data"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["body"] == fact)
        {
            assert_eq!(source["source_id"], fact_id.as_ref().unwrap().as_str());
            assert_eq!(source["actor_id"], f.owner.identity_id);
            assert!(source["provenance"]
                .as_str()
                .unwrap()
                .starts_with("conversation-journal:"));
            assert_eq!(source["state"], "available");
            found = true;
            break;
        }
        assert!(page["data"]["omitted"].as_u64().unwrap() > 0);
        assert_ne!(page["data"]["next"], after);
        after = page["data"]["next"].clone();
    }
    assert!(
        found && pages > 1,
        "Old fact must require continuation beyond the first page"
    );
    owner
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    let TaskOutcome::Record(other) = owner
        .task(TaskCommand::Create {
            request_id: "other-recall-task".into(),
            definition: TaskDefinition {
                conversation_id: f.task.conversation_id.clone(),
                scope: f.task.scope.clone(),
                expires_at: f.task.expires_at,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap()
    else {
        panic!("other task")
    };
    // The fixture owner's administrative credential deliberately lacks recall.
    // Read the sibling through its own explicitly issued narrow Native grant.
    let mut other_binding = f.binding.clone();
    other_binding.external_id = "sibling-recall-reader".into();
    f.server
        .service
        .register_task_binding(
            f.owner.credential_id.clone(),
            other.id.clone(),
            other_binding.clone(),
        )
        .await
        .unwrap();
    let other_grant = f
        .server
        .service
        .issue_task_grant(
            f.owner.credential_id.clone(),
            "sibling-recall-grant".into(),
            GrantDefinition {
                task_id: other.id.clone(),
                binding: other_binding,
                operations: vec![TaskPermission::History],
                expires_at: chrono::Utc::now().timestamp() + 120,
            },
        )
        .await
        .unwrap();
    let protected = ProtectedTaskGrant::new(
        other_grant.grant.clone(),
        other_grant.expose_secret().into(),
    );
    let mut sibling = ConnectClient::open(f.owner.clone().with_task_grant(&protected).unwrap())
        .await
        .unwrap();
    sibling.negotiate_task(&other.id).await.unwrap();
    let TaskOutcome::Recall { next, .. } = sibling
        .task(TaskCommand::Recall {
            task_id: other.id.clone(),
            after: None,
            limit: 1,
        })
        .await
        .unwrap()
    else {
        panic!("other recall")
    };
    assert!(next.is_some());
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let denied = mcp::call(&client, "connect_recall", json!({"after":next,"limit":100})).await;
    assert_eq!(denied.is_error, Some(true));
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    // The bound task remains usable after rejecting a sibling task's cursor.
    let status = mcp::success(mcp::call(&client, "connect_task_status", json!({})).await);
    assert_eq!(status["data"]["id"], f.task.id);
    sibling.close().await;
    owner.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}
