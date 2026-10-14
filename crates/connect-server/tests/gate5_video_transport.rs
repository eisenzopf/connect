//! Transport regression only: fixture VP8 bytes are not decoded-frame qualification.
#![allow(clippy::unwrap_used)]
use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome};
use connect_server::{
    cli::init,
    config::{InboundMediaRoute, ServerConfig, WebRtcMediaConfig},
    runtime::RunningServer,
};
use connect_store::{Scope, Store};
use futures_util::{SinkExt, StreamExt};
use rvoip_webrtc::{
    media::send_fixture_media_burst, CallTarget, SessionMedium, WebRtcClient, WebRtcConfig,
};
use std::time::Duration;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};
mod gate3_support;
use gate3_support::webrtc::AuthenticatedSignaler;

async fn reject_video_offer(url: &str, token: &str, sdp: &str) {
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rvoip.webrtc.v1, token.{token}")).unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"offer-ready","sdp":sdp,"request_id":"bad-profile"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(message) = socket.next().await {
            let Ok(message) = message else { return };
            if message.is_close() {
                return;
            }
            if message.is_text() {
                let value: serde_json::Value =
                    serde_json::from_str(message.to_text().unwrap()).unwrap();
                assert!(
                    !matches!(value["type"].as_str(), Some("answer" | "ready")),
                    "invalid video offer must not start media"
                );
                if value["type"] == "rejected" {
                    return;
                }
            }
        }
    })
    .await
    .expect("unsupported profile must be rejected promptly");
}

#[test]
fn normalized_video_strips_source_extensions_and_keeps_rtp_marker_and_sequence() {
    // One CSRC, one RFC 8285 extension, and a two-byte VP8 payload.
    let wire = vec![
        0x91, 0xe0, 0xff, 0xff, 0, 1, 0x5f, 0x90, 0, 0, 0, 7, 0, 0, 0, 99, 0xbe, 0xde, 0, 1, 0x10,
        42, 0, 0, 0x10, 1,
    ];
    let frame = rvoip_core::stream::MediaFrame {
        source_clock: None,
        stream_id: rvoip_core::ids::StreamId::new(),
        kind: rvoip_core::stream::StreamKind::Video,
        payload: wire.into(),
        timestamp_rtp: 90_000,
        captured_at: chrono::Utc::now(),
        payload_type: Some(96),
    };
    let output = rvoip_core::video_bridge::normalize_packet(&frame, 9, 100).unwrap();
    let observation = rvoip_uctp::substrate::observe_rtp(output).unwrap();
    let packet = observation.packet();
    assert!(packet.header.marker);
    assert_eq!(packet.header.sequence_number, u16::MAX);
    assert_eq!(packet.header.timestamp, 90_000);
    assert_eq!(packet.header.ssrc, 9);
    assert_eq!(packet.header.payload_type, 100);
    assert!(packet.header.csrc.is_empty());
    assert!(!packet.header.extension);
    assert!(packet.header.extensions.is_none());
    assert_eq!(packet.payload.as_ref(), &[0x10, 1]);
}

#[tokio::test]
async fn two_authenticated_webrtc_participants_exchange_video_and_audio_through_connect() {
    let _ = tracing_subscriber::fmt().with_env_filter("warn").try_init();
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
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
                media: vec!["audio".into(), "video".into()],
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
    drop(store);
    let server = RunningServer::start_local(config, None, false)
        .await
        .unwrap();
    let url = format!("ws://{}", server.webrtc_address.unwrap());
    let exact = "m=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=rtpmap:0 PCMU/8000\r\nm=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\na=sendrecv\r\n";
    for invalid in [
        exact.replace("VP8/90000", "H264/90000"),
        format!("{exact}m=video 9 UDP/TLS/RTP/SAVPF 96\r\na=rtpmap:96 VP8/90000\r\n"),
        exact.replace("m=video 9", "m=video 0"),
    ] {
        reject_video_offer(&url, &app_key.token, &invalid).await;
    }
    let left = WebRtcClient::connect(WebRtcConfig::loopback(), &url)
        .await
        .unwrap();
    let right = WebRtcClient::connect(WebRtcConfig::loopback(), &url)
        .await
        .unwrap();
    let ls = AuthenticatedSignaler::new(url.clone(), app_key.token.clone());
    let rs = AuthenticatedSignaler::new(url, agent_key.token.clone());
    let lc = left
        .call(
            &ls,
            CallTarget::Uri("connect".into()),
            SessionMedium::AudioVideo,
        )
        .await
        .unwrap();
    lc.wait_connected(Duration::from_secs(10)).await.unwrap();
    ls.wait_ready().await;
    let rc = right
        .call(
            &rs,
            CallTarget::Uri("connect".into()),
            SessionMedium::AudioVideo,
        )
        .await
        .unwrap();
    rc.wait_connected(Duration::from_secs(10)).await.unwrap();
    rs.wait_ready().await;
    tokio::join!(
        send_fixture_media_burst(lc.peer(), true),
        send_fixture_media_burst(rc.peer(), true)
    );
    for peer in [lc.peer(), rc.peer()] {
        for track in [
            peer.wait_remote_track(Duration::from_secs(5)).await,
            peer.wait_remote_video_track(Duration::from_secs(5)).await,
        ] {
            let track = track.expect("Connect must deliver both media kinds in each direction");
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if track
                        .poll()
                        .await
                        .is_some_and(|event| format!("{event:?}").starts_with("OnRtpPacket"))
                    {
                        break;
                    }
                }
            })
            .await
            .expect("real RTP packet must arrive through Connect bridge");
        }
    }
    // A downstream receiver's PLI must cross Connect back to the exact
    // upstream camera, rather than stopping at the server's outgoing track.
    let upstream_feedback = rc.peer().video_keyframe_requests();
    lc.peer().request_video_keyframe().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), upstream_feedback.notified())
        .await
        .expect("Connect must proxy the receiver's keyframe request to its camera source");
    ls.end_route(lc.connection_id().as_str()).await;
    rs.wait_remote_end().await;
    lc.close().await.unwrap();
    rc.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), server.shutdown())
        .await
        .unwrap()
        .unwrap();
}

#[path = "gate5_support/video.rs"]
mod accepted_video;

#[tokio::test]
async fn shutdown_joins_incomplete_browser_av_peers_and_native_attachment_within_five_seconds() {
    use rvoip_webrtc::peer::session::{PeerRole, RvoipPeerConnection};
    let _ = tracing_subscriber::fmt()
        .with_env_filter("connect_server::runtime=debug,rvoip_webrtc=warn")
        .try_init();
    let mut f = accepted_video::Fixture::new().await;
    let native = f
        .left
        .open_native_video(&f.conversation, &f.session, &f.right_id)
        .await
        .unwrap();
    let mut sockets = Vec::new();
    let mut peers = Vec::new();
    for token in [&f.left_token, &f.right_token] {
        let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
            .await
            .unwrap();
        peer.add_local_audio_track().await.unwrap();
        peer.add_local_video_track().await.unwrap();
        let offer = peer.create_offer_and_gather().await.unwrap();
        // Negotiate genuine AV but never apply the answer or establish ICE.
        let offer = offer
            .lines()
            .filter(|line| !line.starts_with("a=candidate:"))
            .collect::<Vec<_>>()
            .join("\r\n")
            + "\r\n";
        let mut request = format!("ws://{}", f.server.webrtc_address.unwrap())
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            HeaderValue::from_str(&format!("rvoip.webrtc.v1, token.{token}")).unwrap(),
        );
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket.send(Message::Text(serde_json::json!({"type":"offer-ready", "sdp":offer,"request_id":uuid::Uuid::new_v4().to_string()}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Text(body) = message {
                    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
                    assert_ne!(value["type"], "rejected");
                    if value["type"] == "answer" {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        sockets.push(socket);
        peers.push(peer);
    }
    let shutdown = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(5), f.server.shutdown())
        .await
        .expect("incomplete AV cleanup must respect five seconds")
        .unwrap();
    assert!(shutdown.elapsed() < Duration::from_secs(5));
    tokio::time::timeout(Duration::from_secs(1), async {
        while !native.is_ended() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(sockets);
    for peer in peers {
        peer.close().await.unwrap();
    }
    f.left.close().await;
    f.right.close().await;
}
