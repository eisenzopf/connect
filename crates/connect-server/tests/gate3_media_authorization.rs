#![allow(clippy::unwrap_used)] // Bounded integration assertions deliberately panic.

use connect_client::{ClientConfig, ConnectClient};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use std::time::Duration;

mod gate3_support;
use gate3_support::wire_audio::{wait_binding_ended, WireAudioPeer};

#[tokio::test]
async fn expiry_closes_an_active_authenticated_media_peer() {
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
    let expired = server
        .store
        .issue(
            &app_config.identity_id,
            Scope::communications(vec![agent_config.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 4,
        )
        .unwrap();
    let mut expiring_config = app_config.clone();
    expiring_config.credential = expired.token;
    expiring_config.credential_id = expired.id;
    let wire = WireAudioPeer::open(
        &expiring_config,
        &conversation.id,
        &session.id,
        &agent_config.identity_id,
        "expiring-publisher",
    )
    .await;
    let mut audio = agent
        .open_native_audio(&conversation.id, &session.id, &app_config.identity_id)
        .await
        .unwrap();
    agent
        .subscribe_native_audio(&audio, "expiring-publisher")
        .await
        .unwrap();
    wire.send(0, wire.local_id, 0x42);
    tokio::time::sleep(Duration::from_millis(30)).await;
    wire.send(1, wire.local_id, 0x42);
    assert_eq!(
        audio.receive(Duration::from_secs(1)).await.unwrap().payload,
        vec![0x42; 160]
    );
    // The raw media peer has a separate short-lived credential. Neither
    // renewing another client nor reliable application traffic extends it.
    tokio::time::timeout(Duration::from_secs(6), wire.peer.connection.closed())
        .await
        .unwrap();
    wait_binding_ended(&database, &session.id, &expiring_config.credential_id).await;
    assert!(wire
        .peer
        .connection
        .send_datagram(vec![0; 8].into())
        .is_err());
    assert!(ConnectClient::open(expiring_config).await.is_err());
    app.send(
        "after-expiry",
        &conversation.id,
        "The conversation remains open.",
    )
    .await
    .unwrap();
    agent.close_native_audio(&audio).await.unwrap();
    agent.close().await;
    app.close().await;
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap()
        .unwrap();
}
