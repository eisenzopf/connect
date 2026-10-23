//! Parsed work arriving after the tool cutoff gets an honest recovery handle.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_client::ConnectClient;
use connect_mcp::{FixedBindingAuth, RunningMcp};
use connect_protocol::tasks::TaskPermission;
use futures_util::{stream, StreamExt};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_decoded_tool_returns_uncertain_task_handle_and_original_ids_recover_once() {
    let f = fixture::Fixture::start_permitted(false, vec![TaskPermission::Send]).await;
    let listener = RunningMcp::start_local(
        f.server.service.clone(),
        Arc::new(FixedBindingAuth::new(f.server.service.clone(), f.binding.clone()).unwrap()),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .unwrap();
    let args = json!({"request_id":"late-body-once","step_id":"draft","message": connect_protocol::messages::MessageDraft::text_all(("One logical draft request.").to_string())});
    let rpc = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"connect_send","arguments":args}})).unwrap();
    let rest = rpc[1..].to_owned();
    let body = stream::once(async { Ok::<_, std::io::Error>(vec![b'{']) }).chain(stream::once(
        async move {
            // Header/body admission has begun, but the body is incomplete. Deliver
            // it after the 9.5s tool cutoff and before the 10s HTTP cutoff.
            tokio::time::sleep(Duration::from_millis(9600)).await;
            Ok::<_, std::io::Error>(rest)
        },
    ));
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(11))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let response = http
        .post(format!("http://{}/mcp", listener.address))
        .bearer_auth(f.issued.expose_secret())
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json")
        .header("MCP-Protocol-Version", "2025-11-25")
        .body(reqwest_mcp::Body::wrap_stream(body))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let result: serde_json::Value = response.json().await.unwrap();
    assert_eq!(result["result"]["isError"], true);
    assert_eq!(
        result["result"]["structuredContent"]["code"],
        "transport-uncertain"
    );
    assert_eq!(result["result"]["_meta"]["connect"]["task_id"], f.task.id);
    assert_eq!(result["result"]["_meta"]["connect"]["reconcile"], true);
    assert!(!result.to_string().contains(f.issued.expose_secret()));
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
    assert!(peer
        .history(&f.task.conversation_id, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    let client = mcp::client_from_grant(listener.address, f.issued.expose_secret()).await;
    let accepted = mcp::call(&client, "connect_send", args.clone()).await;
    assert_eq!(
        accepted.meta.as_ref().unwrap()["connect"]["request_id"],
        args["request_id"]
    );
    assert_eq!(
        accepted.meta.as_ref().unwrap()["connect"]["step_id"],
        args["step_id"]
    );
    let outcome = mcp::success(accepted);
    assert_eq!(
        mcp::success(mcp::call(&client, "connect_send", args).await),
        outcome
    );
    assert_eq!(
        peer.history(&f.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    peer.close().await;
    client.cancel().await.unwrap();
    listener.shutdown().await.unwrap();
    f.close().await;
}
