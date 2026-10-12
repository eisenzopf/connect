#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

mod gate3_support;

use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, SipMediaConfig, WebRtcMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store};
use gate3_support::webrtc::AuthenticatedSignaler;
use rvoip_sip::{AudioFrame, Config as SipConfig, StreamPeer};
use rvoip_webrtc::{
    media::send_fixture_media_burst, CallTarget, SessionMedium, WebRtcClient, WebRtcConfig,
};
use std::time::Duration;

fn free_udp_address() -> std::net::SocketAddr {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap()
}

#[tokio::test]
async fn browser_and_sip_share_one_durable_session_and_duplex_media_bridge() {
    let _ = tracing_subscriber::fmt::try_init();
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let store = Store::open(&config.database, config.limits.clone()).unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3_600;
    let app_key = store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(
            &agent.identity_id,
            Scope::communications(vec![app.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let conversation = match store
        .execute(
            &app_key.id,
            &Operation::Create {
                request_id: "interop-create".into(),
                participants: vec![agent.identity_id.clone()],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    let session = match store
        .execute(
            &app_key.id,
            &Operation::SessionInvite {
                request_id: "interop-invite".into(),
                conversation_id: conversation.id.clone(),
                participant_id: agent.identity_id.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    {
        Outcome::Session(value) => value,
        other => panic!("expected session, got {other:?}"),
    };
    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "interop-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    let route = InboundMediaRoute {
        credential_id: app_key.id.clone(),
        conversation_id: conversation.id.clone(),
        assistant_identity_id: agent.identity_id,
        session_id: Some(session.id.clone()),
    };
    config.sip = Some(SipMediaConfig {
        bind: free_udp_address(),
        media_bind_ip: None,
        advertised: None,
        media_public: None,
        outbound: None,
        route: route.clone(),
    });
    config.webrtc = Some(WebRtcMediaConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        route,
    });
    drop(store);

    let sip_peer_address = free_udp_address();
    let mut sip_peer =
        StreamPeer::with_config(SipConfig::local("interop-peer", sip_peer_address.port()))
            .await
            .unwrap();
    let server = RunningServer::start_local(config, None, false)
        .await
        .unwrap();
    app.endpoint = server.address;
    app.credential_id = app_key.id.clone();
    app.credential = app_key.token.clone();
    let mut client = ConnectClient::open(app).await.unwrap();

    let signaling_url = format!("ws://{}", server.webrtc_address.unwrap());
    let browser = WebRtcClient::connect(WebRtcConfig::loopback(), &signaling_url)
        .await
        .unwrap();
    let signaler = AuthenticatedSignaler::new(signaling_url.clone(), app_key.token.clone());
    let browser_call = browser
        .call(
            &signaler,
            CallTarget::Uri("connect".into()),
            SessionMedium::Audio,
        )
        .await
        .unwrap();
    browser_call
        .wait_connected(Duration::from_secs(10))
        .await
        .unwrap();
    signaler.wait_ready().await;

    let sip_leg = client
        .sip_dial(
            "interop-sip-dial",
            &conversation.id,
            &session.id,
            &format!("sip:peer@{sip_peer_address}"),
        )
        .await
        .unwrap();
    let incoming = tokio::time::timeout(Duration::from_secs(10), sip_peer.wait_for_incoming())
        .await
        .unwrap()
        .unwrap();
    let sip_call = incoming.accept().await.unwrap();
    let (sip_sender, mut sip_receiver) = sip_call.audio().await.unwrap().split();

    send_fixture_media_burst(browser_call.peer(), false).await;
    let browser_to_sip = tokio::time::timeout(Duration::from_secs(5), sip_receiver.recv())
        .await
        .unwrap()
        .expect("browser media should reach SIP");
    assert_eq!(browser_to_sip.sample_rate, 8_000);
    assert!(!browser_to_sip.samples.is_empty());

    for sequence in 0..8 {
        sip_sender
            .send(AudioFrame::new(vec![900; 160], 8_000, 1, sequence * 160))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let remote_track = browser_call
        .peer()
        .wait_remote_track(Duration::from_secs(5))
        .await
        .expect("SIP media should create a browser audio track");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if remote_track
                .poll()
                .await
                .is_some_and(|event| format!("{event:?}").starts_with("OnRtpPacket"))
            {
                break;
            }
        }
    })
    .await
    .expect("SIP RTP should reach the browser track");

    sip_call.hangup().await.unwrap();
    signaler.wait_remote_end().await;
    let terminal = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let leg = client
                .sip_status(&conversation.id, &sip_leg.id)
                .await
                .unwrap();
            if matches!(
                leg.state.as_str(),
                "ended" | "failed" | "cancelled" | "uncertain"
            ) {
                break leg;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(terminal.state, "ended", "remote BYE is a normal SIP ending");

    browser_call.close().await.unwrap();
    client.close().await;
    sip_peer.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn browser_and_native_quic_share_one_durable_session_and_duplex_media_bridge() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let store = Store::open(&config.database, config.limits.clone()).unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3_600;
    let app_key = store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(
            &agent.identity_id,
            Scope::communications(vec![app.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let conversation = match store
        .execute(
            &app_key.id,
            &Operation::Create {
                request_id: "native-interop-create".into(),
                participants: vec![agent.identity_id.clone()],
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    let session = match store
        .execute(
            &app_key.id,
            &Operation::SessionInvite {
                request_id: "native-interop-invite".into(),
                conversation_id: conversation.id.clone(),
                participant_id: agent.identity_id.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    {
        Outcome::Session(value) => value,
        other => panic!("expected session, got {other:?}"),
    };
    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "native-interop-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    config.webrtc = Some(WebRtcMediaConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        route: InboundMediaRoute {
            credential_id: app_key.id.clone(),
            conversation_id: conversation.id.clone(),
            assistant_identity_id: agent.identity_id.clone(),
            session_id: Some(session.id.clone()),
        },
    });
    let restart_config = config.clone();
    drop(store);

    let server = RunningServer::start_local(config, None, false)
        .await
        .unwrap();
    app.endpoint = server.address;
    app.credential_id = app_key.id.clone();
    app.credential = app_key.token.clone();
    let mut native = ConnectClient::open(app).await.unwrap();
    let mut native_audio = native
        .open_native_audio(&conversation.id, &session.id, &agent.identity_id)
        .await
        .unwrap();

    let signaling_url = format!("ws://{}", server.webrtc_address.unwrap());
    let browser = WebRtcClient::connect(WebRtcConfig::loopback(), &signaling_url)
        .await
        .unwrap();
    let signaler = AuthenticatedSignaler::new(signaling_url.clone(), app_key.token.clone());
    let browser_call = browser
        .call(
            &signaler,
            CallTarget::Uri("connect".into()),
            SessionMedium::Audio,
        )
        .await
        .unwrap();
    browser_call
        .wait_connected(Duration::from_secs(10))
        .await
        .unwrap();
    signaler.wait_ready().await;

    for _ in 0..4 {
        native_audio.send_pcmu(&[0x2a; 160]).await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let remote_track = browser_call
        .peer()
        .wait_remote_track(Duration::from_secs(5))
        .await
        .expect("native QUIC media should create a browser audio track");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if remote_track
                .poll()
                .await
                .is_some_and(|event| format!("{event:?}").starts_with("OnRtpPacket"))
            {
                break;
            }
        }
    })
    .await
    .expect("native QUIC audio should reach the browser");

    send_fixture_media_burst(browser_call.peer(), false).await;
    let browser_to_native = native_audio
        .receive(Duration::from_secs(5))
        .await
        .expect("browser audio should reach native QUIC");
    assert!(!browser_to_native.payload.is_empty());

    signaler
        .end_route(browser_call.connection_id().as_str())
        .await;
    browser_call.close().await.unwrap();
    let _ = native.close_native_audio(&native_audio).await;

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if native
                .session_get(&conversation.id, &session.id)
                .await
                .unwrap()
                .state
                == "accepted"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("paired native and browser bindings should both leave the Session");

    native.close().await;
    server.shutdown().await.unwrap();

    let restarted = RunningServer::start_local(restart_config, None, false)
        .await
        .unwrap();
    let mut replacement_config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    replacement_config.endpoint = restarted.address;
    replacement_config.credential_id = app_key.id.clone();
    replacement_config.credential = app_key.token.clone();
    let mut replacement_native = ConnectClient::open(replacement_config).await.unwrap();
    let mut replacement_audio = replacement_native
        .open_native_audio(&conversation.id, &session.id, &agent.identity_id)
        .await
        .unwrap();
    let signaling_url = format!("ws://{}", restarted.webrtc_address.unwrap());
    let replacement_browser = WebRtcClient::connect(WebRtcConfig::loopback(), &signaling_url)
        .await
        .unwrap();
    let replacement_signaler = AuthenticatedSignaler::new(signaling_url, app_key.token);
    let replacement_call = replacement_browser
        .call(
            &replacement_signaler,
            CallTarget::Uri("connect".into()),
            SessionMedium::Audio,
        )
        .await
        .unwrap();
    replacement_call
        .wait_connected(Duration::from_secs(10))
        .await
        .unwrap();
    replacement_signaler.wait_ready().await;
    send_fixture_media_burst(replacement_call.peer(), false).await;
    assert!(!replacement_audio
        .receive(Duration::from_secs(5))
        .await
        .unwrap()
        .payload
        .is_empty());
    replacement_signaler
        .end_route(replacement_call.connection_id().as_str())
        .await;
    replacement_call.close().await.unwrap();
    let _ = replacement_native
        .close_native_audio(&replacement_audio)
        .await;
    replacement_native.close().await;
    restarted.shutdown().await.unwrap();
}
