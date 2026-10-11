//! The first voice callback uses real MCP while rvoip owns one provider POST.
#![allow(clippy::unwrap_used)]
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ClientError, ConnectClient};
use connect_connectors::Vapi;
use connect_store::Store;
use rmcp::model::CallToolRequestParams;
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
use call_setup::{definition, http, mcp, route};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_call_tools_context_and_native_quic_audio_share_one_task_and_conversation() {
    let mut fake = gate3_support::FakeVapi::start_paused().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    route(&f);
    f.h.owner
        .send("old-voice-fact", &f.h.cid, "VOICE-BOOT-CANARY")
        .await
        .unwrap();
    let (_, config) = f
        .grant("voice-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "tool-voice").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("call-setup");
    let sdk = Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap());
    let address =
        f.h.server
            .prepare_vapi_task_session(&session.id, definition(&f), &directory, sdk.clone())
            .await
            .unwrap();
    let cid = f.h.cid.clone();
    let peer = f.h.peer_id.clone();
    let sid = session.id.clone();
    let opening = tokio::spawn(async move {
        let audio = client.open_native_audio(&cid, &sid, &peer).await.unwrap();
        (client, audio)
    });
    let post = fake.next("create-call").await;
    // Native signaling may be ready before the provider leg. The fixture's
    // explicit create-response semaphore, not that signaling future, gates POST.
    assert_eq!(post["body"]["transport"]["provider"], "vapi.websocket");
    assert_eq!(
        post["body"]["assistantOverrides"]["tools:append"][0]["server"]["url"],
        format!("http://{address}/mcp")
    );
    assert!(post["body"]["assistantOverrides"].get("model").is_none());
    let tools = mcp(&post["body"]).await;
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
        .contains("VOICE-BOOT-CANARY"));
    let arguments=json!({"request_id":"voice-tool-message","step_id":"draft","body":"First voice tool result"}).as_object().unwrap().clone();
    let first = tools
        .call_tool(CallToolRequestParams::new("connect_send").with_arguments(arguments.clone()))
        .await
        .unwrap();
    let retry = tools
        .call_tool(CallToolRequestParams::new("connect_send").with_arguments(arguments))
        .await
        .unwrap();
    assert_eq!(first.structured_content, retry.structured_content);
    assert_ne!(first.is_error, Some(true));
    assert_eq!(
        first.structured_content.unwrap()["data"]["task_id"],
        f.task.id
    );
    // Even a protected repeated staging cannot dispatch a second provider call.
    assert!(connect_server::vapi_bootstrap::stage_call(
        f.h.server.service.clone(),
        &directory,
        &f.h.owner_config.credential_id,
        &f.h.server
            .store
            .authorize_vapi_task_session_setup(
                &f.h.owner_config.credential_id,
                &f.task.id,
                &session.id
            )
            .unwrap(),
    )
    .await
    .is_err());
    fake.release_call();
    let (mut client, mut audio) = opening.await.unwrap();
    let context = fake.next("add-message").await.to_string();
    assert!(context.contains("VOICE-BOOT-CANARY"));
    assert!(context.contains("untrusted historical data"));
    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    assert_eq!(fake.next("audio").await["size"], 160);
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let view = f.h.owner.context_view(&f.h.cid, 100).await.unwrap();
            if serde_json::to_string(&view)
                .unwrap()
                .contains("sapphire-nine")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    client.close_native_audio(&audio).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while tokio::net::TcpStream::connect(address).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    client.close().await;
    let status =
        f.h.owner
            .task(TaskCommand::Get {
                task_id: f.task.id.clone(),
            })
            .await
            .unwrap();
    let TaskOutcome::Record(task) = status else {
        panic!("task")
    };
    assert_eq!(task.conversation_id, f.h.cid);
    assert!(!task.state.is_terminal());
    assert_eq!(
        f.h.peer
            .history(&f.h.cid, None, 100)
            .await
            .unwrap()
            .messages
            .iter()
            .filter(|m| m.body == "First voice tool result")
            .count(),
        1
    );
    tools.cancel().await.unwrap();
    let failed = temp.path().join("duplicate-call");
    assert!(f
        .h
        .server
        .prepare_vapi_task_session(&session.id, definition(&f), &failed, sdk)
        .await
        .is_err());
    fake.assert_no_message("create-call").await;
    f.close().await;
    assert!(
        tokio::net::TcpStream::connect(address).await.is_err(),
        "owned MCP listener must drain with the server"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_after_dispatch_denies_first_callback_and_context_without_repeating_call() {
    let mut fake = gate3_support::FakeVapi::start_paused().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    route(&f);
    let (_, config) = f
        .grant("cancel-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "cancel-voice").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("cancel-call");
    let address =
        f.h.server
            .prepare_vapi_task_session(
                &session.id,
                definition(&f),
                &directory,
                Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap()),
            )
            .await
            .unwrap();
    let cid = f.h.cid.clone();
    let peer = f.h.peer_id.clone();
    let sid = session.id.clone();
    let opening = tokio::spawn(async move {
        let denied = match client.open_native_audio(&cid, &sid, &peer).await {
            Err(_) => true,
            Ok(mut audio) => {
                let denied = matches!(
                    audio.receive(Duration::from_secs(5)).await,
                    Err(ClientError::MediaEnded | ClientError::Transport)
                );
                let _ = client.close_native_audio(&audio).await;
                denied
            }
        };
        client.close().await;
        denied
    });
    let expected = f
        .h
        .server
        .store
        .authorize_vapi_task_session_setup(&f.h.owner_config.credential_id, &f.task.id, &session.id)
        .unwrap();
    let post = fake.next("create-call").await;
    f.h.owner
        .task(TaskCommand::Cancel {
            task_id: f.task.id.clone(),
            request_id: "cancel-first-callback".into(),
            reason: None,
        })
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let response=http().await.post(format!("http://{address}/mcp")).header("x-call-id","call-connect-gate-three").header("Accept","application/json, text/event-stream")
        .header("Authorization",post["body"]["assistantOverrides"]["tools:append"][0]["server"]["headers"]["Authorization"].as_str().unwrap())
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cancel-voice","version":"1"}}})).send().await.unwrap();
    assert!(response.status().is_client_error());
    assert_eq!(
        Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    fake.release_call();
    assert!(tokio::time::timeout(Duration::from_secs(15), opening)
        .await
        .unwrap()
        .unwrap());
    fake.assert_no_message("add-message").await;
    assert!(connect_server::vapi_bootstrap::stage_call(
        f.h.server.service.clone(),
        &directory,
        &f.h.owner_config.credential_id,
        &expected
    )
    .await
    .is_err());
    fake.assert_no_message("create-call").await;
    f.close().await;
}
