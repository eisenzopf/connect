#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use connect_client::{ClientConfig, ConnectClient};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::{Scope, VoiceRoute};
mod gate3_support;
use gate3_support::FakeVapi;

#[tokio::test]
async fn accepted_session_bridges_native_audio_and_bounded_context_to_vapi() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut fake = FakeVapi::start().await;

    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let server = RunningServer::start_with_vapi_media(config, None, fake.adapter.clone())
        .await
        .unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let operator = server
        .store
        .issue(&app.identity_id, Scope::operator(), expiry)
        .unwrap();
    let app_key = server
        .store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = server
        .store
        .issue(
            &agent.identity_id,
            Scope::communications(vec![app.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    app.endpoint = server.address;
    let app_token = app_key.token.clone();
    app.credential = app_token.clone();
    app.credential_id = app_key.id.clone();
    let mut client = ConnectClient::open(app).await.unwrap();
    let conversation = client
        .create("vapi-create", vec![agent.identity_id.clone()])
        .await
        .unwrap();
    client
        .send(
            "vapi-context",
            &conversation.id,
            "The release canary is cobalt-seven.",
        )
        .await
        .unwrap();
    server
        .store
        .configure_voice_route(
            &operator.id,
            &VoiceRoute {
                id: "vapi-session".into(),
                conversation_id: conversation.id.clone(),
                identity_id: agent.identity_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+14155550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let session = client
        .session_invite(
            "vapi-invite",
            &conversation.id,
            &agent.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    server
        .store
        .execute(
            &agent_key.id,
            &connect_protocol::Operation::SessionAccept {
                request_id: "vapi-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();

    let mut audio = client
        .open_native_audio(&conversation.id, &session.id, &agent.identity_id)
        .await
        .unwrap();
    let context = fake.next("add-message").await;
    let serialized = context.to_string();
    assert!(serialized.contains("cobalt-seven"));
    assert!(serialized.contains("untrusted historical data"));
    let projected = client.context_view(&conversation.id, 100).await.unwrap();
    assert!(projected.deliveries.iter().any(|delivery| {
        delivery.session_id.as_deref() == Some(session.id.as_str())
            && delivery.provider == "vapi"
            && delivery.state == "delivered"
    }));

    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    let sent_to_vapi = fake.next("audio").await;
    assert_eq!(sent_to_vapi["size"], 160);
    assert_eq!(sent_to_vapi["first"], 0x39);
    let from_vapi = audio
        .receive(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(from_vapi.payload, vec![0x2a; 160]);

    let voice_context = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let value = client.context_view(&conversation.id, 100).await.unwrap();
            if value.items.iter().any(|item| {
                item.kind == "voice_transcript"
                    && item.body == "The voice decision is sapphire-nine."
            }) {
                break value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        voice_context
            .items
            .iter()
            .filter(|item| item.kind == "voice_transcript")
            .count(),
        1,
        "duplicate final provider events must append one durable outcome"
    );
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let value = client.context_view(&conversation.id, 100).await.unwrap();
            if let Some(item) = value.items.iter().find(|item| item.kind == "voice_outcome") {
                break item.clone();
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(outcome.body.contains("assistant-ended-call"));
    assert!(outcome.body.contains("release decision"));
    assert!(!outcome.body.contains("must-not-enter-context"));
    let with_outcome = client.context_view(&conversation.id, 100).await.unwrap();
    assert_eq!(
        with_outcome
            .items
            .iter()
            .filter(|item| item.kind == "voice_outcome")
            .count(),
        1,
        "duplicate provider end reports must append one durable outcome"
    );

    assert!(
        matches!(
            audio.receive(std::time::Duration::from_secs(5)).await,
            Err(connect_client::ClientError::MediaEnded)
        ),
        "provider hangup must reach the native caller without an audio timeout"
    );
    assert!(matches!(
        audio.send_pcmu(&[0x39; 160]).await,
        Err(connect_client::ClientError::MediaEnded)
    ));
    client.close_native_audio(&audio).await.unwrap();
    client.close().await;
    server.shutdown().await.unwrap();

    let mut restarted_fake = FakeVapi::start().await;
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let restarted =
        RunningServer::start_with_vapi_media(config, None, restarted_fake.adapter.clone())
            .await
            .unwrap();
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    app.endpoint = restarted.address;
    app.credential_id = app_key.id;
    app.credential = app_token;
    let mut client = ConnectClient::open(app).await.unwrap();
    let next_session = client
        .session_invite(
            "vapi-restart-invite",
            &conversation.id,
            &agent.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    restarted
        .store
        .execute(
            &agent_key.id,
            &connect_protocol::Operation::SessionAccept {
                request_id: "vapi-restart-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: next_session.id.clone(),
            },
        )
        .unwrap();
    let restarted_audio = client
        .open_native_audio(&conversation.id, &next_session.id, &agent.identity_id)
        .await
        .unwrap();
    let restarted_context = restarted_fake.next("add-message").await.to_string();
    assert!(restarted_context.contains("sapphire-nine"));
    assert!(restarted_context.contains("release decision"));
    client.close_native_audio(&restarted_audio).await.unwrap();
    client.close().await;
    restarted.shutdown().await.unwrap();
}
