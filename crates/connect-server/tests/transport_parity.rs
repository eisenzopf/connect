#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
mod support;
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{Operation, Outcome, Response, PROFILE, REQUEST_TYPE};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use rvoip_uctp::{envelope::UctpEnvelope, types::MessageType};
use support::Peer;

#[tokio::test]
async fn native_and_ws_share_state_replay_authority_and_notifications() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    let server = RunningServer::start(config).await.unwrap();
    let ws_address = server.websocket_address.unwrap();
    let mut app_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    app_config.endpoint = server.address;
    let agent_config = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let mut native = ConnectClient::open(app_config.clone()).await.unwrap();
    let mut ws = Peer::websocket(ws_address).await;
    assert!(ws.authenticate(&app_config.credential).await);
    assert_eq!(
        native.request(Operation::Health {}).await.unwrap(),
        ws.request(Operation::Health {}).await.unwrap()
    );
    assert_eq!(
        native
            .request(Operation::Resolve {
                address: "connect:agent@local".into()
            })
            .await
            .unwrap(),
        ws.request(Operation::Resolve {
            address: "connect:agent@local".into()
        })
        .await
        .unwrap()
    );
    let create = Operation::Create {
        request_id: "cross-create".into(),
        participants: vec![agent_config.identity_id.clone()],
    };
    let original = ws.request(create.clone()).await.unwrap();
    assert_eq!(native.request(create).await.unwrap(), original);
    let id = match original {
        Outcome::Conversation(c) => c.id,
        _ => panic!("create"),
    };
    let inbox = ws
        .request(Operation::Inbox {
            cursor: None,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap();
    let cursor = match inbox {
        Outcome::Inbox { cursor, .. } => cursor,
        _ => panic!("inbox"),
    };
    let send = Operation::Send {
        request_id: "cross-send".into(),
        conversation_id: id.clone(),
        body: "same shared service".into(),
    };
    let waiting = native.request(Operation::Inbox {
        cursor,
        limit: 100,
        wait_ms: 1000,
    });
    let sending = ws.request(send.clone());
    let (page, receipt) = tokio::join!(waiting, sending);
    let receipt = receipt.unwrap();
    assert!(matches!(page.unwrap(),Outcome::Inbox {events,..} if events.len()==1));
    assert_eq!(native.request(send).await.unwrap(), receipt);
    let history = Operation::History {
        conversation_id: id.clone(),
        after: None,
        limit: 100,
    };
    assert_eq!(
        native.request(history.clone()).await.unwrap(),
        ws.request(history.clone()).await.unwrap()
    );
    let lookup = Operation::RequestStatus {
        request_id: "cross-send".into(),
    };
    assert_eq!(
        native.request(lookup.clone()).await.unwrap(),
        ws.request(lookup).await.unwrap()
    );
    let close = Operation::Close {
        request_id: "cross-close".into(),
        conversation_id: id.clone(),
    };
    assert_eq!(
        ws.request(close.clone()).await.unwrap(),
        native.request(close).await.unwrap()
    );
    assert!(
        matches!(ws.request(history).await.unwrap(),Outcome::History {messages,..} if messages.len()==1)
    );
    for operation in [
        Operation::Resolve {
            address: "connect:operator@local".into(),
        },
        Operation::History {
            conversation_id: "foreign".into(),
            after: None,
            limit: 100,
        },
        Operation::Send {
            request_id: "foreign-send".into(),
            conversation_id: "foreign".into(),
            body: "denied".into(),
        },
    ] {
        let native_error = match native.request(operation.clone()).await.unwrap_err() {
            ClientError::Application(e) => e,
            other => panic!("{other}"),
        };
        assert_eq!(native_error, ws.request(operation).await.unwrap_err());
    }
    let operator = ClientConfig::load(&directory.join("operator.toml")).unwrap();
    let narrow = server
        .store
        .issue(
            &app_config.identity_id,
            connect_store::Scope::messaging(vec![]),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let mut restricted = app_config.clone();
    restricted.credential = narrow.token;
    restricted.credential_id = narrow.id;
    let mut restricted_native = ConnectClient::open(restricted.clone()).await.unwrap();
    let mut restricted_ws = Peer::websocket(ws_address).await;
    assert!(restricted_ws.authenticate(&restricted.credential).await);
    let saved = Operation::RequestStatus {
        request_id: "cross-send".into(),
    };
    assert!(
        matches!(restricted_native.request(saved.clone()).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    assert_eq!(
        restricted_ws.request(saved).await.unwrap_err().code,
        "forbidden"
    );
    restricted_native.close().await;
    restricted_ws.close().await;
    let authority = server.store.authenticate(&operator.credential).unwrap();
    server
        .store
        .revoke(&authority, &app_config.credential_id)
        .unwrap();
    assert!(
        matches!(native.request(Operation::Health {}).await,Err(ClientError::Application(e)) if e.code=="unauthorized")
    );
    assert_eq!(
        ws.request(Operation::Health {}).await.unwrap_err().code,
        "unauthorized"
    );
    let mut fresh_ws = Peer::websocket(ws_address).await;
    assert!(!fresh_ws.authenticate(&app_config.credential).await);
    fresh_ws.close().await;
    assert!(ConnectClient::open(app_config).await.is_err());
    native.close().await;
    ws.close().await;
    server.shutdown().await.unwrap();
    // The WS listener and its peers are part of the server lifetime.
    let listener = tokio::net::TcpListener::bind(ws_address).await.unwrap();
    drop(listener);
}

#[tokio::test]
async fn negative_envelopes_profile_and_unauthenticated_requests_are_rejected_on_both_wires() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    let server = RunningServer::start(config).await.unwrap();
    let mut client = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    client.endpoint = server.address;
    let mut peers = vec![
        Peer::quic(&client).await,
        Peer::websocket(server.websocket_address.unwrap()).await,
    ];
    for peer in &mut peers {
        let anonymous = peer
            .exchange(UctpEnvelope::new(
                MessageType::Unknown(REQUEST_TYPE.into()),
                serde_json::json!({"profile":PROFILE,"op":"health"}),
            ))
            .await;
        assert_eq!(anonymous.msg_type, MessageType::Error);
        assert!(peer.authenticate(&client.credential).await);
        let unsupported = peer
            .exchange(UctpEnvelope::new(
                MessageType::Unknown(REQUEST_TYPE.into()),
                serde_json::json!({"profile":"unsupported/999","op":"health"}),
            ))
            .await;
        assert_eq!(unsupported.msg_type, MessageType::Error);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/fixtures/negative");
        for path in std::fs::read_dir(directory).unwrap() {
            let envelope: UctpEnvelope =
                serde_json::from_slice(&std::fs::read(path.unwrap().path()).unwrap()).unwrap();
            let reply = peer.exchange(envelope).await;
            let response: Response = reply.decode_payload().unwrap();
            assert_eq!(response.result.unwrap_err().code, "invalid-request");
        }
        let mut wrong_version = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::json!({"profile":PROFILE,"op":"health"}),
        );
        wrong_version.v = 255;
        let reply = peer.exchange(wrong_version).await;
        assert!(
            reply.msg_type == MessageType::Error
                || reply.decode_payload::<Response>().unwrap().result.is_err()
        );
        peer.close().await;
    }
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn large_escaped_pages_fit_wire_budget_and_resume_without_skipping() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    let mut config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let id = match native
        .request(Operation::Create {
            request_id: "create".into(),
            participants: vec![agent.identity_id],
        })
        .await
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("create"),
    };
    let body = "\n".repeat(16_384);
    for index in 0..20 {
        native
            .request(Operation::Send {
                request_id: format!("send-{index}"),
                conversation_id: id.clone(),
                body: body.clone(),
            })
            .await
            .unwrap();
    }
    let mut after = None;
    let mut messages = Vec::new();
    let mut pages = 0;
    loop {
        match native
            .request(Operation::History {
                conversation_id: id.clone(),
                after: after.clone(),
                limit: 100,
            })
            .await
            .unwrap()
        {
            Outcome::History {
                messages: page,
                next,
            } => {
                if page.is_empty() {
                    break;
                }
                assert!(serde_json::to_vec(&page).unwrap().len() < 1024 * 1024);
                messages.extend(page);
                after = next;
                pages += 1;
            }
            _ => panic!("history"),
        }
    }
    assert_eq!(messages.len(), 20);
    assert!(pages >= 2);
    let mut cursor = None;
    let mut count = 0;
    loop {
        match native
            .request(Operation::Inbox {
                cursor: cursor.clone(),
                limit: 100,
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
                count += events.iter().filter(|e| e.kind == "message").count();
                cursor = next;
            }
            _ => panic!("inbox"),
        }
    }
    assert_eq!(count, 20);
    native.close().await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_positive_operation_fixture_executes_on_both_transports() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.websocket_bind = Some("127.0.0.1:0".parse().unwrap());
    let server = RunningServer::start(config).await.unwrap();
    let mut agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    agent.endpoint = server.address;
    let reviews = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/fixtures");
    let mut peers = vec![
        Peer::quic(&agent).await,
        Peer::websocket(server.websocket_address.unwrap()).await,
    ];
    let mut conversation = String::new();
    let mut saved = std::collections::BTreeMap::new();
    for peer in &mut peers {
        assert!(peer.authenticate(&agent.credential).await);
        for (name, kind) in [
            ("health", "health"),
            ("resolve", "identity"),
            ("create", "conversation"),
            ("send", "accepted"),
            ("history", "history"),
            ("inbox", "inbox"),
            ("request_status", "request_status"),
            ("close", "conversation"),
        ] {
            let mut envelope: UctpEnvelope =
                serde_json::from_slice(&std::fs::read(root.join(format!("{name}.json"))).unwrap())
                    .unwrap();
            if name == "create" {
                envelope.payload["participants"] = serde_json::json!([reviews.identity_id]);
            }
            if envelope.payload.get("conversation_id").is_some() {
                envelope.payload["conversation_id"] = serde_json::json!(conversation);
            }
            let id = envelope.id.clone();
            let reply = peer.exchange(envelope).await;
            assert_eq!(reply.in_reply_to, Some(id));
            assert_eq!(reply.v, rvoip_uctp::UCTP_ENVELOPE_VERSION);
            let response: Response = reply.decode_payload().unwrap();
            assert_eq!(response.profile, PROFILE);
            let outcome = response.result.unwrap();
            let value = serde_json::to_value(&outcome).unwrap();
            assert_eq!(value["kind"], kind);
            if name == "create" {
                conversation = value["data"]["id"].as_str().unwrap().into();
            }
            // Reads after closure see its journal event; mutations still replay
            // their original saved receipt even when the current state is closed.
            if ["create", "send", "close", "request_status", "history"].contains(&name) {
                if let Some(previous) = saved.get(name) {
                    assert_eq!(previous, &outcome);
                } else {
                    saved.insert(name, outcome);
                }
            }
        }
        peer.close().await;
    }
    server.shutdown().await.unwrap();
}
