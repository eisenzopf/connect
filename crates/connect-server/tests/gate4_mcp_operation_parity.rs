//! Every communication-tool projection matches the typed native task API.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
#[path = "support/mcp_operations.rs"]
mod operations;
use connect_client::{ClientError, ConnectClient};
use connect_protocol::tasks::TaskCommand;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_communication_tools_match_native_outcomes_and_known_denials() {
    let f = fixture::Fixture::start_permitted(true, mcp::permissions()).await;
    let mcp = mcp::client(&f).await;
    let mut native = ConnectClient::open(f.owner.clone()).await.unwrap();
    native
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    let cases = operations::cases(&f.task.conversation_id, &f.peer.identity_id);
    assert_eq!(cases.len(), 29);
    let mut succeeded = 0;
    let mut denied = 0;
    for operation in cases {
        let name = format!("connect_{}", operation.name());
        let request_id = name.clone();
        let mut args = serde_json::to_value(&operation)
            .unwrap()
            .as_object()
            .unwrap()
            .clone();
        args.remove("op");
        args.remove("conversation_id");
        args.insert("request_id".into(), json!(request_id));
        args.insert("step_id".into(), json!("parity"));
        let result = mcp::call(&mcp, &name, args.into()).await;
        let positive = matches!(
            operation.name(),
            "resolve"
                | "capabilities"
                | "send"
                | "history"
                | "context_view"
                | "context_deliveries"
                | "artifact_begin"
        );
        let expected = native
            .task(TaskCommand::Invoke {
                request_id,
                task_id: f.task.id.clone(),
                step_id: "parity".into(),
                operation: Box::new(operation),
                reply_to: None,
                wait_for_reply: false,
            })
            .await;
        match expected {
            Ok(outcome) => {
                assert!(
                    positive,
                    "{name} unexpectedly accepted an unavailable resource"
                );
                assert_ne!(result.is_error, Some(true), "{name}");
                assert_eq!(
                    result.structured_content.unwrap(),
                    serde_json::to_value(outcome).unwrap(),
                    "{name}"
                );
                succeeded += 1;
            }
            Err(ClientError::Application(error)) => {
                assert!(!positive, "{name} rejected an authorized positive fixture");
                assert_eq!(result.is_error, Some(true), "{name}");
                let result = result.structured_content.unwrap();
                assert_eq!(result["code"], error.code, "{name}");
                assert_eq!(result["message"], error.message, "{name}");
                denied += 1;
            }
            Err(_) => panic!("native task protocol failed for {name}"),
        }
    }
    assert_eq!(succeeded, 7);
    assert_eq!(denied, 22);
    eprintln!("MCP/native parity: {succeeded} positive outcomes, {denied} known denials");
    native.close().await;
    mcp.cancel().await.unwrap();
    f.close().await;
}
