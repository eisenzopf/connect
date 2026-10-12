//! Task cancellation requires explicit ceilings and never selects another task.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_client::{tasks::*, ConnectClient};
use connect_store::Store;
use rmcp::model::CallToolRequestParams;
use serde_json::json;

#[tokio::test]
async fn ordinary_grants_cannot_discover_cancel_or_amplify_the_task_ceiling() {
    let f = fixture::Fixture::start_configured(true).await;
    let client = mcp::client(&f).await;
    assert!(client
        .list_tools(None)
        .await
        .unwrap()
        .tools
        .iter()
        .all(|t| t.name != "connect_task_cancel"));
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(client
        .call_tool(
            CallToolRequestParams::new("connect_task_cancel").with_arguments(
                json!({"request_id":"missing-permission"})
                    .as_object()
                    .unwrap()
                    .clone()
            )
        )
        .await
        .is_err());
    let issued = f
        .server
        .service
        .issue_task_grant(
            f.owner.credential_id.clone(),
            "no-ceiling".into(),
            GrantDefinition {
                task_id: f.task.id.clone(),
                binding: f.binding.clone(),
                operations: vec![TaskPermission::TaskCancel],
                expires_at: chrono::Utc::now().timestamp() + 120,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(issued.code, "forbidden");
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn explicit_cancel_affects_only_the_bound_task_and_revokes_its_own_connection() {
    let f = fixture::Fixture::start_permitted(true, vec![TaskPermission::TaskCancel]).await;
    let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
    owner
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    let TaskOutcome::Record(sibling) = owner
        .task(TaskCommand::Create {
            request_id: "sibling-task".into(),
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
        panic!("sibling")
    };
    let client = mcp::client(&f).await;
    let tools = client.list_tools(None).await.unwrap().tools;
    let cancel = tools
        .iter()
        .find(|t| t.name == "connect_task_cancel")
        .unwrap();
    assert_eq!(cancel.input_schema["additionalProperties"], false);
    let protected =
        ProtectedTaskGrant::new(f.issued.grant.clone(), f.issued.expose_secret().into());
    let mut native = ConnectClient::open(f.owner.clone().with_task_grant(&protected).unwrap())
        .await
        .unwrap();
    native.negotiate_task(&f.task.id).await.unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let Err(connect_client::ClientError::Application(error)) = native
        .task(TaskCommand::Cancel {
            request_id: "native-foreign-task".into(),
            task_id: sibling.id.clone(),
            reason: None,
        })
        .await
    else {
        panic!("Native grant must remain bound to its original task")
    };
    assert_eq!(error.code, "forbidden");
    assert!(native
        .send(
            "cancel-is-not-send",
            &f.task.conversation_id,
            "Not delegated."
        )
        .await
        .is_err());
    for args in [
        json!({"request_id":"foreign-task","task_id":sibling.id}),
        json!({"request_id":"forged-owner","owner_id":f.owner.identity_id}),
        json!({"request_id":"bad-reason","reason":"x".repeat(513)}),
        json!({"request_id":"bad-type","reason":true}),
    ] {
        assert!(client
            .call_tool(
                CallToolRequestParams::new("connect_task_cancel")
                    .with_arguments(args.as_object().unwrap().clone())
            )
            .await
            .is_err());
    }
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    let result = mcp::success(
        mcp::call(
            &client,
            "connect_task_cancel",
            json!({"request_id":"cancel-bound","reason":"Work withdrawn"}),
        )
        .await,
    );
    assert_eq!(result["data"]["id"], f.task.id);
    assert_eq!(result["data"]["state"], "cancelled");
    let after = Store::inspect_snapshot(f.server.store.path()).unwrap();
    assert!(client
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .is_err());
    assert!(client
        .call_tool(
            CallToolRequestParams::new("connect_task_cancel").with_arguments(
                json!({"request_id":"cancel-bound","reason":"Work withdrawn"})
                    .as_object()
                    .unwrap()
                    .clone()
            )
        )
        .await
        .is_err());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        after
    );
    assert!(native
        .task(TaskCommand::Get {
            task_id: f.task.id.clone()
        })
        .await
        .is_err());
    native.close().await;
    // The protected owner can inspect the original accepted control receipt.
    assert_eq!(
        serde_json::to_value(
            owner
                .task(TaskCommand::Cancel {
                    request_id: "cancel-bound".into(),
                    task_id: f.task.id.clone(),
                    reason: Some("Work withdrawn".into()),
                })
                .await
                .unwrap()
        )
        .unwrap(),
        result
    );
    let TaskOutcome::Record(current) = owner
        .task(TaskCommand::Get {
            task_id: sibling.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("sibling")
    };
    assert_eq!(current.state, TaskState::Ready);
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    peer.send(
        "after-task-cancel",
        &f.task.conversation_id,
        "Conversation continues.",
    )
    .await
    .unwrap();
    assert_eq!(
        peer.history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    peer.close().await;
    owner.close().await;
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test]
async fn explicit_native_cancellation_uses_the_same_scoped_terminal_contract() {
    let f = fixture::Fixture::start_permitted(true, vec![TaskPermission::TaskCancel]).await;
    let grant = ProtectedTaskGrant::new(f.issued.grant.clone(), f.issued.expose_secret().into());
    let mut client = ConnectClient::open(f.owner.clone().with_task_grant(&grant).unwrap())
        .await
        .unwrap();
    client.negotiate_task(&f.task.id).await.unwrap();
    let TaskOutcome::Record(cancelled) = client
        .task(TaskCommand::Cancel {
            request_id: "native-bound-cancel".into(),
            task_id: f.task.id.clone(),
            reason: None,
        })
        .await
        .unwrap()
    else {
        panic!("cancelled record")
    };
    assert_eq!(cancelled.id, f.task.id);
    assert_eq!(cancelled.state, TaskState::Cancelled);
    assert!(client
        .task(TaskCommand::Get {
            task_id: f.task.id.clone()
        })
        .await
        .is_err());
    client.close().await;
    f.close().await;
}
