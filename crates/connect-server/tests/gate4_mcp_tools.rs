//! Public MCP communication, artifacts, recall and bounded continuation.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_client::ConnectClient;
#[path = "support/mcp_client.rs"]
mod mcp;
use mcp::{call, client, permissions, success};
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_tools_share_one_effect_artifact_sources_and_pending_journal_handle() {
    let f = fixture::Fixture::start_permitted(true, permissions()).await;
    let client = client(&f).await;
    let send = json!({"request_id":"send-once","step_id":"draft","body":"Please review the exact blue draft."});
    let accepted = call(&client, "connect_send", send.clone()).await;
    assert_ne!(accepted.is_error, Some(true));
    let accepted = accepted.structured_content.unwrap();
    assert_eq!(
        call(&client, "connect_send", send.clone())
            .await
            .structured_content
            .unwrap(),
        accepted
    );
    let mut changed = send.clone();
    changed["body"] = json!("different intent");
    let rejected = call(&client, "connect_send", changed).await;
    assert_eq!(rejected.is_error, Some(true));
    assert_eq!(
        rejected.structured_content.unwrap()["code"],
        "request-conflict"
    );
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    let history = peer
        .history(&f.task.conversation_id, None, 100)
        .await
        .unwrap();
    assert_eq!(
        history.messages.len(),
        1,
        "matching retries duplicated a message"
    );
    assert_eq!(history.messages[0].body, send["body"].as_str().unwrap());
    let bytes = b"blue draft";
    let sha256 = hex::encode(Sha256::digest(bytes));
    let artifact = success(call(&client, "connect_artifact_begin", json!({"request_id":"file-begin","step_id":"file","name":"draft.txt","media_type":"text/plain","size":bytes.len(),"sha256":sha256})).await);
    let id = artifact["data"]["outcome"]["data"]["id"].as_str().unwrap();
    use base64::Engine;
    let base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    success(call(&client, "connect_artifact_append", json!({"request_id":"file-append","step_id":"file","artifact_id":id,"offset":0,"data":base64})).await);
    success(
        call(
            &client,
            "connect_artifact_finish",
            json!({"request_id":"file-finish","step_id":"file","artifact_id":id}),
        )
        .await,
    );
    let retrieved = success(call(&client, "connect_artifact_get", json!({"request_id":"file-get","step_id":"file-read","artifact_id":id,"offset":0,"limit":32768})).await);
    assert_eq!(retrieved["data"]["outcome"]["data"]["data"], base64);
    let recalled = success(call(&client, "connect_recall", json!({"limit":100})).await);
    assert!(recalled["data"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .any(|source| source["body"]
            .as_str()
            .is_some_and(|s| s.contains("exact blue draft"))));
    let inbox = success(
        call(
            &client,
            "connect_task_wait",
            json!({"limit":100,"wait_ms":0}),
        )
        .await,
    );
    let cursor = inbox["data"]["next"].clone();
    let effect_id = inbox["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|event| {
            (event["data"]["kind"] == "effect").then(|| event["data"]["data"]["id"].clone())
        })
        .unwrap();
    assert_eq!(
        success(
            call(
                &client,
                "connect_effect_status",
                json!({"effect_id":effect_id})
            )
            .await
        )["kind"],
        "effect"
    );
    let started = Instant::now();
    let pending = call(
        &client,
        "connect_task_wait",
        json!({"after":cursor,"limit":100,"wait_ms":100}),
    )
    .await;
    assert_ne!(pending.is_error, Some(true));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(started.elapsed() >= Duration::from_millis(90));
    let content = serde_json::to_string(&pending.content).unwrap();
    assert!(content.contains(&format!("Pending handle: {}", f.task.id)));
    assert!(!content.contains(f.issued.expose_secret()));
    let meta = pending.meta.unwrap();
    assert_eq!(meta["connect"]["task_id"], f.task.id);
    assert_eq!(meta["connect"]["pending"], true);
    assert_eq!(
        pending.structured_content.unwrap()["data"]["events"],
        json!([])
    );
    // An unavailable route is a known business result, never a new conversation.
    let unsupported = call(&client, "connect_call_start", json!({"request_id":"no-route","step_id":"voice","route_id":"absent","destination":"+15555550101"})).await;
    assert_eq!(unsupported.is_error, Some(true));
    success(
        call(
            &client,
            "connect_artifact_delete",
            json!({"request_id":"file-delete","step_id":"file","artifact_id":id}),
        )
        .await,
    );
    let deleted = call(&client, "connect_artifact_get", json!({"request_id":"file-after-delete","step_id":"file-read","artifact_id":id,"offset":0,"limit":100})).await;
    assert_eq!(deleted.is_error, Some(true));
    peer.close().await;
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    assert!(client
        .call_tool(
            CallToolRequestParams::new("connect_send")
                .with_arguments(send.as_object().unwrap().clone())
        )
        .await
        .is_err());
    client.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_advertised_schema_rejects_authority_selectors_and_invalid_shapes() {
    let f = fixture::Fixture::start_permitted(true, permissions()).await;
    let client = client(&f).await;
    let tools = client.list_tools(None).await.unwrap();
    assert_eq!(tools.tools.len(), 28);
    for tool in &tools.tools {
        assert_eq!(tool.input_schema["additionalProperties"], false);
        for selector in [
            "task_id",
            "owner_id",
            "conversation_id",
            "credential",
            "binding",
            "path",
        ] {
            assert!(
                client
                    .call_tool(
                        CallToolRequestParams::new(tool.name.clone()).with_arguments(
                            json!({selector:"forged"}).as_object().unwrap().clone()
                        )
                    )
                    .await
                    .is_err(),
                "{} accepted {selector}",
                tool.name
            );
        }
    }
    for args in [
        json!({"limit":0,"wait_ms":0}),
        json!({"limit":1,"wait_ms":10001}),
        json!({"limit":true,"wait_ms":0}),
        json!({"limit":1,"wait_ms":0,"after":"x".repeat(1025)}),
    ] {
        assert!(client
            .call_tool(
                CallToolRequestParams::new("connect_task_wait")
                    .with_arguments(args.as_object().unwrap().clone())
            )
            .await
            .is_err());
    }
    assert!(client.call_tool(CallToolRequestParams::new("connect_artifact_append").with_arguments(json!({"request_id":"oversize","step_id":"file","artifact_id":"unknown","offset":0,"data":"x".repeat(43693)}).as_object().unwrap().clone())).await.is_err());
    assert!(client
        .call_tool(
            CallToolRequestParams::new("connect_send").with_arguments(
                json!({"request_id":"bad id","step_id":"send","body":"x"})
                    .as_object()
                    .unwrap()
                    .clone()
            )
        )
        .await
        .is_err());
    client.cancel().await.unwrap();
    f.close().await;
}
