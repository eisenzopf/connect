#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, WebRtcMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
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
}

async fn fixture() -> RunningFixture {
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
                participants: vec![agent.identity_id.clone()],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
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
    let server = RunningServer::start_local(config, None, false)
        .await
        .unwrap();
    RunningFixture {
        signaling: format!("ws://{}", server.webrtc_address.unwrap()),
        server,
        _directory: directory,
        token: key.token,
        foreign_tokens,
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

#[tokio::test]
async fn other_valid_credentials_cannot_assume_the_configured_browser_route() {
    let running = fixture().await;
    for token in &running.foreign_tokens {
        assert!(running.server.store.authenticate(token).is_ok());
        assert!(connect_async(request(&running.signaling, token))
            .await
            .is_err());
    }
    let (mut socket, _) = connect_async(request(&running.signaling, &running.token))
        .await
        .unwrap();
    socket.close(None).await.unwrap();
    running.server.shutdown().await.unwrap();
}
