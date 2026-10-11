//! Advertised callbacks traverse a local operator mapping with current authority.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
#[path = "support/callback_forwarder.rs"]
mod forwarder;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::TaskCommand, ConnectClient};
use connect_connectors::Vapi;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn advertised_callback_reaches_owned_listener_and_rechecks_cancellation() {
    let mut fake = gate3_support::FakeVapi::start_paused().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    call_setup::route(&f);
    f.h.owner
        .send("callback-source", &f.h.cid, "CALLBACK-MAPPING-CONTEXT")
        .await
        .unwrap();
    let (_, config) = f
        .grant("callback-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "callback-session").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("setup");
    let gateway = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_address = gateway.local_addr().unwrap();
    let advertised = format!("http://{gateway_address}/mcp");
    let base = fake.api_base.clone();
    let provider = tokio::task::spawn_blocking(move || {
        Arc::new(Vapi::new("fake-private-key", &base).unwrap())
    })
    .await
    .unwrap();
    let address =
        f.h.server
            .prepare_vapi_task_session_with_callback(
                &session.id,
                call_setup::definition(&f),
                &directory,
                provider,
                Some(&advertised),
            )
            .await
            .unwrap();
    assert!(address.ip().is_loopback());
    assert_ne!(address, gateway_address);
    let proxy = forwarder::Forwarder::start(gateway, address).await;
    let (cid, peer, sid) = (f.h.cid.clone(), f.h.peer_id.clone(), session.id.clone());
    let opening = tokio::spawn(async move {
        let audio = client.open_native_audio(&cid, &sid, &peer).await.unwrap();
        (client, audio)
    });
    let post = fake.next("create-call").await;
    let tool_server = &post["body"]["assistantOverrides"]["tools:append"][0]["server"];
    assert_eq!(tool_server["url"], advertised);
    let tools = call_setup::mcp(&post["body"]).await;
    let recall = tools
        .call_tool(
            CallToolRequestParams::new("connect_recall")
                .with_arguments(json!({"limit":100}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(recall.is_error, Some(true));
    assert!(recall
        .structured_content
        .unwrap()
        .to_string()
        .contains("CALLBACK-MAPPING-CONTEXT"));
    let http = call_setup::http().await;
    let secret = tool_server["headers"]["Authorization"].as_str().unwrap();
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},
        "clientInfo":{"name":"mapped","version":"1"}}});
    // Proxying must not bypass origin/auth checks or broaden backend hosts.
    let origin = http
        .post(&advertised)
        .header("x-call-id", "call-connect-gate-three")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", secret)
        .header("Origin", "https://untrusted.example.invalid")
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(origin.status(), reqwest_mcp::StatusCode::FORBIDDEN);
    let host = http
        .post(format!("http://{address}/mcp"))
        .header("x-call-id", "call-connect-gate-three")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", secret)
        .header("Host", gateway_address.to_string())
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(host.status(), reqwest_mcp::StatusCode::FORBIDDEN);
    let unauthenticated = http
        .post(&advertised)
        .header("x-call-id", "call-connect-gate-three")
        .json(&initialize)
        .send()
        .await
        .unwrap();
    assert_eq!(
        unauthenticated.status(),
        reqwest_mcp::StatusCode::UNAUTHORIZED
    );
    fake.release_call();
    let (client, mut audio) = opening.await.unwrap();
    fake.next("add-message").await;
    audio.send_pcmu(&[0x31; 160]).await.unwrap();
    assert_eq!(fake.next("audio").await["size"], 160);
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    f.h.owner
        .task(TaskCommand::Cancel {
            task_id: f.task.id.clone(),
            request_id: "cancel-advertised-callback".into(),
            reason: None,
        })
        .await
        .unwrap();
    let denied = http.post(&advertised)
        .header("x-call-id", "call-connect-gate-three")
        .header("Accept", "application/json, text/event-stream")
        .header("Authorization", secret)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"mapped","version":"1"}}}))
        .send().await.unwrap();
    assert_eq!(denied.status(), reqwest_mcp::StatusCode::UNAUTHORIZED);
    let _ = tools.cancel().await;
    client.close().await;
    f.close().await;
    proxy.close().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(tokio::net::TcpStream::connect(gateway_address)
        .await
        .is_err());
}
