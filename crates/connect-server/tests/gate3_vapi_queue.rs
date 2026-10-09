#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use connect_client::{ClientConfig, ConnectClient};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::{Scope, VoiceRoute};
mod gate3_support;
use gate3_support::FakeVapi;

#[tokio::test]
async fn production_vapi_burst_is_bounded_and_cancellation_tears_down_both_legs() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut fake = FakeVapi::start_burst().await;

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
    let mut client = ConnectClient::open(app.clone()).await.unwrap();
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
    fake.next("add-message").await;
    let context = client.context_view(&conversation.id, 100).await.unwrap();
    let provider_ref = context
        .deliveries
        .iter()
        .find(|delivery| {
            delivery.session_id.as_deref() == Some(session.id.as_str())
                && delivery.state == "delivered"
        })
        .unwrap()
        .provider_ref
        .clone()
        .unwrap();
    let provider_connection = rvoip_core::ids::ConnectionId::from_string(provider_ref);
    audio.send_pcmu(&[0x39; 160]).await.unwrap();
    fake.next("audio").await;
    let health = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(health) = fake.adapter.media_health(&provider_connection) {
                if health.inbound_dropped > 0 {
                    break health;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(health.largest_inbound_chunk_bytes, 160 * 1000);
    assert!(health.inbound_dropped >= 990);
    assert!(health.inbound_queue_high_water_ms <= 200, "{health:?}");
    assert!(health.outbound_queue_high_water_ms <= 200, "{health:?}");
    assert!(health.jitter_target_ms <= 200, "{health:?}");
    println!("Gate 3 production queue saturation: {health:?}");

    // Application control and explicit cancellation must progress while the
    // receiver is deliberately stopped and provider audio is saturated.
    client
        .send("burst-control", &conversation.id, "Control still works.")
        .await
        .unwrap();
    let teardown = std::time::Instant::now();
    client
        .session_end(
            "burst-end",
            &conversation.id,
            &session.id,
            Some("fixture-cancel".into()),
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if fake.adapter.media_health(&provider_connection).is_none() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        loop {
            match audio.receive(std::time::Duration::from_secs(1)).await {
                // Ending the durable Session revokes its physical media peer.
                // A normal provider hangup has its separate MediaEnded proof.
                Err(connect_client::ClientError::Transport) => break,
                Ok(_) => continue, // Frames already delivered before cancellation.
                result => panic!("terminal event missing after cancellation: {result:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(teardown.elapsed() < std::time::Duration::from_secs(5));
    assert!(matches!(
        audio.send_pcmu(&[0x39; 160]).await,
        Err(connect_client::ClientError::Transport)
    ));
    client.close().await;
    let mut client = ConnectClient::open(app).await.unwrap();
    assert_eq!(
        client
            .session_get(&conversation.id, &session.id)
            .await
            .unwrap()
            .state,
        "ended"
    );
    let history = client.history(&conversation.id, None, 100).await.unwrap();
    for body in [
        "The release canary is cobalt-seven.",
        "Control still works.",
    ] {
        assert_eq!(
            history
                .messages
                .iter()
                .filter(|message| message.body == body)
                .count(),
            1
        );
    }
    client.close().await;
    tokio::time::timeout(std::time::Duration::from_secs(5), server.shutdown())
        .await
        .unwrap()
        .unwrap();
}
