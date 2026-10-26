//! Real loopback ICE/DTLS and Store fences; no signaling facade or media proof.
#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::{
    connection_control::OfferIntent,
    provisioning::*,
    session_control::{Action, Request},
    Limits, Operation, Outcome,
};
use connect_store::{AuthorityWait, PeerAuthorityRequest, PeerCommand, PeerDeviceClaim, Scope};
use serde_json::json;

struct Fixture {
    _temp: tempfile::TempDir,
    store: Arc<Store>,
    origin: Arc<NativeMediaOrigin>,
    admin: String,
    endpoint: String,
    attempt: Option<NativeOfferLease>,
    credential: String,
}
impl Fixture {
    fn new(sdp: String) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&temp.path().join("webrtc.sqlite3"), Limits::default()).unwrap(),
        );
        let identity = store.create_identity("owner").unwrap().id;
        let now = chrono::Utc::now().timestamp();
        let admin = store
            .issue(&identity, Scope::operator(), now + 3600)
            .unwrap()
            .id;
        let key = store
            .issue(
                &identity,
                Scope::collaboration(vec!["*".into()]),
                now + 3600,
            )
            .unwrap()
            .id;
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Endpoint {
                    operation_id: "endpoint".into(),
                    identity_id: identity.clone(),
                    device: DeviceSpec {
                        id: "device".into(),
                        kind: "desktop".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Webrtc,
                    address: "https://localhost/webrtc".into(),
                    capabilities: vec!["audio".into()],
                    secret_ref: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: "verify".into(),
                    endpoint_id: endpoint_id.clone(),
                    expected_revision: 0,
                    evidence_ref: "controlled-store-fixture".into(),
                },
            )
            .unwrap();
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::SigningKey {
                    operation_id: "signer".into(),
                    identity_id: identity,
                    credential_id: key.clone(),
                    device_id: "device".into(),
                    key_id: "signer".into(),
                    algorithm: SigningAlgorithm::EdDSA,
                    public_key: URL_SAFE_NO_PAD.encode(pair.public_key_raw()),
                    not_before: now - 1,
                    expires_at: now + 600,
                    expected_revision: None,
                },
            )
            .unwrap();
        let Outcome::Conversation(room) = store
            .execute(
                &key,
                &Operation::Create {
                    request_id: "room".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        store
            .execute(
                &key,
                &Operation::SessionControl {
                    request: Request {
                        operation_id: "solo-session".into(),
                        conversation_id: room.id.clone(),
                        session_id: "session".into(),
                        by_participant: None,
                        expected_revision: None,
                        action: Action::Invite {
                            participants: vec![],
                            medium: "voice".into(),
                            intent: "conference".into(),
                            capabilities: json!({"audio":["opus"]}),
                        },
                    },
                },
            )
            .unwrap();
        let request = PeerAuthorityRequest {
            credentials: vec![key.clone()],
            device: PeerDeviceClaim {
                id: "device".into(),
                kind: "desktop".into(),
                platform: "fixture".into(),
            },
            key_id: "signer".into(),
            algorithm: "EdDSA".into(),
            command: PeerCommand::NativeConnectionOffer(OfferIntent {
                operation_id: "original-offer".into(),
                conversation_id: room.id,
                session_id: "session".into(),
                connection_id: "connection".into(),
                by_participant: None,
                expected_revision: Some(0),
                offer: json!({"substrate":"websocket+webrtc","capabilities":{"audio":["opus"]},"streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]}],"substrate_setup":{"kind":"websocket+webrtc","sdp":sdp}}),
            }),
        };
        let proof = store
            .admit_peer_authority(&request, &AuthorityWait::new(Duration::from_secs(2)))
            .unwrap();
        store
            .prepare_native_offer_with_peer(&key, &proof, Some(EndpointTransport::Webrtc))
            .unwrap();
        let (lease, _) = store
            .begin_native_offer_with_peer(&key, &proof)
            .unwrap()
            .unwrap();
        let origin = store
            .native_media_origin_with_peer(&key, &proof, &lease)
            .unwrap();
        drop(proof);
        Self {
            _temp: temp,
            store,
            origin,
            admin,
            endpoint: endpoint_id,
            attempt: Some(lease),
            credential: key,
        }
    }
}
async fn offerer() -> Arc<RvoipPeerConnection> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    RvoipPeerConnection::new(&WebRtcConfig::loopback(), PeerRole::Offerer)
        .await
        .unwrap()
}
async fn available(semaphore: &tokio::sync::Semaphore) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while semaphore.available_permits() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn start(
    f: &mut Fixture,
    slots: &Arc<tokio::sync::Semaphore>,
    physical: &CancellationToken,
) -> Peer {
    Peer::start(
        f.store.clone(),
        f.origin.clone(),
        f.attempt.take().unwrap(),
        WebRtcConfig::loopback(),
        physical,
        slots.clone().acquire_owned().await.unwrap(),
        Instant::now() + Duration::from_secs(10),
    )
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn store_bound_peer_completes_actual_dtls_and_original_close_releases_capacity() {
    let caller = offerer().await;
    let mut f = Fixture::new(caller.create_offer_and_gather().await.unwrap());
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let physical = CancellationToken::new();
    let reserve = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let address = reserve.local_addr().unwrap();
    drop(reserve);
    let mut config = WebRtcConfig::loopback();
    config.udp_bind = address.to_string();
    let peer = Peer::start(
        f.store.clone(),
        f.origin.clone(),
        f.attempt.take().unwrap(),
        config,
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        Instant::now() + Duration::from_secs(10),
    );
    let answer = peer
        .answer_sdp(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(slots.available_permits(), 0);
    assert!(
        std::net::UdpSocket::bind(address).is_err(),
        "configured peer must actually own its UDP port"
    );
    assert!(!caller.is_connected());
    caller.set_remote_answer(&answer).await.unwrap();
    let (a, b) = tokio::join!(
        caller.wait_connected(Duration::from_secs(5)),
        peer.wait_connected(Instant::now() + Duration::from_secs(5))
    );
    a.unwrap();
    b.unwrap();
    assert!(caller.is_connected());
    assert_eq!(peer.original_attempt().operation_id(), "original-offer");
    let gate = Arc::new(CloseGate::default());
    *peer.close_gate.lock().unwrap() = Some(gate.clone());
    let mut close = Box::pin(peer.shutdown());
    assert!(futures_util::poll!(close.as_mut()).is_pending());
    drop(close);
    tokio::time::timeout(Duration::from_secs(2), gate.entered.notified())
        .await
        .unwrap();
    assert_eq!(
        slots.available_permits(),
        0,
        "pending actual close must retain capacity"
    );
    assert!(
        std::net::UdpSocket::bind(address).is_err(),
        "pending close must retain its actual UDP socket"
    );
    gate.release.notify_one();
    peer.shutdown().await.unwrap();
    peer.shutdown().await.unwrap();
    available(&slots).await;
    let reclaimed =
        std::net::UdpSocket::bind(address).expect("confirmed peer close releases its UDP port");
    drop(reclaimed);
    assert!(peer
        .answer_sdp(Instant::now() + Duration::from_secs(1))
        .await
        .is_err());
    assert!(!physical.is_cancelled());
    caller.close().await.unwrap();
    // ICE/DTLS and cleanup do not manufacture a durable media-ready outcome.
    let record = f
        .store
        .execute_operation_action(
            &f.credential,
            connect_protocol::transitions::OperationAction::Get {
                operation_id: "original-offer".into(),
            },
        )
        .unwrap();
    assert_eq!(
        record.state,
        connect_protocol::transitions::OperationState::Preparing
    );
    assert_eq!(record.effects.len(), 1);
    assert!(record.result.is_none());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn current_endpoint_revocation_retires_idle_dtls_owner_without_retaining_a_reader() {
    let caller = offerer().await;
    let mut f = Fixture::new(caller.create_offer_and_gather().await.unwrap());
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let physical = CancellationToken::new();
    let peer = start(&mut f, &slots, &physical).await;
    peer.answer_sdp(Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    f.store
        .execute_provisioning(
            &f.admin,
            ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke".into(),
                endpoint_id: f.endpoint.clone(),
                expected_revision: 1,
            },
        )
        .unwrap();
    available(&slots).await;
    assert!(peer
        .answer_sdp(Instant::now() + Duration::from_secs(1))
        .await
        .is_err());
    assert!(peer
        .wait_connected(Instant::now() + Duration::from_secs(1))
        .await
        .is_err());
    peer.shutdown().await.unwrap();
    caller.close().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_answer_wait_owner_drop_and_physical_loss_drain_original_workers() {
    for physical_loss in [false, true] {
        let caller = offerer().await;
        let mut f = Fixture::new(caller.create_offer_and_gather().await.unwrap());
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let physical = CancellationToken::new();
        let peer = start(&mut f, &slots, &physical).await;
        // Cancelling an observation does not take the original owned attempt.
        let mut answer = Box::pin(peer.answer_sdp(Instant::now() + Duration::from_secs(5)));
        let _ = futures_util::poll!(answer.as_mut());
        drop(answer);
        assert_eq!(peer.original_attempt().operation_id(), "original-offer");
        if physical_loss {
            physical.cancel();
            peer.shutdown().await.unwrap();
        } else {
            drop(peer);
        }
        available(&slots).await;
        caller.close().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn foreign_installation_unoffered_media_and_deadline_refuse_with_original_custody() {
    let caller = offerer().await;
    let sdp = caller.create_offer_and_gather().await.unwrap();
    let mut one = Fixture::new(sdp.clone());
    let two = Fixture::new(sdp);
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let physical = CancellationToken::new();
    let peer = Peer::start(
        two.store.clone(),
        one.origin.clone(),
        one.attempt.take().unwrap(),
        WebRtcConfig::loopback(),
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        Instant::now() + Duration::from_secs(10),
    );
    assert_eq!(
        peer.answer_sdp(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap_err()
            .code,
        "forbidden"
    );
    peer.shutdown().await.unwrap();
    available(&slots).await;
    caller.close().await.unwrap();
    let av = offerer().await;
    av.add_local_video_track().await.unwrap();
    let mut f = Fixture::new(av.create_offer_and_gather().await.unwrap());
    let peer = start(&mut f, &slots, &physical).await;
    assert_eq!(
        peer.answer_sdp(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap_err()
            .code,
        "incompatible-capabilities"
    );
    peer.shutdown().await.unwrap();
    available(&slots).await;
    av.close().await.unwrap();
    for too_far in [false, true] {
        let caller = offerer().await;
        let mut f = Fixture::new(caller.create_offer_and_gather().await.unwrap());
        let deadline = if too_far {
            Instant::now() + Duration::from_secs(31)
        } else {
            Instant::now() - Duration::from_millis(1)
        };
        let peer = Peer::start(
            f.store.clone(),
            f.origin.clone(),
            f.attempt.take().unwrap(),
            WebRtcConfig::loopback(),
            &physical,
            slots.clone().acquire_owned().await.unwrap(),
            deadline,
        );
        assert_eq!(
            peer.answer_sdp(Instant::now() + Duration::from_secs(5))
                .await
                .unwrap_err()
                .code,
            "invalid-request"
        );
        assert_eq!(peer.original_attempt().operation_id(), "original-offer");
        peer.shutdown().await.unwrap();
        available(&slots).await;
        caller.close().await.unwrap();
    }
}
