//! Lost MCP acknowledgement reconciles the accepted logical effect, without retry IDs.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
#[path = "support/mcp_ack_proxy.rs"]
mod proxy;
use connect_client::ConnectClient;
use connect_protocol::tasks::TaskPermission;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandoned_accepted_response_recovers_same_receipt_and_one_message() {
    let f = fixture::Fixture::start_permitted(true, vec![TaskPermission::Send]).await;
    let proxy = proxy::Proxy::start(f.server.mcp_address.unwrap()).await;
    let client = mcp::client_from_grant(proxy.address, f.issued.expose_secret()).await;
    let args = json!({"request_id":"lost-ack-once","step_id":"draft","message": connect_protocol::messages::MessageDraft::text_all(("One accepted communication effect.").to_string())});
    let (call, accepted) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(2),
            client.call_tool(
                CallToolRequestParams::new("connect_send")
                    .with_arguments(args.as_object().unwrap().clone())
            )
        ),
        tokio::time::timeout(Duration::from_secs(2), proxy.accepted.notified()),
    );
    accepted.unwrap();
    assert!(
        call.is_err(),
        "the proxy delivered the withheld acknowledgement"
    );
    let recorded = proxy.result.lock().unwrap().clone().unwrap();
    assert_ne!(recorded["isError"], true);
    assert_eq!(
        recorded["_meta"]["connect"]["request_id"],
        args["request_id"]
    );
    assert_eq!(recorded["_meta"]["connect"]["step_id"], args["step_id"]);
    assert!(!recorded.to_string().contains(f.issued.expose_secret()));
    // Release the owned proxy/transport; recovery uses a new MCP connection.
    proxy.close().await;
    client.cancel().await.unwrap();
    let client =
        mcp::client_from_grant(f.server.mcp_address.unwrap(), f.issued.expose_secret()).await;
    let recovered = mcp::call(&client, "connect_send", args.clone()).await;
    assert_eq!(mcp::success(recovered), recorded["structuredContent"]);
    assert_eq!(
        mcp::success(mcp::call(&client, "connect_send", args).await),
        recorded["structuredContent"]
    );
    let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
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
    f.close().await;
}
