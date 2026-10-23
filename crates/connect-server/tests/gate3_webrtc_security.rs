#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

mod gate3_support;

use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, WebRtcMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store, VoiceRoute};
use futures_util::{SinkExt, StreamExt};
use gate3_support::FakeVapi;
use rvoip_webrtc::{
    peer::session::{PeerRole, RvoipPeerConnection},
    WebRtcConfig,
};
use std::{path::PathBuf, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

struct RunningFixture {
    _directory: tempfile::TempDir,
    server: RunningServer,
    signaling: String,
    token: String,
    foreign_tokens: Vec<String>,
    database: PathBuf,
    fake: FakeVapi,
}

async fn fixture() -> RunningFixture {
    let fake = FakeVapi::start().await;
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path().join("state");
    init::initialize(&state, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&state.join("server.toml")).unwrap();
    let app = ClientConfig::load(&state.join("reviews.toml")).unwrap();
    let agent = ClientConfig::load(&state.join("agent.toml")).unwrap();
    let store = Store::open(&config.database, config.limits.clone()).unwrap();
    let key = store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 3_600,
        )
        .unwrap();
    let outsider = store.create_identity("outsider").unwrap();
    let mut foreign_tokens = Vec::new();
    for identity in [&app.identity_id, &outsider.id] {
        foreign_tokens.push(
            store
                .issue(
                    identity,
                    Scope::communications(vec![agent.identity_id.clone()]),
                    chrono::Utc::now().timestamp() + 3_600,
                )
                .unwrap()
                .token,
        );
    }
    let conversation = match store
        .execute(
            &key.id,
            &Operation::Create {
                request_id: "webrtc-security-create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![agent
                    .identity_id
                    .clone()]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    // The automatic legacy audio route requires a configured assistant route.
    // Exercise it against the existing loopback provider fixture only.
    let operator = store
        .issue(
            &app.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 3_600,
        )
        .unwrap();
    store
        .configure_voice_route(
            &operator.id,
            &VoiceRoute {
                id: "webrtc-security-route".into(),
                conversation_id: conversation.id.clone(),
                identity_id: agent.identity_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+14155550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    config.webrtc = Some(WebRtcMediaConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        route: InboundMediaRoute {
            credential_id: key.id,
            conversation_id: conversation.id,
            assistant_identity_id: agent.identity_id,
            session_id: None,
        },
    });
    drop(store);
    let database = config.database.clone();
    let server = RunningServer::start_with_vapi_media(config, None, fake.adapter.clone())
        .await
        .unwrap();
    RunningFixture {
        signaling: format!("ws://{}", server.webrtc_address.unwrap()),
        server,
        _directory: directory,
        token: key.token,
        foreign_tokens,
        database,
        fake,
    }
}

fn request(signaling: &str, token: &str) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let mut request = signaling.into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rvoip.webrtc.v1, token.{token}")).unwrap(),
    );
    request
}

#[tokio::test]
async fn invalid_credential_and_oversized_offer_never_reach_media_admission() {
    let running = fixture().await;

    let invalid = connect_async(request(&running.signaling, "invalid")).await;
    assert!(
        invalid.is_err(),
        "invalid signaling credential was accepted"
    );

    let (mut socket, _) = connect_async(request(&running.signaling, &running.token))
        .await
        .unwrap();
    let sent = socket
        .send(Message::Text(
            serde_json::json!({
                "type": "offer-ready",
                "request_id": "oversized-offer",
                "sdp": "x".repeat(1_048_576),
            })
            .to_string()
            .into(),
        ))
        .await;
    if sent.is_ok() {
        let response = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap();
        match response {
            None | Some(Err(_)) => {}
            Some(Ok(message)) if message.is_close() => {}
            Some(Ok(message)) if message.is_text() => {
                let value: serde_json::Value =
                    serde_json::from_str(message.to_text().unwrap()).unwrap();
                assert_ne!(value["type"], "answer", "oversized SDP was answered");
                assert_ne!(value["type"], "ready", "oversized SDP reached admission");
            }
            Some(Ok(message)) => panic!("unexpected signaling response: {message:?}"),
        }
    }

    running.server.shutdown().await.unwrap();
}

// Test-owned durable observations complement the public signaling boundary.
fn media_rows(running: &RunningFixture) -> (u64, u64) {
    let db = rusqlite::Connection::open_with_flags(
        &running.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    (
        db.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap(),
        db.query_row("SELECT count(*) FROM session_bindings", [], |r| r.get(0))
            .unwrap(),
    )
}

#[tokio::test]
async fn other_valid_credentials_cannot_assume_the_configured_browser_route() {
    let mut running = fixture().await;
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .unwrap();
    peer.add_local_audio_track().await.unwrap();
    let offer = peer.create_offer_and_gather().await.unwrap();
    let initial_rows = media_rows(&running);
    assert_eq!(initial_rows, (0, 0));
    for token in &running.foreign_tokens {
        assert!(running.server.store.authenticate(token).is_ok());
        for kind in ["offer", "offer-ready"] {
            // Gate5 authenticates normal participant keys at upgrade so they
            // can select their own accepted AV Session. Omitted audio selection
            // still requires the exact configured credential at offer admission.
            let (mut socket, _) = connect_async(request(&running.signaling, token))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({
                        "type": kind,
                        "request_id": "foreign-route-offer",
                        "sdp": offer,
                    })
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let denied = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match socket.next().await {
                        None | Some(Err(_)) => break,
                        Some(Ok(message)) if message.is_close() => break,
                        Some(Ok(message)) if message.is_ping() || message.is_pong() => {}
                        Some(Ok(message)) => {
                            panic!("foreign credential received a route response: {message:?}")
                        }
                    }
                }
            })
            .await;
            assert!(denied.is_ok(), "foreign offer was not rejected promptly");
            assert_eq!(media_rows(&running), initial_rows);
        }
    }
    running.fake.assert_no_message("create-call").await;
    // The identical generated SDP succeeds for the configured key. Therefore
    // foreign refusal is not a malformed-SDP or unsupported-codec false positive.
    let (mut socket, _) = connect_async(request(&running.signaling, &running.token))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "offer-ready", "request_id": "configured-route-offer", "sdp": offer,
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut answered = false;
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(body) = message {
                let value: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(value["request_id"], "configured-route-offer");
                match value["type"].as_str() {
                    Some("answer") => {
                        peer.set_remote_answer(value["sdp"].as_str().unwrap())
                            .await
                            .unwrap();
                        answered = true;
                    }
                    Some("ready") => {
                        assert!(answered);
                        break;
                    }
                    other => panic!("configured route was not admitted: {other:?}: {value}"),
                }
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(media_rows(&running), (1, 1));
    socket.close(None).await.unwrap();
    peer.close().await.unwrap();
    running.server.shutdown().await.unwrap();
}
