//! G6-051 actual Core QueuedMedia -> production CoreAudioEgress -> RTC/SRTP probe.
//! Core relationships/policy are isolated fixtures; both controllers and queue
//! admission/delivery are the ordinary production Core implementation. Synthetic
//! capture bytes and metadata-only cleanup are explicit fixture adapters. The
//! RTC pair is independently owned and drained here. This does not qualify the
//! canonical Store adapter, port cleanup, browser fallback or Gate acceptance.
use rtc::{
    media_stream::MediaStreamTrack,
    peer_connection::configuration::{
        interceptor_registry::register_owned_dtmf_interceptors, media_engine::MediaEngine,
    },
    rtp,
    rtp_transceiver::rtp_sender::{
        RTCRtpCodec, RTCRtpCodecParameters, RTCRtpCodingParameters, RTCRtpEncodingParameters,
        RtpCodecKind,
    },
};
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, RwLock, mpsc};
use webrtc::{
    media_stream::{
        track_local::{owned_rtp::*, static_rtp::TrackLocalStaticRTP},
        track_remote::{TrackRemote, TrackRemoteEvent},
    },
    peer_connection::{
        PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
        RTCPeerConnectionState,
    },
    runtime::{AsyncUdpSocket, JoinHandle, Runtime, TokioRuntime},
};
#[derive(Debug, Default)]
struct Gate {
    sequences: std::sync::Mutex<Vec<u16>>,
    enabled: AtomicBool,
    alive: AtomicBool,
    entered: Notify,
    release: Notify,
    sends: AtomicUsize,
    readers: AtomicUsize,
    readers_at_drop: AtomicUsize,
}
struct Flight(Arc<Gate>);
impl Drop for Flight {
    fn drop(&mut self) {
        self.0
            .readers_at_drop
            .store(self.0.readers.load(Ordering::SeqCst), Ordering::SeqCst);
        self.0.alive.store(false, Ordering::SeqCst);
    }
}
#[derive(Debug)]
struct Socket {
    inner: Arc<dyn AsyncUdpSocket>,
    gate: Arc<Gate>,
}
impl AsyncUdpSocket for Socket {
    fn send_to<'a>(
        &'a self,
        b: &'a [u8],
        target: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = io::Result<usize>> + Send + 'a>> {
        Box::pin(async move {
            if b.len() >= 12 && b[0] >> 6 == 2 && b[1] & 127 == 0 {
                if self.gate.enabled.load(Ordering::SeqCst) {
                    let _flight = Flight(self.gate.clone());
                    self.gate.alive.store(true, Ordering::SeqCst);
                    self.gate.entered.notify_one();
                    self.gate.release.notified().await;
                }
                let n = self.inner.send_to(b, target).await?;
                self.gate.sends.fetch_add(1, Ordering::SeqCst);
                self.gate
                    .sequences
                    .lock()
                    .unwrap()
                    .push(u16::from_be_bytes([b[2], b[3]]));
                Ok(n)
            } else {
                self.inner.send_to(b, target).await
            }
        })
    }
    fn recv_from<'a>(
        &'a self,
        b: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<(usize, SocketAddr)>> + Send + 'a>> {
        self.inner.recv_from(b)
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}
#[derive(Debug)]
struct HarnessRuntime(Arc<Gate>);
impl Runtime for HarnessRuntime {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send>>) -> JoinHandle {
        TokioRuntime.spawn(f)
    }
    fn wrap_udp_socket(&self, s: std::net::UdpSocket) -> io::Result<Arc<dyn AsyncUdpSocket>> {
        Ok(Arc::new(Socket {
            inner: TokioRuntime.wrap_udp_socket(s)?,
            gate: self.0.clone(),
        }))
    }
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
fn codec() -> RTCRtpCodec {
    RTCRtpCodec {
        mime_type: "audio/PCMU".into(),
        clock_rate: 8000,
        channels: 1,
        ..Default::default()
    }
}
fn media() -> MediaEngine {
    let mut m = MediaEngine::default();
    m.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: codec(),
            payload_type: 0,
        },
        RtpCodecKind::Audio,
    )
    .unwrap();
    m.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: "audio/PCMA".into(),
                ..codec()
            },
            payload_type: 8,
        },
        RtpCodecKind::Audio,
    )
    .unwrap();
    m
}
fn track(ssrc: u32) -> Arc<TrackLocalStaticRTP> {
    Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        "stream".into(),
        format!("track{ssrc}"),
        "owned".into(),
        RtpCodecKind::Audio,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(ssrc),
                ..Default::default()
            },
            codec: codec(),
            ..Default::default()
        }],
    )))
}
fn packet(seq: u16) -> rtp::Packet {
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            payload_type: 0,
            ssrc: 1111,
            sequence_number: seq,
            timestamp: u32::from(seq) * 160,
            ..Default::default()
        },
        payload: vec![0xff; 160].into(),
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
    fn audio() -> MediaFormat {
        MediaFormat::Encoded {
            name: "pcmu".into(),
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
    fn profile() -> EndpointMediaCapabilities {
        let capabilities = vec![
            MediaCapability {
                medium: Medium::Audio,
                formats: vec![audio()],
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
    fn owner(name: &str) -> MediaOwner {
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
            local: profile(),
            remote: profile(),
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
        adapter: Arc<Adapter>,
        input: tokio::sync::mpsc::Receiver<MediaPacket>,
    }
    fn deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(2)
    }
    impl Fixture {
        pub async fn new(gate: Arc<Gate>) -> Self {
            Self::with_pt(gate, 0).await
        }
        pub async fn with_pt(gate: Arc<Gate>, pt: u8) -> Self {
            let policy = Arc::new(Policy {
                state: Arc::new(RwLock::new(PolicyState {
                    owners: [
                        ("alice".into(), owner("alice")),
                        ("bob".into(), owner("bob")),
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
                adapter: a,
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
        pub fn revoke(&self, name: &str, yes: bool) {
            let mut p = self
                .policy
                .state
                .try_write()
                .expect("Core metadata queue must not retain permission");
            if yes {
                p.revoked.insert(name.into());
            } else {
                p.revoked.remove(name);
            }
        }
        pub fn assert_writable(&self) {
            drop(
                self.policy
                    .state
                    .try_write()
                    .expect("all Core readers must be released"),
            );
        }
        pub fn assert_held(&self) {
            assert!(self.policy.state.try_write().is_err());
        }
        pub async fn replace_source(&mut self) {
            let id = StreamId::from_string("audio");
            self.source
                .change(
                    self.source.revision(),
                    &[DesiredStream {
                        logical_id: id.clone(),
                        medium: Medium::Audio,
                        direction: StreamDirection::Receive,
                    }],
                    &[id.clone()].into_iter().collect(),
                    deadline(),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            let next = self.adapter.port.lock().unwrap().clone().unwrap();
            assert_ne!(
                self.source_port.binding().transport_id,
                next.binding().transport_id
            );
            self.source_port = next;
            self.input = self.source.take_ingress(&id).unwrap();
        }
        pub async fn shutdown(&self) {
            assert_eq!(self.source.shutdown(deadline()).await, 0);
            assert_eq!(self.target.shutdown(deadline()).await, 0);
        }
    }
}

async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), f)
        .await
        .expect("bounded actual RTC event")
}
async fn exercise() {
    let gate = Arc::new(Gate::default());
    let (ag, mut agr) = mpsc::channel(1);
    let (bg, mut bgr) = mpsc::channel(1);
    let (ac, mut acr) = mpsc::channel(1);
    let (bc, mut bcr) = mpsc::channel(1);
    let (ao, _aor) = mpsc::channel(1);
    let (bo, mut bor) = mpsc::channel(1);
    let mut am = media();
    let ar =
        register_owned_dtmf_interceptors(&mut am, rtc::interceptor::SenderClockHandle::default())
            .unwrap();
    let a = PeerConnectionBuilder::new()
        .with_media_engine(am)
        .with_interceptor_registry(ar)
        .with_runtime(Arc::new(HarnessRuntime(gate.clone())))
        .with_handler(Arc::new(Handler {
            gather: ag,
            connected: ac,
            opened: ao,
        }))
        .with_udp_addrs(vec!["127.0.0.1:0"])
        .build()
        .await
        .unwrap();
    let mut bm = media();
    let br =
        register_owned_dtmf_interceptors(&mut bm, rtc::interceptor::SenderClockHandle::default())
            .unwrap();
    let b = PeerConnectionBuilder::new()
        .with_media_engine(bm)
        .with_interceptor_registry(br)
        .with_handler(Arc::new(Handler {
            gather: bg,
            connected: bc,
            opened: bo,
        }))
        .with_udp_addrs(vec!["127.0.0.1:0"])
        .build()
        .await
        .unwrap();
    let t = track(1111);
    a.add_track(t.clone()).await.unwrap();
    b.add_track(track(2222)).await.unwrap();
    a.set_local_description(a.create_offer(None).await.unwrap())
        .await
        .unwrap();
    bounded(agr.recv()).await.unwrap();
    b.set_remote_description(a.local_description().await.unwrap())
        .await
        .unwrap();
    b.set_local_description(b.create_answer(None).await.unwrap())
        .await
        .unwrap();
    bounded(bgr.recv()).await.unwrap();
    a.set_remote_description(b.local_description().await.unwrap())
        .await
        .unwrap();
    bounded(acr.recv()).await.unwrap();
    bounded(bcr.recv()).await.unwrap();

    {
        let mut wrong = core_fixture::Fixture::with_pt(gate.clone(), 8).await;
        let mut invalid = rvoip_webrtc::media::core_audio::CoreAudioEgress::bind(
            &wrong.target_port,
            t.clone(),
            1111,
            8,
            Instant::now() + Duration::from_secs(1),
        )
        .await
        .unwrap();
        wrong.enqueue(160).await;
        let original = invalid
            .submit_next(Instant::now() + Duration::from_millis(200))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.result().await, Err(OwnedRtpError::Refused));
        original.cancel_and_drain().await.unwrap();
        assert_eq!(gate.sends.load(Ordering::SeqCst), 0);
        wrong.shutdown().await;
    }
    println!(
        "PASS a negotiated sibling codec/PT cannot silently retarget Core audio onto a different encoding"
    );
    let mut f = core_fixture::Fixture::new(gate.clone()).await;
    let mut egress = rvoip_webrtc::media::core_audio::CoreAudioEgress::bind(
        &f.target_port,
        t.clone(),
        1111,
        0,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    f.enqueue(160).await;
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    f.assert_writable();
    let owner = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(owner.result().await, Ok(()));
    owner.cancel_and_drain().await.unwrap();
    let remote = bounded(bor.recv()).await.unwrap();
    let received = bounded(async {
        loop {
            if let Some(TrackRemoteEvent::OnRtpPacket(p, _)) = remote.poll().await {
                break p;
            }
        }
    })
    .await;
    assert_eq!(received, packet(1));
    assert_eq!(gate.sends.load(Ordering::SeqCst), 1);
    println!(
        "PASS exact production Core port queue delivers expected packet through actual ICE/DTLS/SRTP"
    );
    for name in ["alice", "bob"] {
        f.enqueue(320).await;
        f.revoke(name, true);
        let o = egress
            .submit_next(Instant::now() + Duration::from_millis(200))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(o.result().await, Err(OwnedRtpError::Refused));
        o.cancel_and_drain().await.unwrap();
        assert_eq!(gate.sends.load(Ordering::SeqCst), 1);
        assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
        f.revoke(name, false);
    }
    println!(
        "PASS Core source and target revocation after enqueue refuse before UDP with no queued reader"
    );
    for stage in [1, 2] {
        f.enqueue(320 + stage as u32 * 160).await;
        f.policy.busy.store(stage, Ordering::SeqCst);
        let o = egress
            .submit_next(Instant::now() + Duration::from_millis(200))
            .await
            .unwrap()
            .unwrap();
        bounded(f.policy.entered.notified()).await;
        assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
        f.assert_writable();
        f.policy.busy.store(0, Ordering::SeqCst);
        assert_eq!(o.result().await, Ok(()));
        o.cancel_and_drain().await.unwrap();
    }
    assert_eq!(gate.sends.load(Ordering::SeqCst), 3);
    println!(
        "PASS production Core source/target metadata retries preserve original packet and release all readers between attempts"
    );
    f.enqueue(800).await;
    gate.enabled.store(true, Ordering::SeqCst);
    let owner = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    bounded(gate.entered.notified()).await;
    assert_eq!(gate.readers.load(Ordering::SeqCst), 2);
    f.assert_held();
    drop(egress);
    let next_fixture = core_fixture::Fixture::new(gate.clone()).await;
    let next_egress = rvoip_webrtc::media::core_audio::CoreAudioEgress::bind(
        &next_fixture.target_port,
        t.clone(),
        1111,
        0,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    let mut wait = Box::pin(owner.result());
    std::future::poll_fn(|cx| {
        assert!(wait.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(wait);
    assert!(gate.alive.load(Ordering::SeqCst));
    assert_eq!(gate.readers.load(Ordering::SeqCst), 2);
    assert!(owner.cancel());
    owner.cancel_and_drain().await.unwrap();
    assert_eq!(owner.result().await, Err(OwnedRtpError::OutcomeUnknown));
    assert!(!gate.alive.load(Ordering::SeqCst));
    assert_eq!(gate.readers_at_drop.load(Ordering::SeqCst), 2);
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    f.assert_writable();
    f.shutdown().await;
    f = next_fixture;
    egress = next_egress;
    println!(
        "PASS cancelled result observation retains original IO and both actual Core permission leases until driver destroys UDP future before releasing them"
    );
    f.enqueue(960).await;
    let timed = egress
        .submit_next(Instant::now() + Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    bounded(gate.entered.notified()).await;
    assert_eq!(gate.readers.load(Ordering::SeqCst), 2);
    assert_eq!(
        bounded(timed.result()).await,
        Err(OwnedRtpError::OutcomeUnknown)
    );
    timed.cancel_and_drain().await.unwrap();
    assert!(!gate.alive.load(Ordering::SeqCst));
    assert_eq!(gate.readers_at_drop.load(Ordering::SeqCst), 2);
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    f.assert_writable();
    gate.enabled.store(false, Ordering::SeqCst);
    gate.release.notify_one();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(gate.sends.load(Ordering::SeqCst), 3);
    println!(
        "PASS original frame/write deadline destroys pending UDP before actual Core leases, original drain confirms, no late packet"
    );
    f.enqueue(1120).await;
    let good = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(good.result().await, Ok(()));
    good.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 4);
    println!(
        "PASS same Core queue and RTC peer deliver subsequent authorized frame after cancelled/expired writes"
    );
    f.enqueue(1280).await;
    tokio::time::sleep(Duration::from_millis(210)).await;
    let expired = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.result().await, Err(OwnedRtpError::Refused));
    expired.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 4);
    println!(
        "PASS Core original queued-frame age cannot be refreshed by a new submission deadline"
    );
    f.enqueue(1440).await;
    f.replace_source().await;
    let old = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        old.result().await,
        Err(OwnedRtpError::Refused | OwnedRtpError::DeadlineBeforeWrite)
    ));
    old.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 4);
    println!("PASS replaced original Core source generation fences old queued bytes before UDP");
    f.enqueue(1600).await;
    let replacement = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(replacement.result().await, Ok(()));
    replacement.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 5);
    assert_eq!(
        *gate.sequences.lock().unwrap(),
        vec![1, 2, 3, 6, 7],
        "the original SSRC sequence owner must outlive egress drop while its write is pending"
    );
    println!(
        "PASS dropped egress preserves the original shared SSRC timeline through pending write ownership and rebind"
    );
    println!("PASS replacement Core source delivers on unchanged target and RTC peer");
    let _ = tokio::time::timeout(Duration::from_millis(1), gate.release.notified()).await;
    f.enqueue(1760).await;
    gate.enabled.store(true, Ordering::SeqCst);
    let lost = egress
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    bounded(gate.entered.notified()).await;
    assert_eq!(gate.readers.load(Ordering::SeqCst), 2);
    bounded(a.close()).await.unwrap();
    assert!(!gate.alive.load(Ordering::SeqCst));
    assert_eq!(gate.readers_at_drop.load(Ordering::SeqCst), 2);
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    assert_eq!(lost.result().await, Err(OwnedRtpError::OutcomeUnknown));
    assert_eq!(
        lost.cancel_and_drain().await,
        Err(OwnedRtpError::OutcomeUnknown)
    );
    f.assert_writable();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 5);
    f.shutdown().await;
    println!(
        "PASS missing original driver receipt remains unconfirmed; actual parent RTC close destroys pending IO before actual Core permission release"
    );

    bounded(a.close()).await.unwrap();
    bounded(b.close()).await.unwrap();
}
fn main() {
    webrtc::runtime::block_on(exercise());
}
