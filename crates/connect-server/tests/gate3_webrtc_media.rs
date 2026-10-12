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
use gate3_support::{webrtc::AuthenticatedSignaler, FakeVapi};
use rvoip_webrtc::{
    media::send_fixture_media_burst, CallTarget, SessionMedium, WebRtcClient, WebRtcConfig,
};
use std::time::Duration;

#[tokio::test]
async fn inbound_webrtc_uses_configured_authority_and_bridges_vapi_audio() {
    let _ = tracing_subscriber::fmt::try_init();
    let mut fake = FakeVapi::start().await;
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let store = Store::open(&config.database, config.limits.clone()).unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let operator = store
        .issue(&app.identity_id, Scope::operator(), expiry)
        .unwrap();
    let app_key = store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let conversation = match store
        .execute(
            &app_key.id,
            &Operation::Create {
                request_id: "webrtc-create".into(),
                participants: vec![agent.identity_id.clone()],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    store
        .execute(
            &app_key.id,
            &Operation::Send {
                request_id: "webrtc-context".into(),
                conversation_id: conversation.id.clone(),
                body: "The browser route fact is violet-six.".into(),
            },
        )
        .unwrap();
    store
        .configure_voice_route(
            &operator.id,
            &VoiceRoute {
                id: "webrtc-vapi-route".into(),
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
            credential_id: app_key.id.clone(),
            conversation_id: conversation.id.clone(),
            assistant_identity_id: agent.identity_id.clone(),
            session_id: None,
        },
    });
    drop(store);

    let server = RunningServer::start_with_vapi_media(config, None, fake.adapter.clone())
        .await
        .unwrap();
    let signaling = format!("ws://{}", server.webrtc_address.unwrap());
    let client = WebRtcClient::connect(WebRtcConfig::loopback(), &signaling)
        .await
        .unwrap();
    let signaler = AuthenticatedSignaler::new(signaling, app_key.token.clone());
    let call = client
        .call(
            &signaler,
            CallTarget::Uri("connect".into()),
            SessionMedium::Audio,
        )
        .await
        .unwrap();
    call.wait_connected(Duration::from_secs(10)).await.unwrap();
    signaler.wait_ready().await;
    let context = fake.next("add-message").await.to_string();
    assert!(context.contains("violet-six"));

    send_fixture_media_burst(call.peer(), false).await;
    assert_eq!(fake.next("audio").await["size"], 160);
    assert!(call
        .peer()
        .wait_remote_track(Duration::from_secs(5))
        .await
        .is_some());

    signaler.end_route(call.connection_id().as_str()).await;
    call.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let context = match server
                .store
                .execute(
                    &app_key.id,
                    &Operation::ContextView {
                        conversation_id: conversation.id.clone(),
                        limit: 100,
                    },
                )
                .unwrap()
            {
                Outcome::ContextView(value) => value,
                other => panic!("expected context, got {other:?}"),
            };
            if context
                .items
                .iter()
                .any(|item| item.kind == "session_ended")
            {
                assert!(context
                    .items
                    .iter()
                    .any(|item| item.kind == "voice_transcript"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(!conversation.closed, "a WebRTC leg keeps its conversation");
    server.shutdown().await.unwrap();
}
