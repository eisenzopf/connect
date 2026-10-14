//! Genuine WebRTC route regression for public selected invitations. Fixture
//! RTP is transport proof only; independently decoded qualification is separate.
#![allow(clippy::unwrap_used)]
#[path = "gate5_support/video.rs"]
mod video;
use connect_protocol::{Operation, Outcome};
use futures_util::{SinkExt, StreamExt};
use rvoip_webrtc::{
    media::send_fixture_media_burst,
    peer::session::{PeerRole, RvoipPeerConnection},
    WebRtcConfig,
};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
    MaybeTlsStream, WebSocketStream,
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
async fn selected_peer(
    url: &str,
    token: &str,
    cid: &str,
    sid: &str,
) -> (Arc<RvoipPeerConnection>, Socket) {
    let peer = RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .unwrap();
    peer.add_local_audio_track().await.unwrap();
    peer.add_local_video_track().await.unwrap();
    let offer = peer.create_offer_and_gather().await.unwrap();
    let mut request = url.into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        HeaderValue::from_str(&format!("rvoip.webrtc.v1, token.{token}")).unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"offer-ready","sdp":offer,
        "request_id":uuid::Uuid::new_v4().to_string(),"conversation_id":cid,"session_id":sid})
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let message = socket.next().await.unwrap().unwrap();
            if let Message::Text(body) = message {
                let value: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_ne!(value["type"], "rejected");
                if value["type"] == "answer" {
                    peer.set_remote_answer(value["sdp"].as_str().unwrap())
                        .await
                        .unwrap();
                }
                if value["type"] == "ready" {
                    break;
                }
            }
        }
        peer.wait_connected(Duration::from_secs(5)).await.unwrap();
    })
    .await
    .unwrap();
    (peer, socket)
}
#[tokio::test]
async fn fresh_selected_invitation_routes_both_members_after_explicit_session_end_without_restart()
{
    let mut f = video::Fixture::new().await;
    f.left
        .request(Operation::SessionEnd {
            request_id: "browser-end-initial".into(),
            conversation_id: f.conversation.clone(),
            session_id: f.session.clone(),
            reason: None,
        })
        .await
        .unwrap();
    let Outcome::Conversation(conversation) = f
        .right
        .request(Operation::Create {
            request_id: "browser-next-conversation".into(),
            participants: vec![f.left_id.clone()],
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let Outcome::Session(session) = f
        .right
        .request(Operation::SessionInvite {
            request_id: "browser-next-invite".into(),
            conversation_id: conversation.id.clone(),
            participant_id: f.left_id.clone(),
            media: vec!["audio".into(), "video".into()],
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    f.left
        .request(Operation::SessionAccept {
            request_id: "browser-next-accept".into(),
            conversation_id: conversation.id.clone(),
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    for (client, peer) in [(&mut f.left, &f.right_id), (&mut f.right, &f.left_id)] {
        let Outcome::BrowserBinding(binding) = client
            .request(Operation::BrowserBinding {
                conversation_id: Some(conversation.id.clone()),
                session_id: Some(session.id.clone()),
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(&binding.participant_id, peer);
        assert_eq!(binding.session_id.as_deref(), Some(session.id.as_str()));
    }
    let url = format!("ws://{}", f.server.webrtc_address.unwrap());
    // Recipient joins first: projection must retain Session participant IDs,
    // not duplicate the authenticated caller as both ends of the pair.
    let (left, left_socket) =
        selected_peer(&url, &f.left_token, &conversation.id, &session.id).await;
    let (right, right_socket) =
        selected_peer(&url, &f.right_token, &conversation.id, &session.id).await;
    tokio::join!(
        send_fixture_media_burst(&left, true),
        send_fixture_media_burst(&right, true)
    );
    for peer in [&left, &right] {
        for track in [
            peer.wait_remote_track(Duration::from_secs(5)).await,
            peer.wait_remote_video_track(Duration::from_secs(5)).await,
        ] {
            let track = track.unwrap();
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
            .unwrap();
        }
    }
    let Outcome::Health {
        readiness: Some(readiness),
        ..
    } = f.left.request(Operation::Health {}).await.unwrap()
    else {
        panic!()
    };
    let media = readiness.media.unwrap();
    assert!(media
        .forwarding
        .iter()
        .any(|stage| stage.stage == "core-video-bridge" && stage.samples > 0));
    assert!(media
        .queues
        .iter()
        .filter(|stage| stage.kind == "video")
        .all(|stage| stage.capacity_packets <= 64
            && stage.sampled_high_water_packets <= stage.capacity_packets));
    let diagnostic = serde_json::to_string(&media).unwrap();
    for secret in [
        &f.left_token,
        &f.right_token,
        &conversation.id,
        &session.id,
        &f.left_id,
        &f.right_id,
    ] {
        assert!(!diagnostic.contains(secret));
    }
    f.left
        .request(Operation::SessionEnd {
            request_id: "browser-end-selected".into(),
            conversation_id: conversation.id.clone(),
            session_id: session.id.clone(),
            reason: None,
        })
        .await
        .unwrap();
    drop(left_socket);
    drop(right_socket);
    left.close().await.unwrap();
    right.close().await.unwrap();
    let Outcome::Accepted {
        conversation_id, ..
    } = f
        .right
        .request(Operation::Send {
            request_id: "browser-context-after-end".into(),
            conversation_id: conversation.id.clone(),
            body: "Retained selected Conversation after media end".into(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(conversation_id, conversation.id);
    f.left.close().await;
    f.right.close().await;
    tokio::time::timeout(Duration::from_secs(5), f.server.shutdown())
        .await
        .unwrap()
        .unwrap();
}
