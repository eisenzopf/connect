#![allow(clippy::unwrap_used)] // Bounded integration assertions deliberately panic.

use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::media_profile::{AUDIO_FRAME_DURATION, AUDIO_QUEUE_CAPACITY};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use std::time::{Duration, Instant};

mod gate3_support;
use gate3_support::wire_audio::{wait_binding_ended, WireAudioPeer};

#[tokio::test]
async fn reordered_rtp_unknown_stream_burst_and_peer_drop_preserve_control() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let database = config.database.clone();
    let server = RunningServer::start(config).await.unwrap();
    let mut app_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let mut agent_config = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let app_identity = app_config.identity_id.clone();
    let agent_identity = agent_config.identity_id.clone();
    for (config, remote) in [
        (&mut app_config, agent_identity),
        (&mut agent_config, app_identity),
    ] {
        let key = server
            .store
            .issue(
                &config.identity_id,
                Scope::communications(vec![remote]),
                chrono::Utc::now().timestamp() + 60,
            )
            .unwrap();
        config.endpoint = server.address;
        config.credential = key.token;
        config.credential_id = key.id;
    }
    let mut app = ConnectClient::open(app_config.clone()).await.unwrap();
    let mut agent = ConnectClient::open(agent_config.clone()).await.unwrap();
    let conversation = app
        .create("fault-create", vec![agent_config.identity_id.clone()])
        .await
        .unwrap();
    let session = app
        .session_invite(
            "fault-invite",
            &conversation.id,
            &agent_config.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    agent
        .session_accept("fault-accept", &conversation.id, &session.id)
        .await
        .unwrap();
    let wire = WireAudioPeer::open(
        &app_config,
        &conversation.id,
        &session.id,
        &agent_config.identity_id,
        "wire-publisher",
    )
    .await;
    let mut audio = agent
        .open_native_audio(&conversation.id, &session.id, &app_config.identity_id)
        .await
        .unwrap();
    agent
        .subscribe_native_audio(&audio, "wire-publisher")
        .await
        .unwrap();
    wire.send(0, wire.local_id, 0);
    tokio::time::sleep(Duration::from_millis(30)).await;
    wire.send(1, wire.local_id, 1);
    audio.receive(Duration::from_secs(2)).await.unwrap();
    // Deliberately send RTP sequence/timestamps 10,12,11,14: one
    // reordered packet plus one missing packet, on actual QUIC datagrams.
    for sequence in [10, 12, 11, 14] {
        wire.send(sequence, wire.local_id, sequence as u8);
        let frame = audio.receive(Duration::from_secs(2)).await.unwrap();
        assert_eq!(frame.payload, vec![sequence as u8; 160]);
        assert_eq!(frame.timestamp_rtp, sequence * 160);
    }
    wire.send(15, u16::MAX, 0xfa);
    assert!(
        matches!(
            audio.receive(Duration::from_millis(150)).await,
            Err(connect_client::ClientError::Timeout)
        ),
        "unknown local stream must not be admitted"
    );
    // Stop consuming, deliberately overrun the bounded audio path, and
    // ensure reliable application traffic and terminal events still progress.
    for sequence in 16..1016 {
        wire.send(sequence, wire.local_id, 0x55);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    app.send(
        "fault-after-burst",
        &conversation.id,
        "Control survived the media burst.",
    )
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    wire.peer
        .connection
        .close(0u32.into(), b"fixture-peer-drop");
    wait_binding_ended(&database, &session.id, &app_config.credential_id).await;
    // The other physical connection is still live. Retiring it must leave
    // the durable Session reconnectable, without ending the Conversation.
    agent.close_native_audio(&audio).await.unwrap();
    wait_binding_ended(&database, &session.id, &agent_config.credential_id).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let session = app
                .session_get(&conversation.id, &session.id)
                .await
                .unwrap();
            if session.state == "accepted" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        app.history(&conversation.id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    agent.close().await;
    app.close().await;
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn production_audio_configuration_obeys_the_frozen_queue_budget() {
    let capacity = AUDIO_QUEUE_CAPACITY.get();
    assert_eq!(
        AUDIO_FRAME_DURATION * capacity as u32,
        Duration::from_millis(200)
    );
    let core = connect_server::media_profile::orchestrator_config();
    assert_eq!(core.audio_queue_capacity, Some(AUDIO_QUEUE_CAPACITY));
    let provider = connect_server::media_profile::vapi_config(
        rvoip_vapi::VapiApiKey::new("fixture-only-key").unwrap(),
    );
    provider.validate().unwrap();
    for value in [
        provider.media_queue_capacity,
        provider.startup_audio_frames,
        provider.inbound_queue_capacity,
        provider.outbound_queue_capacity,
    ] {
        assert!(value <= capacity);
    }
    assert!(provider.jitter_target_ceiling_ms <= 200);
}
