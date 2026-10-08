#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
mod support;
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome, Request, REQUEST_TYPE};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};
use std::time::Duration;
use support::Peer;

#[tokio::test]
async fn shutdown_cancels_a_ws_reader_blocked_on_a_full_inbound_queue() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    config.limits.request_timeout_ms = 60_000;
    let server = RunningServer::start(config).await.unwrap();
    let address = server.websocket_address.unwrap();
    let client = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let mut observer = Peer::websocket(address).await;
    assert!(observer.authenticate(&client.credential).await);
    // Empty inbox waits occupy dispatch while the 256-slot input queue fills.
    // Keep the socket open, with no application reads or client cancellation.
    for _ in 0..600 {
        observer
            .send(UctpEnvelope::new(
                MessageType::Unknown(REQUEST_TYPE.into()),
                serde_json::to_value(Request::new(Operation::Inbox {
                    cursor: None,
                    limit: 100,
                    wait_ms: 5000,
                }))
                .unwrap(),
            ))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    tokio::time::timeout(Duration::from_secs(3), server.shutdown())
        .await
        .unwrap()
        .unwrap();
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    drop(listener);
    observer.close().await;
}

#[tokio::test]
async fn stalled_readers_are_bounded_and_healthy_peer_recovers_all_journal_pages() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    config.limits.request_timeout_ms = 500;
    let server = RunningServer::start(config).await.unwrap();
    let mut app_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    app_config.endpoint = server.address;
    let mut agent_config = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    agent_config.endpoint = server.address;
    let mut app = ConnectClient::open(app_config.clone()).await.unwrap();
    let id = match app
        .request(Operation::Create {
            request_id: "create".into(),
            participants: vec![agent_config.identity_id.clone()],
        })
        .await
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("create"),
    };
    app.request(Operation::Send {
        request_id: "seed".into(),
        conversation_id: id.clone(),
        body: "x".repeat(16384),
    })
    .await
    .unwrap();
    let mut slow_native = Peer::quic(&agent_config).await;
    assert!(slow_native.authenticate(&agent_config.credential).await);
    let mut slow_ws = Peer::websocket(server.websocket_address.unwrap()).await;
    assert!(slow_ws.authenticate(&agent_config.credential).await);
    for _ in 0..600 {
        let request = Request::new(Operation::History {
            conversation_id: id.clone(),
            after: None,
            limit: 100,
        });
        let native = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(&request).unwrap(),
        );
        let websocket = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(&request).unwrap(),
        );
        tokio::time::timeout(Duration::from_secs(2), slow_native.send(native))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), slow_ws.send(websocket))
            .await
            .unwrap();
    }
    for index in 0..20 {
        app.request(Operation::Send {
            request_id: format!("live-{index}"),
            conversation_id: id.clone(),
            body: format!("message-{index}"),
        })
        .await
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while slow_native.incoming_len() < 256 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(slow_native.incoming_len(), 256);
    // Hold both observers without reading, beyond the configured send deadline.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let peak = server.websocket_output_high_water();
    assert!(peak > 0 && peak <= 256, "bounded WS queue peak={peak}");
    slow_native.close().await;
    slow_ws.close().await;
    let mut recovered = ConnectClient::open(agent_config).await.unwrap();
    let mut cursor = None;
    let mut messages = Vec::new();
    loop {
        match recovered
            .request(Operation::Inbox {
                cursor: cursor.clone(),
                limit: 3,
                wait_ms: 0,
            })
            .await
            .unwrap()
        {
            Outcome::Inbox {
                events,
                cursor: next,
            } => {
                if events.is_empty() {
                    break;
                }
                messages.extend(events.into_iter().filter_map(|e| e.message));
                cursor = next;
            }
            _ => panic!("inbox"),
        }
    }
    assert_eq!(messages.len(), 21);
    assert_eq!(messages.last().unwrap().body, "message-19");
    recovered.close().await;
    app.close().await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn listener_capacity_refuses_excess_peers_and_preserves_existing_connections() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    config.limits.connections = 1;
    let server = RunningServer::start(config).await.unwrap();
    let mut client = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    client.endpoint = server.address;
    client.timeout_ms = 500;
    let mut existing = ConnectClient::open(client.clone()).await.unwrap();
    assert!(ConnectClient::open(client.clone()).await.is_err());
    existing.request(Operation::Health {}).await.unwrap();
    let mut ws = Peer::websocket(server.websocket_address.unwrap()).await;
    assert!(ws.authenticate(&client.credential).await);
    assert!(tokio_tungstenite::connect_async(format!(
        "ws://{}",
        server.websocket_address.unwrap()
    ))
    .await
    .is_err());
    ws.request(Operation::Health {}).await.unwrap();
    existing.close().await;
    ws.close().await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn expiry_and_identity_scoped_cursors_apply_to_both_active_transports() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    let server = RunningServer::start(config).await.unwrap();
    let mut app_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    app_config.endpoint = server.address;
    let mut agent_config = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    agent_config.endpoint = server.address;
    let mut app = ConnectClient::open(app_config.clone()).await.unwrap();
    app.request(Operation::Create {
        request_id: "create".into(),
        participants: vec![agent_config.identity_id.clone()],
    })
    .await
    .unwrap();
    let cursor = match app
        .request(Operation::Inbox {
            cursor: None,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap()
    {
        Outcome::Inbox { cursor, .. } => cursor,
        _ => panic!("inbox"),
    };
    let mut agent = ConnectClient::open(agent_config.clone()).await.unwrap();
    let mut ws = Peer::websocket(server.websocket_address.unwrap()).await;
    assert!(ws.authenticate(&agent_config.credential).await);
    let wrong = Operation::Inbox {
        cursor,
        limit: 100,
        wait_ms: 0,
    };
    assert!(agent.request(wrong.clone()).await.is_err());
    assert_eq!(ws.request(wrong).await.unwrap_err().code, "invalid-request");
    agent.close().await;
    ws.close().await;
    let expiry = chrono::Utc::now().timestamp() + 2;
    let key = server
        .store
        .issue(
            &agent_config.identity_id,
            Scope::messaging(vec![app_config.identity_id]),
            expiry,
        )
        .unwrap();
    agent_config.credential = key.token;
    agent_config.credential_id = key.id;
    let mut expiring = ConnectClient::open(agent_config.clone()).await.unwrap();
    expiring.request(Operation::Health {}).await.unwrap();
    let mut expiring_ws = Peer::websocket(server.websocket_address.unwrap()).await;
    assert!(expiring_ws.authenticate(&agent_config.credential).await);
    let wait_ms = ((expiry + 1 - chrono::Utc::now().timestamp()) * 1000).max(0) as u64;
    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
    assert!(expiring.request(Operation::Health {}).await.is_err());
    // The WS guard may close the socket before another request: inspect its
    // transport directly instead of requiring an application response.
    use futures_util::StreamExt;
    let active_denied = tokio::time::timeout(Duration::from_secs(5), async {
        if let Peer::WebSocket(socket) = &mut expiring_ws {
            loop {
                match socket.next().await {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        let envelope: UctpEnvelope = serde_json::from_str(&text).unwrap();
                        if envelope.msg_type == MessageType::Error {
                            return true;
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))
                    | Some(Err(_))
                    | None => return true,
                    _ => {}
                }
            }
        }
        false
    })
    .await
    .unwrap();
    assert!(active_denied);
    let mut fresh = Peer::websocket(server.websocket_address.unwrap()).await;
    assert!(!fresh.authenticate(&agent_config.credential).await);
    fresh.close().await;
    assert!(ConnectClient::open(agent_config).await.is_err());
    expiring.close().await;
    expiring_ws.close().await;
    app.close().await;
    server.shutdown().await.unwrap();
}
