//! Retiring a task callback must not turn its durable session into an ordinary call.
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
mod support;
use connect_client::{ClientError, ConnectClient};
use connect_connectors::Vapi;
use connect_server::{config::ServerConfig, runtime::RunningServer};
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_setup_cannot_fall_back_to_an_unscoped_call_after_server_restart() {
    let mut fake = gate3_support::FakeVapi::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    call_setup::route(&f);
    f.h.owner
        .send("retained-fact", &f.h.cid, "VOICE-RESTART-CONTEXT")
        .await
        .unwrap();
    let (_, mut config) = f
        .grant("restart-native", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let session = f.session(&mut client, "restart-session").await;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("setup");
    let mut definition = call_setup::definition(&f);
    definition.expires_at = chrono::Utc::now().timestamp() + 3;
    let address =
        f.h.server
            .prepare_vapi_task_session(
                &session.id,
                definition,
                &directory,
                Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap()),
            )
            .await
            .unwrap();
    let intent = std::fs::read(directory.join("intent.json")).unwrap();
    tokio::time::timeout(Duration::from_secs(6), async {
        while tokio::net::TcpStream::connect(address).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    client.close().await;
    f.h.owner.close().await;
    f.h.peer.close().await;
    let installation = f.h.server.store.path().parent().unwrap().to_owned();
    f.h.server.shutdown().await.unwrap();
    f.h.server = RunningServer::start_with_vapi_media(
        ServerConfig::load(&installation.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        fake.adapter.clone(),
    )
    .await
    .unwrap();
    config.endpoint = f.h.server.address;
    f.h.owner_config.endpoint = f.h.server.address;
    f.peer_config.endpoint = f.h.server.address;
    f.h.owner = ConnectClient::open(f.h.owner_config.clone()).await.unwrap();
    f.h.peer = ConnectClient::open(f.peer_config.clone()).await.unwrap();
    let mut client = ConnectClient::open(config).await.unwrap();
    client.negotiate_task(&f.task.id).await.unwrap();
    let replacement = temp.path().join("replacement");
    let replacement_denied =
        f.h.server
            .prepare_vapi_task_session(
                &session.id,
                call_setup::definition(&f),
                &replacement,
                Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap()),
            )
            .await
            .is_err();
    let denied = media_denied(&mut client, &f.h.cid, &session.id, &f.h.peer_id).await;
    let owner_denied = media_denied(&mut f.h.owner, &f.h.cid, &session.id, &f.h.peer_id).await;
    let provider_call =
        tokio::time::timeout(Duration::from_millis(250), fake.next("create-call")).await;
    assert_eq!(
        std::fs::read(directory.join("intent.json")).unwrap(),
        intent
    );
    client.negotiate_task(&f.task.id).await.unwrap();
    // A new leg continues the original task and conversation with its own
    // explicit setup. Retirement must not disable the agent's ongoing work.
    let continued = f.session(&mut client, "continued-session").await;
    f.h.server
        .prepare_vapi_task_session(
            &continued.id,
            call_setup::definition(&f),
            &temp.path().join("continued"),
            Arc::new(Vapi::new("fake-private-key", &fake.api_base).unwrap()),
        )
        .await
        .unwrap();
    let mut audio = client
        .open_native_audio(&f.h.cid, &continued.id, &f.h.peer_id)
        .await
        .unwrap();
    let continued_post = fake.next("create-call").await;
    assert!(continued_post["body"]["assistantOverrides"]["tools:append"].is_array());
    assert!(fake
        .next("add-message")
        .await
        .to_string()
        .contains("VOICE-RESTART-CONTEXT"));
    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    assert_eq!(fake.next("audio").await["size"], 160);
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    client.close_native_audio(&audio).await.unwrap();
    client.close().await;
    f.close().await;
    assert!(
        provider_call.is_err(),
        "retired task setup dispatched a provider call after restart"
    );
    assert!(
        denied && owner_denied,
        "retired task setup must deny native and owner media attachment"
    );
    assert!(
        replacement_denied,
        "restart must retain the original setup identity"
    );
}

async fn media_denied(
    client: &mut ConnectClient,
    conversation: &str,
    session: &str,
    peer: &str,
) -> bool {
    match client.open_native_audio(conversation, session, peer).await {
        Err(ClientError::Application(_) | ClientError::MediaEnded | ClientError::Transport) => true,
        Err(error) => panic!("unexpected media failure: {error}"),
        Ok(mut audio) => {
            let denied = matches!(
                audio.receive(Duration::from_secs(5)).await,
                Err(ClientError::MediaEnded | ClientError::Transport)
            );
            let _ = client.close_native_audio(&audio).await;
            denied
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_voice_without_a_configured_tool_setup_remains_available() {
    let mut fake = gate3_support::FakeVapi::start().await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    call_setup::route(&f);
    let (_, config) = f
        .grant("voice-only", 120, media::Fixture::permissions())
        .await;
    let mut client = ConnectClient::open(config).await.unwrap();
    let session = f.session(&mut client, "voice-only-session").await;
    let mut audio = client
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await
        .unwrap();
    let post = fake.next("create-call").await;
    assert!(post["body"]["assistantOverrides"]["tools:append"].is_null());
    fake.next("add-message").await;
    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    assert_eq!(fake.next("audio").await["size"], 160);
    assert_eq!(
        audio.receive(Duration::from_secs(5)).await.unwrap().payload,
        vec![0x2a; 160]
    );
    client.close_native_audio(&audio).await.unwrap();
    client.close().await;
    f.close().await;
}
