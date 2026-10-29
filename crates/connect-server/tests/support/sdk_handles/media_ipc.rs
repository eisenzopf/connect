//! Actual managed Client attachment, media custody and control under backpressure.
use super::*;
use connect_client::sdk::helper::{self, media, media_ipc as wire};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    net::TcpListener,
};

struct Peer {
    reader: wire::Reader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    sequence: u32,
}
impl Peer {
    async fn send(&mut self, command: Value, payload: Vec<u8>) -> u32 {
        self.sequence += 1;
        wire::write(
            &mut self.writer,
            &wire::Frame {
                kind: wire::Kind::Request,
                request_id: self.sequence,
                metadata: serde_json::to_vec(&json!({"timeout_ms":60_000,"command":command}))
                    .unwrap(),
                payload,
            },
        )
        .await
        .unwrap();
        self.sequence
    }
    async fn next(&mut self) -> wire::Frame {
        tokio::time::timeout(Duration::from_secs(4), self.reader.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }
    async fn request(&mut self, command: Value, payload: Vec<u8>) -> (wire::Kind, Value) {
        let id = self.send(command, payload).await;
        let frame = self.next().await;
        assert_eq!(frame.request_id, id);
        assert!(frame.payload.is_empty());
        (frame.kind, serde_json::from_slice(&frame.metadata).unwrap())
    }
}
fn request(action: Value) -> helper::Request {
    serde_json::from_value(json!({"timeout_ms":4000,"action":action})).unwrap()
}
async fn accept(listener: &TcpListener, token: &str, reply_token: &str) -> Peer {
    let (socket, address) = listener.accept().await.unwrap();
    assert!(address.ip().is_loopback());
    let (input, mut output) = socket.into_split();
    let mut reader = wire::Reader::new(input);
    let frame = reader.next().await.unwrap().unwrap();
    assert_eq!(frame.kind, wire::Kind::Request);
    assert_eq!(frame.request_id, 1);
    assert!(frame.payload.is_empty());
    let hello: Value = serde_json::from_slice(&frame.metadata).unwrap();
    assert_eq!(hello, json!({"profile":media::PROFILE,"token":token}));
    wire::write(
        &mut output,
        &wire::Frame {
            kind: wire::Kind::Response,
            request_id: 1,
            metadata: serde_json::to_vec(&json!({"profile":media::PROFILE,"token":reply_token}))
                .unwrap(),
            payload: Vec::new(),
        },
    )
    .await
    .unwrap();
    Peer {
        reader,
        writer: output,
        sequence: 1,
    }
}
async fn attach(
    owner: &helper::ManagedClient,
    token: &str,
    reply: &str,
) -> (sdk::Result<Value>, Peer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let action = request(
        json!({"type":"media_attach","attachment":{"endpoint":listener.local_addr().unwrap(),"token":token}}),
    );
    let (attached, peer) = tokio::join!(owner.execute(action), accept(&listener, token, reply));
    (attached, peer)
}

#[tokio::test]
async fn managed_media_channel_authenticates_original_custody_bounds_inflight_and_closes_while_control_lives(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        super::super::canonical_native_connections::ready_endpoint_with_capabilities(
            &f,
            vec!["audio".into()],
        )
        .await;
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = client
            .create_conversation(
                vec![],
                ConversationPolicy::Persistent,
                mutation("ipc-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let session = room
            .invite(
                "ipc-session".parse().unwrap(),
                vec![],
                sdk::SessionMedium::Voice,
                "synthetic helper IPC".into(),
                json!({"audio":["g.711-mu"]}),
                mutation("ipc-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let offer = session
            .offer_connection(
                "ipc-native".parse().unwrap(),
                NonEmpty {
                    first: sdk::OfferedStream {
                        id: "ipc-audio".parse().unwrap(),
                        kind: sdk::Medium::Audio,
                        direction: sdk::Direction::Sendrecv,
                        codecs: NonEmpty {
                            first: "g.711-mu".into(),
                            rest: vec![],
                        },
                    },
                    rest: vec![],
                },
                mutation("ipc-offer"),
            )
            .await
            .unwrap();
        let saved = offer.reference_json();
        let connection = offer.wait(options()).await.unwrap().value;
        assert!(connection.native_negotiation().unwrap().is_some());
        let owner = helper::ManagedClient::new(client.clone());
        let token = "a".repeat(64);
        let bad = request(
            json!({"type":"media_attach","attachment":{"endpoint":"192.0.2.1:443","token":token}}),
        );
        assert!(matches!(owner.execute(bad).await,Err(e) if e.kind==sdk::ErrorKind::Validation));
        let (bad, mut refused) = attach(&owner, &token, &"b".repeat(64)).await;
        assert!(matches!(bad,Err(e) if e.kind==sdk::ErrorKind::Authentication));
        assert!(
            tokio::time::timeout(Duration::from_secs(1), refused.reader.next())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        drop(refused);
        let (attached, mut peer) = attach(&owner, &token, &token).await;
        let attached = attached.unwrap();
        assert_eq!(attached["window"], 16);
        assert_eq!(attached["profile"], media::PROFILE);
        let binding = json!({"conversation_id":room.id(),"session_id":session.id(),"connection_id":connection.id()});
        let (kind, selected) = peer
            .request(json!({"action":"describe","binding":binding}), Vec::new())
            .await;
        assert_eq!(kind, wire::Kind::Response);
        assert_eq!(selected["streams"].as_array().unwrap().len(), 1);
        assert_eq!(selected["streams"][0]["id"], "ipc-audio");
        assert_eq!(
            selected["stream_revision"],
            connection
                .native_negotiation()
                .unwrap()
                .unwrap()
                .stream_revision
                .get()
        );
        let (kind, invalid) = peer
            .request(
                json!({"action":"stats","credential":"must-refuse"}),
                Vec::new(),
            )
            .await;
        assert_eq!(kind, wire::Kind::Failure);
        assert_eq!(invalid["kind"], "validation");
        let (kind,invalid)=peer.request(json!({"action":"send_rtp","binding":binding,"stream_id":"ipc-audio","datagram_sequence":9}),vec![1]).await;
        assert_eq!(kind, wire::Kind::Failure);
        assert_eq!(invalid["kind"], "validation");
        // The same credential and saved operation still cannot restore another
        // physical Client's hop handles or binding ownership.
        let fresh = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let recovered = fresh
            .restore(saved, fresh.operation_kinds().connection_offer)
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert!(recovered.value.native_negotiation().unwrap().is_none());
        let observer = helper::ManagedClient::new(fresh);
        let (attached, mut alien) = attach(&observer, &token, &token).await;
        attached.unwrap();
        let (kind, denied) = alien
            .request(json!({"action":"describe","binding":binding}), Vec::new())
            .await;
        assert_eq!(kind, wire::Kind::Failure);
        assert_eq!(denied["kind"], "unsupported");
        observer.close(options()).await.unwrap();
        drop(alien);
        // Sixteen actual receive waits fill the media actor; JSON control has
        // its own path and shutdown does not wait for their60-second deadlines.
        for _ in 0..16 {
            peer.send(
                json!({"action":"receive","binding":binding,"stream_id":"ipc-audio"}),
                Vec::new(),
            )
            .await;
        }
        let (kind, bounded) = peer.request(json!({"action":"stats"}), Vec::new()).await;
        assert_eq!(kind, wire::Kind::Failure);
        assert_eq!(bounded["kind"], "application");
        assert_eq!(bounded["certainty"], "not-submitted");
        owner
            .execute(request(json!({"type":"capabilities_refresh"})))
            .await
            .unwrap();
        assert_eq!(
            offer.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        let started = tokio::time::Instant::now();
        let closed = owner
            .execute(request(json!({"type":"media_detach"})))
            .await
            .unwrap();
        assert_eq!(closed["closed"], true);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            tokio::time::timeout(Duration::from_secs(1), peer.reader.next())
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        drop(peer);
        owner.close(options()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let view = f.server.service.canonical_runtime_snapshot().unwrap();
                if view.physical_peers == 0 && view.media_readers == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!(
            "managed-media-channel-witness {}",
            json!({"profile":media::PROFILE,"algorithm":algorithm,
            "wrong_attachment_refused":true,"remote_attachment_refused":true,"original_binding_observed":true,
            "restored_foreign_custody_refused":true,"invalid_packet_and_metadata_refused":true,
            "inflight_bounded":16,"control_live_while_media_reads_blocked":true,"detach_cancelled_and_joined_reads":true,
            "server_joined":true,"physical_peers_drained":true,"media_readers_drained":true})
        );
    }
}
