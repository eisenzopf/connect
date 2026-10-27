//! G6-051 peer-bound Core G.711 egress probe against ordinary production libraries.
//! Current Core policy, relationships, synthetic capture and metadata-only port
//! readiness/cleanup are isolated fixtures. Actual RvoipPeerConnection, codec
//! construction, stable SDP mapping, Core queue and RTC/SRTP delivery are real.
//! This does not qualify canonical Store admission/readiness or browser media.
use rtc::{
    media_stream::MediaStreamTrack,
    peer_connection::configuration::{
        interceptor_registry::register_owned_dtmf_interceptors, media_engine::MediaEngine,
    },
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RtpCodecKind,
    },
};
use rvoip_webrtc::{
    WebRtcConfig,
    peer::{PeerRole, RvoipPeerConnection},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, RwLock, mpsc};
use webrtc::{
    media_stream::{
        Track,
        track_local::static_rtp::TrackLocalStaticRTP,
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState,
    },
};
#[derive(Default)]
struct Gate {
    readers: AtomicUsize,
}
struct Handler {
    gather: mpsc::Sender<()>,
    connected: mpsc::Sender<()>,
    opened: mpsc::Sender<Arc<dyn TrackRemote>>,
}
#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, s: RTCIceGatheringState) {
        if s == RTCIceGatheringState::Complete {
            let _ = self.gather.try_send(());
        }
    }
    async fn on_connection_state_change(&self, s: RTCPeerConnectionState) {
        if s == RTCPeerConnectionState::Connected {
            let _ = self.connected.try_send(());
        }
    }
    async fn on_track(&self, t: Arc<dyn TrackRemote>) {
        let _ = self.opened.try_send(t);
    }
}
mod core_fixture {
    use super::*;
    use rvoip_core::{
        capability::DataProtocol,
        connection::Transport,
        identity::{AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance},
        ids::*,
        participant::{ParticipantKind, ParticipantRole},
        relationships::*,
        staged_streams::*,
        stream::{MediaFrame, StreamKind},
        stream_lifecycle::*,
    };
    use std::collections::{BTreeSet, HashMap, HashSet};
    use std::sync::Mutex;
    use tokio::sync::OwnedRwLockReadGuard;
    use tokio_util::sync::CancellationToken;
    fn principal(name: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: name.into(),
            tenant: Some("tenant".into()),
            scopes: vec![],
            issuer: Some("fixture".into()),
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Anonymous,
        }
    }
    fn audio(codec: &str) -> MediaFormat {
        MediaFormat::Encoded {
            name: codec.into(),
            clock_rate_hz: 8000,
            channels: 1,
            fmtp: None,
        }
    }
    fn video() -> MediaFormat {
        MediaFormat::Encoded {
            name: "vp8".into(),
            clock_rate_hz: 90000,
            channels: 0,
            fmtp: None,
        }
    }
    fn profile(codec: &str) -> EndpointMediaCapabilities {
        let capabilities = vec![
            MediaCapability {
                medium: Medium::Audio,
                formats: vec![audio(codec)],
            },
            MediaCapability {
                medium: Medium::Video,
                formats: vec![video()],
            },
            MediaCapability {
                medium: Medium::Screen,
                formats: vec![video()],
            },
            MediaCapability {
                medium: Medium::Data,
                formats: vec![MediaFormat::ReliableOrdered(DataProtocol::Json)],
            },
        ];
        EndpointMediaCapabilities {
            send: capabilities.clone(),
            receive: capabilities,
            max_streams: 8,
            renegotiation: true,
        }
    }
    fn owner(name: &str, codec: &str) -> MediaOwner {
        let principal = principal(name);
        let identity = IdentityBinding::from_authenticated(
            &principal,
            IdentityId::from_string(name),
            chrono::Utc::now(),
        )
        .unwrap();
        let member = Membership::admit(
            identity.clone(),
            ConversationId::from_string("conversation"),
            ParticipantId::from_string(format!("participant-{name}")),
            ParticipantKind::Human,
            ParticipantRole::Agent,
        )
        .unwrap();
        let presence =
            SessionAppearance::admit(&member, SessionId::from_string("session"), None).unwrap();
        let mut endpoint = EndpointBinding::provision(
            identity.clone(),
            EndpointId::from_string(format!("endpoint-{name}")),
            DeviceId::from_string(format!("device-{name}")),
            Transport::WebRtc,
            "fixture-address".into(),
        )
        .unwrap();
        endpoint
            .verify(&identity, "fixture-evidence".into(), chrono::Utc::now())
            .unwrap();
        let mut connection = ConnectionBinding::prepare(
            &member,
            &presence,
            &endpoint,
            ConnectionId::from_string(name),
        )
        .unwrap();
        connection
            .observe_ready(&member, &presence, &endpoint, 1)
            .unwrap();
        connection.activate(&member, &presence, &endpoint).unwrap();
        MediaOwner {
            actor: member.clone(),
            member,
            presence,
            endpoint,
            connection,
            may_control: true,
            send_media: [Medium::Audio, Medium::Video, Medium::Screen, Medium::Data]
                .into_iter()
                .collect(),
            receive_media: [Medium::Audio, Medium::Video, Medium::Screen, Medium::Data]
                .into_iter()
                .collect(),
            local: profile(codec),
            remote: profile(codec),
            may_route_source: true,
        }
    }

    struct PolicyState {
        owners: HashMap<String, MediaOwner>,
        revoked: HashSet<String>,
        private: bool,
    }
    pub struct Policy {
        state: Arc<RwLock<PolicyState>>,
        gate: Arc<Gate>,
        pub busy: AtomicUsize,
        pub entered: Notify,
    }
    struct Lease {
        authority: MediaOwner,
        _guard: OwnedRwLockReadGuard<PolicyState>,
        gate: Arc<Gate>,
    }
    impl Drop for Lease {
        fn drop(&mut self) {
            self.gate.readers.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl MediaAuthorityLease for Lease {
        fn authority(&self) -> &MediaOwner {
            &self.authority
        }
    }
    impl MediaPolicy for Policy {
        fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
            let guard = self
                .state
                .clone()
                .try_read_owned()
                .map_err(|_| StreamControlError::Authority)?;
            let busy = self.busy.load(Ordering::SeqCst);
            if (busy == 1 && scope.connection.as_str() == "alice")
                || (busy == 2 && scope.connection.as_str() == "bob")
            {
                self.entered.notify_one();
                return Err(StreamControlError::NotReady);
            }
            if guard.revoked.contains(&scope.principal.subject) {
                return Err(StreamControlError::Authority);
            }
            let mut authority = guard
                .owners
                .get(scope.connection.as_str())
                .ok_or(StreamControlError::Authority)?
                .clone();
            if let MediaPurpose::Receive { source } = scope.purpose {
                let original = guard
                    .owners
                    .get(source.connection.as_str())
                    .ok_or(StreamControlError::Authority)?;
                if source.conversation != *scope.conversation
                    || source.session != *scope.session
                    || source.connection != *original.connection.connection()
                    || source.participant != *original.member.participant()
                    || source.membership_epoch != original.member.epoch()
                    || source.connection_generation != original.connection.generation()
                {
                    return Err(StreamControlError::Authority);
                }
                authority.may_route_source = !guard.private;
            }
            self.gate.readers.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Lease {
                authority,
                _guard: guard,
                gate: self.gate.clone(),
            }))
        }
    }
    struct Cleanup;
    #[async_trait::async_trait]
    impl MediaPortCleanup for Cleanup {
        async fn close(&self) -> ControlResult<()> {
            Ok(())
        }
    }
    struct Adapter {
        pt: u8,
        port: Mutex<Option<AdapterMediaPort>>,
    }
    #[async_trait::async_trait]
    impl StagedMediaAdapter for Adapter {
        async fn prepare(
            &self,
            ports: Vec<PortPreparation>,
            _: tokio::time::Instant,
            _: CancellationToken,
        ) -> ControlResult<()> {
            for p in ports {
                if p.is_new {
                    p.port.install_cleanup(Arc::new(Cleanup))?;
                    p.port.observe_ready(
                        &p.port.binding().negotiated.format,
                        true,
                        true,
                        Some(self.pt),
                    )?;
                }
                *self.port.lock().unwrap() = Some(p.port);
            }
            Ok(())
        }
    }
    pub struct Fixture {
        pt: u8,
        pub policy: Arc<Policy>,
        pub source: Arc<StreamController>,
        pub target: Arc<StreamController>,
        pub source_port: AdapterMediaPort,
        pub target_port: AdapterMediaPort,
        input: tokio::sync::mpsc::Receiver<MediaPacket>,
    }
    fn deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(2)
    }
    impl Fixture {
        pub async fn with_audio(gate: Arc<Gate>, codec: &str, pt: u8) -> Self {
            let policy = Arc::new(Policy {
                state: Arc::new(RwLock::new(PolicyState {
                    owners: [
                        ("alice".into(), owner("alice", codec)),
                        ("bob".into(), owner("bob", codec)),
                    ]
                    .into_iter()
                    .collect(),
                    revoked: HashSet::new(),
                    private: false,
                })),
                gate,
                busy: AtomicUsize::new(0),
                entered: Notify::new(),
            });
            let a = Arc::new(Adapter {
                pt,
                port: Mutex::new(None),
            });
            let b = Arc::new(Adapter {
                pt,
                port: Mutex::new(None),
            });
            let handles = Arc::new(StreamHandleAllocator::default());
            let make = |name: &str, adapter: Arc<Adapter>| {
                StreamController::new(
                    ConversationId::from_string("conversation"),
                    SessionId::from_string("session"),
                    ConnectionId::from_string(name),
                    principal(name),
                    policy.clone(),
                    adapter,
                    handles.clone(),
                    2,
                )
                .unwrap()
            };
            let source = make("alice", a.clone());
            let target = make("bob", b.clone());
            for (controller, direction) in [
                (&source, StreamDirection::Receive),
                (&target, StreamDirection::Send),
            ] {
                controller
                    .change(
                        0,
                        &[DesiredStream {
                            logical_id: StreamId::from_string("audio"),
                            medium: Medium::Audio,
                            direction,
                        }],
                        &BTreeSet::new(),
                        deadline(),
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap();
            }
            let source_port = a.port.lock().unwrap().clone().unwrap();
            let target_port = b.port.lock().unwrap().clone().unwrap();
            let input = source
                .take_ingress(&StreamId::from_string("audio"))
                .unwrap();
            Self {
                pt,
                policy,
                source,
                target,
                source_port,
                target_port,
                input,
            }
        }
        pub async fn enqueue(&mut self, timestamp: u32) {
            self.source_port
                .try_publish(&MediaFrame {
                    stream_id: self.source_port.binding().transport_id.clone(),
                    kind: StreamKind::Audio,
                    payload: vec![0xff; 160].into(),
                    timestamp_rtp: timestamp,
                    source_clock: None,
                    captured_at: chrono::Utc::now(),
                    payload_type: Some(self.pt),
                })
                .unwrap();
            bounded(self.input.recv())
                .await
                .unwrap()
                .try_forward_to(&self.target, &StreamId::from_string("audio"))
                .unwrap();
        }

        pub fn assert_writable(&self) {
            drop(
                self.policy
                    .state
                    .try_write()
                    .expect("all Core readers must be released"),
            );
        }

        pub async fn shutdown(&self) {
            assert_eq!(self.source.shutdown(deadline()).await, 0);
            assert_eq!(self.target.shutdown(deadline()).await, 0);
        }
    }
}

async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), f)
        .await
        .expect("bounded actual peer operation")
}
async fn exercise(name: &str, mime: &str, pt: u8) {
    use rvoip_core::staged_streams::StreamControlError as E;
    let codec = RTCRtpCodec {
        mime_type: mime.into(),
        clock_rate: 8000,
        channels: 1,
        ..Default::default()
    };
    let mut media = MediaEngine::default();
    media
        .register_codec(
            RTCRtpCodecParameters {
                rtp_codec: codec.clone(),
                payload_type: pt,
            },
            RtpCodecKind::Audio,
        )
        .unwrap();
    let registry = register_owned_dtmf_interceptors(
        &mut media,
        rtc::interceptor::SenderClockHandle::default(),
    )
    .unwrap();
    let (ag, mut agr) = mpsc::channel(1);
    let (ac, mut acr) = mpsc::channel(1);
    let (ao, mut aor) = mpsc::channel(1);
    let raw = PeerConnectionBuilder::new()
        .with_media_engine(media)
        .with_interceptor_registry(registry)
        .with_handler(Arc::new(Handler {
            gather: ag,
            connected: ac,
            opened: ao,
        }))
        .with_udp_addrs(vec!["127.0.0.1:0"])
        .build()
        .await
        .unwrap();
    let track = Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        "raw".into(),
        "raw-audio".into(),
        "fixture".into(),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(1111),
                ..Default::default()
            },
            codec,
            ..Default::default()
        }],
    )));
    raw.add_track(track).await.unwrap();
    let mut config = WebRtcConfig::loopback();
    config.capabilities.audio_codecs = vec![rvoip_core::capability::CodecInfo {
        name: name.into(),
        clock_rate_hz: 8000,
        channels: 1,
        fmtp: None,
        payload_type: None,
    }];
    let peer = RvoipPeerConnection::new(&config, PeerRole::Answerer)
        .await
        .unwrap();
    let gate = Arc::new(Gate::default());
    let mut f = core_fixture::Fixture::with_audio(gate.clone(), name, pt).await;
    assert!(matches!(
        peer.core_audio_egress(&f.target_port, Instant::now() + Duration::from_secs(1))
            .await,
        Err(E::NotReady)
    ));
    raw.set_local_description(raw.create_offer(None).await.unwrap())
        .await
        .unwrap();
    bounded(agr.recv()).await.unwrap();
    let offer = raw.local_description().await.unwrap();
    let answer = peer.accept_offer_and_gather(&offer.sdp).await.unwrap();
    raw.set_remote_description(
        webrtc::peer_connection::RTCSessionDescription::answer(answer).unwrap(),
    )
    .await
    .unwrap();
    bounded(acr.recv()).await.unwrap();
    peer.wait_connected(Duration::from_secs(3)).await.unwrap();
    let ssrc = peer.local_audio_ssrc().unwrap();
    let actual = peer.local_audio_track().unwrap().codec(ssrc).await.unwrap();
    assert_eq!(
        (
            actual.mime_type.as_str(),
            actual.clock_rate,
            actual.channels
        ),
        (mime, 8000, 1)
    );
    println!(
        "PASS {name}: configured local track uses exact G.711 codec/8000/1 rather than Opus/48000"
    );
    let wrong_name = if name == "pcmu" { "pcma" } else { "pcmu" };
    let wrong = core_fixture::Fixture::with_audio(gate.clone(), wrong_name, pt).await;
    assert!(matches!(
        peer.core_audio_egress(&wrong.target_port, Instant::now() + Duration::from_secs(1))
            .await,
        Err(E::Unsupported)
    ));
    assert!(wrong.target_port.take_egress().is_ok());
    wrong.shutdown().await;
    let weak = Arc::downgrade(&peer);
    let mut egress = peer
        .core_audio_egress(&f.target_port, Instant::now() + Duration::from_secs(1))
        .await
        .unwrap();
    drop(peer);
    assert!(weak.upgrade().is_some());
    f.enqueue(160).await;
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    let write = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.result().await, Ok(()));
    write.cancel_and_drain().await.unwrap();
    let remote = bounded(aor.recv()).await.unwrap();
    let first = bounded(async {
        loop {
            if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = remote.poll().await {
                break p;
            }
        }
    })
    .await;
    assert_eq!(
        (
            first.header.payload_type,
            first.header.ssrc,
            first.header.sequence_number,
            first.header.timestamp
        ),
        (pt, ssrc, 1, 160)
    );
    assert_eq!(first.payload.as_ref(), &[0xff; 160]);
    f.enqueue(320).await;
    let write = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(write.result().await, Ok(()));
    write.cancel_and_drain().await.unwrap();
    let second = bounded(async {
        loop {
            if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = remote.poll().await {
                break p;
            }
        }
    })
    .await;
    assert_eq!(
        (
            second.header.payload_type,
            second.header.ssrc,
            second.header.sequence_number,
            second.header.timestamp
        ),
        (pt, ssrc, 2, 320)
    );
    assert_eq!(second.payload.as_ref(), &[0xff; 160]);
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    f.assert_writable();
    println!(
        "PASS {name}: actual completed SDP selects PT{pt}; peer-bound Core egress sends exact decrypted RTP payload, SSRC and 160-sample timeline"
    );
    println!(
        "PASS {name}: unconnected/mismatched candidates refuse without consuming Core queue; egress retains actual peer after caller drop"
    );
    f.shutdown().await;
    weak.upgrade().unwrap().close().await.unwrap();
    drop(egress);
    assert!(weak.upgrade().is_none());
    bounded(raw.close()).await.unwrap();
    println!(
        "PASS {name}: original physical RTC closes and retained peer is released after queue-owner drop"
    );
}
fn main() {
    webrtc::runtime::block_on(async {
        exercise("pcmu", "audio/PCMU", 109).await;
        exercise("pcma", "audio/PCMA", 8).await;
    });
}
