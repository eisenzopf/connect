//! G6-051 actual Core QueuedMedia/VideoRepair to owned RTC/SRTP/SRTCP probe.
//! Both Core controllers and delivery/repair admission are production code.
//! Relationship/policy and synthetic RTP capture use isolated fixtures; port
//! cleanup is a metadata-only fixture while the actual RTC pair is joined.
//! Packets are VP8-declared RTP, not independently decoded pictures. This does
//! not qualify Store/canonical video readiness, browser fallback or Gate closure.
use rtc::{
    media_stream::MediaStreamTrack,
    peer_connection::configuration::{
        interceptor_registry::register_owned_video_interceptors, media_engine::MediaEngine,
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
        Arc, Mutex,
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
    enabled: AtomicBool,
    alive: AtomicBool,
    entered: Notify,
    release: Notify,
    sends: AtomicUsize,
    readers: AtomicUsize,
    readers_at_drop: AtomicUsize,
    discard: Mutex<Option<SocketAddr>>,
    lost: AtomicBool,
    wire: Mutex<Vec<(u8, u16, u32)>>,
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
            if b.len() >= 12 && b[0] >> 6 == 2 && (b[1] & 127 == 96 || b[1] & 127 == 97) {
                if self.gate.enabled.load(Ordering::SeqCst) {
                    let _flight = Flight(self.gate.clone());
                    self.gate.alive.store(true, Ordering::SeqCst);
                    self.gate.entered.notify_one();
                    self.gate.release.notified().await;
                }
                let pt = b[1] & 127;
                let seq = u16::from_be_bytes([b[2], b[3]]);
                let ssrc = u32::from_be_bytes(b[8..12].try_into().unwrap());
                let lost = pt == 96 && seq == 11 && !self.gate.lost.swap(true, Ordering::SeqCst);
                let destination = if lost {
                    self.gate.discard.lock().unwrap().unwrap()
                } else {
                    target
                };
                let n = self.inner.send_to(b, destination).await?;
                self.gate.wire.lock().unwrap().push((pt, seq, ssrc));
                self.gate.sends.fetch_add(1, Ordering::SeqCst);
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
        mime_type: "video/VP8".into(),
        clock_rate: 90000,
        ..Default::default()
    }
}
fn media(rtx: bool) -> MediaEngine {
    let mut m = MediaEngine::default();
    m.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: codec(),
            payload_type: 96,
        },
        RtpCodecKind::Video,
    )
    .unwrap();
    if rtx {
        m.register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: "video/rtx".into(),
                    clock_rate: 90000,
                    sdp_fmtp_line: "apt=96".into(),
                    ..Default::default()
                },
                payload_type: 97,
            },
            RtpCodecKind::Video,
        )
        .unwrap();
    }
    m
}
fn track() -> Arc<TrackLocalStaticRTP> {
    Arc::new(TrackLocalStaticRTP::new(MediaStreamTrack::new(
        "stream".into(),
        "video".into(),
        "guarded".into(),
        RtpCodecKind::Video,
        vec![RTCRtpEncodingParameters {
            rtp_coding_parameters: RTCRtpCodingParameters {
                ssrc: Some(1111),
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
            payload_type: 96,
            ssrc: 1111,
            sequence_number: seq,
            timestamp: u32::from(seq) * 3000,
            marker: true,
            ..Default::default()
        },
        payload: vec![0x10, seq as u8, 42].into(),
    }
}
struct Reader(Arc<Gate>);
impl Drop for Reader {
    fn drop(&mut self) {
        self.0.readers.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Producer {
    source: Arc<RwLock<bool>>,
    target: Arc<RwLock<bool>>,
    gate: Arc<Gate>,
    packet: rtp::Packet,
    expires: Instant,
    cache: bool,
    original_ok: AtomicBool,
    repairs: AtomicUsize,
    entered: Notify,
    substitute: AtomicBool,
}
impl Producer {
    async fn emit(&self, io: &mut dyn OwnedRtpIo, repair: bool) -> OwnedRtpResult {
        let _signal = if repair {
            self.repairs.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            true
        } else {
            false
        };
        let source = self.source.read().await;
        let target = self.target.read().await;
        if !*source || !*target {
            return Err(OwnedRtpError::Refused);
        }
        self.gate.readers.fetch_add(2, Ordering::SeqCst);
        let _a = Reader(self.gate.clone());
        let _b = Reader(self.gate.clone());
        let mut p = self.packet.clone();
        if repair && self.substitute.load(Ordering::SeqCst) {
            p.payload = vec![0x10, 99, 99].into();
        }
        io.write(p).await.map_err(|e| {
            eprintln!("guarded IO refused: {e:?}");
            OwnedRtpError::Refused
        })
    }
}
#[async_trait::async_trait]
impl OwnedRtpProducer for Producer {
    async fn write(&self, io: &mut dyn OwnedRtpIo) -> OwnedRtpResult {
        let result = self.emit(io, false).await;
        if result.is_ok() {
            self.original_ok.store(true, Ordering::SeqCst);
        }
        result
    }
    fn repair_deadline(&self) -> Option<Instant> {
        (self.cache && self.original_ok.load(Ordering::SeqCst)).then_some(self.expires)
    }
    async fn repair(&self, io: &mut dyn OwnedRtpIo) -> OwnedRtpResult {
        self.emit(io, true).await
    }
}
async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), f)
        .await
        .expect("bounded actual RTC event")
}
async fn next(remote: &Arc<dyn TrackRemote>) -> rtp::Packet {
    bounded(async {
        loop {
            match remote.poll().await {
                Some(TrackRemoteEvent::OnRtpPacket(p, _)) => break p,
                None => panic!("remote ended"),
                _ => {}
            }
        }
    })
    .await
}
async fn nack(remote: &Arc<dyn TrackRemote>, ssrc: u32, sequence: u16) {
    use rtc::rtcp::transport_feedbacks::transport_layer_nack::{NackPair, TransportLayerNack};
    remote
        .write_rtcp(vec![Box::new(TransportLayerNack {
            sender_ssrc: 2222,
            media_ssrc: ssrc,
            nacks: vec![NackPair {
                packet_id: sequence,
                lost_packets: 0,
            }],
        })])
        .await
        .unwrap();
}
struct Peers<A: PeerConnection, B: PeerConnection> {
    a: A,
    b: B,
    local: Arc<TrackLocalStaticRTP>,
    remote: Arc<dyn TrackRemote>,
    gate: Arc<Gate>,
    source: Arc<RwLock<bool>>,
    target: Arc<RwLock<bool>>,
    repair_ssrc: Option<u32>,
    clock: rtc::interceptor::SenderClockHandle,
    _discard: std::net::UdpSocket,
}
impl<A: PeerConnection, B: PeerConnection> Peers<A, B> {
    fn producer(&self, seq: u16, cache: bool) -> Arc<Producer> {
        Arc::new(Producer {
            source: self.source.clone(),
            target: self.target.clone(),
            gate: self.gate.clone(),
            packet: packet(seq),
            expires: Instant::now() + Duration::from_millis(200),
            cache,
            original_ok: AtomicBool::new(false),
            repairs: AtomicUsize::new(0),
            entered: Notify::new(),
            substitute: AtomicBool::new(false),
        })
    }
    async fn send(&self, p: Arc<Producer>) {
        let o = self.local.owned_rtp(p.clone(), p.expires);
        assert_eq!(bounded(o.result()).await, Ok(()));
        o.cancel_and_drain().await.unwrap();
    }
    async fn close(self) {
        bounded(self.a.close()).await.unwrap();
        bounded(self.b.close()).await.unwrap();
        assert_eq!(self.gate.readers.load(Ordering::SeqCst), 0);
    }
    fn wire(&self) -> Vec<(u8, u16, u32)> {
        self.gate.wire.lock().unwrap().clone()
    }
}
async fn peers(
    offer_rtx: bool,
    accept_rtx: bool,
) -> Peers<impl PeerConnection, impl PeerConnection> {
    let gate = Arc::new(Gate::default());
    let discard = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    *gate.discard.lock().unwrap() = Some(discard.local_addr().unwrap());
    let (ag, mut agr) = mpsc::channel(1);
    let (bg, mut bgr) = mpsc::channel(1);
    let (ac, mut acr) = mpsc::channel(1);
    let (bc, mut bcr) = mpsc::channel(1);
    let (ao, _) = mpsc::channel(1);
    let (bo, mut bor) = mpsc::channel(1);
    let mut am = media(offer_rtx);
    let clock = rtc::interceptor::SenderClockHandle::default();
    let ar = register_owned_video_interceptors(&mut am, clock.clone()).unwrap();
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
    let mut bm = media(accept_rtx);
    let br =
        register_owned_video_interceptors(&mut bm, rtc::interceptor::SenderClockHandle::default())
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
    let local = track();
    a.add_track(local.clone()).await.unwrap();
    a.set_local_description(a.create_offer(None).await.unwrap())
        .await
        .unwrap();
    eprintln!("wait source gather");
    bounded(agr.recv()).await.unwrap();
    let offer = a.local_description().await.unwrap();
    let repair_ssrc = offer.sdp.lines().find_map(|l| {
        l.strip_prefix("a=ssrc-group:FID 1111 ")
            .and_then(|v| v.trim().parse().ok())
    });
    assert_eq!(repair_ssrc.is_some(), offer_rtx);
    assert!(offer.sdp.contains("a=rtcp-fb:96 nack"));
    b.set_remote_description(offer).await.unwrap();
    b.set_local_description(b.create_answer(None).await.unwrap())
        .await
        .unwrap();
    eprintln!("wait target gather");
    bounded(bgr.recv()).await.unwrap();
    let answer = b.local_description().await.unwrap();
    assert_eq!(
        answer.sdp.contains("a=fmtp:97 apt=96"),
        offer_rtx && accept_rtx
    );
    a.set_remote_description(answer).await.unwrap();
    eprintln!("wait source connected");
    bounded(acr.recv()).await.unwrap();
    eprintln!("wait target connected");
    bounded(bcr.recv()).await.unwrap();
    let source = Arc::new(RwLock::new(true));
    let target = Arc::new(RwLock::new(true));
    let first = Arc::new(Producer {
        source: source.clone(),
        target: target.clone(),
        gate: gate.clone(),
        packet: packet(10),
        expires: Instant::now() + Duration::from_millis(200),
        cache: false,
        original_ok: AtomicBool::new(false),
        repairs: AtomicUsize::new(0),
        entered: Notify::new(),
        substitute: AtomicBool::new(false),
    });
    let o = local.owned_rtp(first, Instant::now() + Duration::from_millis(200));
    assert_eq!(bounded(o.result()).await, Ok(()));
    o.cancel_and_drain().await.unwrap();
    eprintln!("wait opened");
    let remote = bounded(bor.recv()).await.unwrap();
    eprintln!("wait initial RTP");
    assert_eq!(next(&remote).await, packet(10));
    Peers {
        a,
        b,
        local,
        remote,
        gate,
        source,
        target,
        repair_ssrc,
        clock,
        _discard: discard,
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
        pub source_clock: Option<rvoip_core::stream::MediaSourceClock>,
        pub policy: Arc<Policy>,
        pub source: Arc<StreamController>,
        pub target: Arc<StreamController>,
        pub source_port: AdapterMediaPort,
        pub target_port: AdapterMediaPort,
        adapter: Arc<Adapter>,
        target_adapter: Arc<Adapter>,
        input: tokio::sync::mpsc::Receiver<MediaPacket>,
    }
    fn deadline() -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(2)
    }
    impl Fixture {
        pub async fn new(gate: Arc<Gate>) -> Self {
            Self::with_pt(gate, 95).await
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
                            logical_id: StreamId::from_string("video"),
                            medium: Medium::Video,
                            direction,
                        }],
                        &BTreeSet::new(),
                        deadline(),
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap();
            }
            target
                .change_direct_subscriptions(
                    target.revision(),
                    direct_subscriptions::DirectSubscriptionChange::Subscribe(vec![
                        direct_subscriptions::DirectSubscription::new(
                            &source,
                            StreamId::from_string("video"),
                            StreamId::from_string("video"),
                        ),
                    ]),
                    deadline(),
                    CancellationToken::new(),
                    None,
                )
                .await
                .unwrap();
            let source_port = a.port.lock().unwrap().clone().unwrap();
            let target_port = b.port.lock().unwrap().clone().unwrap();
            let input = source
                .take_ingress(&StreamId::from_string("video"))
                .unwrap();
            Self {
                pt,
                source_clock: None,
                policy,
                source,
                target,
                source_port,
                target_port,
                adapter: a,
                target_adapter: b,
                input,
            }
        }
        pub async fn enqueue(&mut self, sequence: u16, ssrc: u32) {
            use rtc::shared::marshal::Marshal;
            let mut p = packet(sequence);
            p.header.payload_type = self.pt;
            p.header.ssrc = ssrc;
            self.source_port
                .try_publish(&MediaFrame {
                    stream_id: self.source_port.binding().transport_id.clone(),
                    kind: StreamKind::Video,
                    payload: p.marshal().unwrap().into(),
                    timestamp_rtp: p.header.timestamp,
                    source_clock: self.source_clock.clone(),
                    captured_at: chrono::Utc::now(),
                    payload_type: Some(self.pt),
                })
                .unwrap();
            bounded(self.input.recv())
                .await
                .unwrap()
                .try_forward_to(&self.target, &StreamId::from_string("video"))
                .unwrap();
        }
        pub fn audience(&self, private: bool) {
            self.policy.state.try_write().unwrap().private = private;
        }
        pub async fn replace_target(&mut self) {
            self.target
                .change(
                    self.target.revision(),
                    &[DesiredStream {
                        logical_id: StreamId::from_string("video"),
                        medium: Medium::Video,
                        direction: StreamDirection::Send,
                    }],
                    &[StreamId::from_string("video")].into_iter().collect(),
                    deadline(),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            self.target_port = self.target_adapter.port.lock().unwrap().clone().unwrap();
        }
        pub async fn clear_selection(&self) {
            self.target
                .change_direct_subscriptions(
                    self.target.revision(),
                    direct_subscriptions::DirectSubscriptionChange::Clear,
                    deadline(),
                    CancellationToken::new(),
                    None,
                )
                .await
                .unwrap();
        }
        pub async fn reselect(&self) {
            self.target
                .change_direct_subscriptions(
                    self.target.revision(),
                    direct_subscriptions::DirectSubscriptionChange::Subscribe(vec![
                        direct_subscriptions::DirectSubscription::new(
                            &self.source,
                            StreamId::from_string("video"),
                            StreamId::from_string("video"),
                        ),
                    ]),
                    deadline(),
                    CancellationToken::new(),
                    None,
                )
                .await
                .unwrap();
        }
        pub async fn gate_source(&self, pause: bool) {
            self.source
                .set_media_gate(
                    self.source.revision(),
                    &[StreamId::from_string("video")].into_iter().collect(),
                    if pause {
                        StreamGateAction::Pause
                    } else {
                        StreamGateAction::Resume
                    },
                    deadline(),
                )
                .await
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
            let id = StreamId::from_string("video");
            self.source
                .change(
                    self.source.revision(),
                    &[DesiredStream {
                        logical_id: id.clone(),
                        medium: Medium::Video,
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

use rvoip_webrtc::media::core_video::CoreVideoEgress;
async fn bind<A: PeerConnection, B: PeerConnection>(
    p: &Peers<A, B>,
    f: &core_fixture::Fixture,
) -> CoreVideoEgress {
    CoreVideoEgress::bind(
        &f.target_port,
        p.local.clone(),
        1111,
        96,
        p.clock.clone(),
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap()
}
async fn send(egress: &mut CoreVideoEgress) -> OwnedRtpWrite {
    let o = bounded(egress.submit_next(Instant::now() + Duration::from_millis(200)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bounded(o.result()).await, Ok(()));
    o.cancel_and_drain().await.unwrap();
    o
}
async fn original(offer: bool, answer: bool) {
    let p = peers(offer, answer).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    f.assert_writable();
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    let w = p.wire();
    assert_eq!(w.len(), 3);
    if offer && answer {
        assert_eq!((w[2].0, w[2].2), (97, p.repair_ssrc.unwrap()));
    } else {
        assert_eq!(w[2], (96, 11, 1111));
    }
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn actual_core_original_and_rtx_repair() {
    original(true, true).await;
}
#[tokio::test]
async fn actual_core_original_and_primary_repair() {
    original(false, false).await;
}
#[tokio::test]
async fn actual_core_declined_rtx_repair() {
    original(true, false).await;
}
async fn revoke(name: &str) {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    f.revoke(name, true);
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(p.wire().len(), 2);
    f.assert_writable();
    f.revoke(name, false);
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    assert_eq!(p.wire().len(), 3);
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn source_revocation_is_rechecked_by_core_repair() {
    revoke("alice").await;
}
#[tokio::test]
async fn target_revocation_is_rechecked_by_core_repair() {
    revoke("bob").await;
}
#[tokio::test]
async fn audience_revocation_is_rechecked_and_restores_only_on_new_nack() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    f.audience(true);
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    assert_eq!(p.wire().len(), 2);
    f.audience(false);
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn clear_and_reselect_never_restore_an_old_repair() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    f.clear_selection().await;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 2);
    f.reselect().await;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 2);
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn pause_resume_never_restore_an_old_repair() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    f.gate_source(true).await;
    f.gate_source(false).await;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 2);
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn core_pending_repair_drops_actual_io_before_authority() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    p.gate.enabled.store(true, Ordering::SeqCst);
    nack(&p.remote, 1111, 11).await;
    bounded(p.gate.entered.notified()).await;
    f.assert_held();
    assert_eq!(p.gate.readers.load(Ordering::SeqCst), 2);
    bounded(async {
        while p.gate.alive.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert_eq!(p.gate.readers_at_drop.load(Ordering::SeqCst), 2);
    f.assert_writable();
    p.gate.enabled.store(false, Ordering::SeqCst);
    p.gate.release.notify_one();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 2);
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn original_cancellation_has_no_retainer_or_late_send() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    p.gate.enabled.store(true, Ordering::SeqCst);
    let o = e
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    bounded(p.gate.entered.notified()).await;
    assert!(o.cancel());
    o.cancel_and_drain().await.unwrap();
    assert_eq!(o.result().await, Err(OwnedRtpError::OutcomeUnknown));
    assert_eq!(p.gate.readers_at_drop.load(Ordering::SeqCst), 2);
    f.assert_writable();
    p.gate.enabled.store(false, Ordering::SeqCst);
    p.gate.release.notify_one();
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 1);
    f.enqueue(12, 3333).await;
    drop(send(&mut e).await);
    assert_eq!(next(&p.remote).await, packet(12));
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn physical_track_timeline_survives_expired_producers_egress_rebind_and_source_replacement() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    drop(e);
    tokio::time::sleep(Duration::from_millis(220)).await;
    f.replace_source().await;
    f.replace_target().await;
    f.reselect().await;
    let mut e = bind(&p, &f).await;
    f.enqueue(5, 4444).await;
    drop(send(&mut e).await);
    let received = next(&p.remote).await;
    let mut expected = packet(5);
    expected.header.sequence_number = 12;
    expected.header.timestamp = 33001;
    assert_eq!(received, expected);
    nack(&p.remote, 1111, 12).await;
    assert_eq!(next(&p.remote).await, expected);
    drop(e);
    f.shutdown().await;
    p.close().await;
}
fn source_clock(live: Arc<AtomicBool>) -> rvoip_core::stream::MediaSourceClock {
    rvoip_core::stream::MediaSourceClock {
        source_id: 7,
        generation: 1,
        anchor: Some(rvoip_core::stream::MediaClockAnchor {
            ntp_time: 1 << 32,
            rtp_time: 33000,
            clock_rate: 90000,
        }),
        supported: true,
        rtp_offset: 0,
        is_current: Arc::new(move || live.load(Ordering::SeqCst)),
    }
}
#[tokio::test]
async fn retired_source_clock_refuses_original_before_udp_and_recovers_same_peer() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    let live = Arc::new(AtomicBool::new(false));
    f.source_clock = Some(source_clock(live.clone()));
    f.enqueue(11, 3333).await;
    let o = e
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(o.result().await, Err(OwnedRtpError::Refused));
    o.cancel_and_drain().await.unwrap();
    assert_eq!(p.wire().len(), 1);
    nack(&p.remote, 1111, 11).await;
    live.store(true, Ordering::SeqCst);
    f.enqueue(12, 3333).await;
    drop(send(&mut e).await);
    assert_eq!(next(&p.remote).await, packet(12));
    assert_eq!(p.wire().len(), 2);
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn retained_repair_rechecks_source_clock_lifetime() {
    let p = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = bind(&p, &f).await;
    let live = Arc::new(AtomicBool::new(true));
    f.source_clock = Some(source_clock(live.clone()));
    f.enqueue(11, 3333).await;
    drop(send(&mut e).await);
    live.store(false, Ordering::SeqCst);
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 2);
    f.assert_writable();
    drop(e);
    f.shutdown().await;
    p.close().await;
}
#[tokio::test]
async fn another_connected_peers_clock_cannot_authorize_or_retime_this_sender() {
    let p = peers(true, true).await;
    let foreign = peers(true, true).await;
    let mut f = core_fixture::Fixture::new(p.gate.clone()).await;
    let mut e = CoreVideoEgress::bind(
        &f.target_port,
        p.local.clone(),
        1111,
        96,
        foreign.clock.clone(),
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    f.enqueue(11, 3333).await;
    let o = e
        .submit_next(Instant::now() + Duration::from_millis(200))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(o.result().await, Err(OwnedRtpError::Refused));
    o.cancel_and_drain().await.unwrap();
    assert_eq!(p.wire().len(), 1);
    assert_eq!(foreign.wire().len(), 1);
    f.assert_writable();
    drop(e);
    f.shutdown().await;
    p.close().await;
    foreign.close().await;
}
