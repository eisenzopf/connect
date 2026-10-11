//! The installed operator command stages real rvoip voice and Connect MCP together.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/vapi_host_process.rs"]
mod process;
mod support;
use connect_client::ConnectClient;
use connect_server::cli::init::private_write;
use rmcp::model::CallToolRequestParams;
use serde_json::json;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn installed_voice_uses_task_tools_and_duplex_quic_without_recreating_a_leg() {
    let mut fake = gate3_support::FakeVapi::start_paused().await;
    let mut f = media::Fixture::start().await;
    call_setup::route(&f);
    f.h.owner
        .send(
            "installed-voice-source",
            &f.h.cid,
            "INSTALLED-VOICE-CONTEXT",
        )
        .await
        .unwrap();
    let (_, config) = f
        .grant("installed-voice-native", 300, media::Fixture::permissions())
        .await;
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let session = f.session(&mut native, "installed-voice-session").await;
    let mut definition = call_setup::definition(&f);
    definition.expires_at = chrono::Utc::now().timestamp() + 300;
    native.close().await;
    let state = f.h.stop_for_host().await;
    let manifest = json!({"profile":"connect-vapi-task-host/1", "definition":definition,
        "session_id":session.id,"state_directory":"voice-private"});
    let input = state.directory.join("voice-host.json");
    private_write(&input, &serde_json::to_vec(&manifest).unwrap()).unwrap();
    let host = process::Process::start(&state, &input, &fake.api_base, "voice").await;
    let mut native = ConnectClient::open(config).await.unwrap();
    let (cid, peer, sid) = (state.cid.clone(), state.peer_id.clone(), session.id.clone());
    let opening = tokio::spawn(async move {
        let audio = native.open_native_audio(&cid, &sid, &peer).await.unwrap();
        (native, audio)
    });
    let post = fake.next("create-call").await;
    assert_eq!(
        post["body"]["assistantOverrides"]["tools:append"][0]["server"]["url"],
        host.ready["callback_local"]
    );
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
        .contains("INSTALLED-VOICE-CONTEXT"));
    fake.release_call();
    let (native, mut audio) = opening.await.unwrap();
    fake.next("add-message").await;
    audio.send_pcmu(&[0x31; 160]).await.unwrap();
    assert_eq!(fake.next("audio").await["size"], 160);
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    tools.cancel().await.unwrap();
    native.close().await;
    let intent_path = state.directory.join("voice-private/intent.json");
    let intent: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&intent_path).unwrap()).unwrap();
    host.stop(&[intent["secret"].as_str().unwrap(), &state.owner.credential])
        .await;
    process::Process::fails(&state, &input, &fake.api_base, "no-leg-replacement").await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(intent_path).unwrap()).unwrap(),
        intent
    );
}
