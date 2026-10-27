//! G6-051 local production-library guarded VP8 RTP/repair probe.
//! Build the current interceptor, RTC and stack sources with retained Root
//! dependencies; exact invocations and hashes are in webrtc-owned-video.json.
//! Policy is an instrumented two-reader fixture. Packets are synthetic RTP
//! declared as VP8, not decoded pictures. This checks actual ICE/DTLS/SRTP/SRTCP,
//! repair permission/identity/expiry/IO custody, not Core/Store composition,
//! canonical video readiness, installed browsers or full fallback qualification.
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
    let ar =
        register_owned_video_interceptors(&mut am, rtc::interceptor::SenderClockHandle::default())
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
        _discard: discard,
    }
}
async fn positive(offer: bool, accept: bool) {
    let p = peers(offer, accept).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    assert_eq!(p.gate.readers.load(Ordering::SeqCst), 0);
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    assert_eq!(original.repairs.load(Ordering::SeqCst), 1);
    let wire = p.wire();
    assert_eq!(wire.len(), 3);
    let repair = wire[2];
    if offer && accept {
        assert_eq!((repair.0, repair.2), (97, p.repair_ssrc.unwrap()));
        assert_ne!(repair.1, 11);
    } else {
        assert_eq!(repair, (96, 11, 1111));
    }
    p.close().await;
}
#[tokio::test]
async fn encrypted_negotiated_rtx_restores_exact_original() {
    positive(true, true).await;
}
#[tokio::test]
async fn encrypted_primary_repair_without_rtx() {
    positive(false, false).await;
}
#[tokio::test]
async fn encrypted_declined_rtx_uses_actual_primary() {
    positive(true, false).await;
}
async fn revoke(source: bool) {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    let policy = if source { &p.source } else { &p.target };
    *policy.try_write().expect("no idle reader") = false;
    nack(&p.remote, 1111, 11).await;
    bounded(original.entered.notified()).await;
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(p.wire().len(), 2);
    assert_eq!(original.repairs.load(Ordering::SeqCst), 1);
    assert_eq!(p.gate.readers.load(Ordering::SeqCst), 0);
    *policy.try_write().unwrap() = true;
    assert_eq!(p.wire().len(), 2);
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    assert_eq!(original.repairs.load(Ordering::SeqCst), 2);
    assert_eq!(p.wire().len(), 3);
    p.close().await;
}
#[tokio::test]
async fn source_revoke_refuses_native_bypass_then_fresh_restore() {
    revoke(true).await;
}
#[tokio::test]
async fn target_revoke_refuses_native_bypass_then_fresh_restore() {
    revoke(false).await;
}
#[tokio::test]
async fn no_retainer_means_no_native_cached_repair() {
    let p = peers(true, true).await;
    let original = p.producer(11, false);
    p.send(original.clone()).await;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(70)).await;
    assert_eq!(original.repairs.load(Ordering::SeqCst), 0);
    assert_eq!(p.wire().len(), 2);
    p.close().await;
}
#[tokio::test]
async fn idle_expiry_drops_producer_without_nack_and_never_renews() {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    let weak = Arc::downgrade(&original);
    p.send(original.clone()).await;
    drop(original);
    assert!(weak.upgrade().is_some());
    bounded(async {
        while weak.upgrade().is_some() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(p.wire().len(), 2);
    p.close().await;
}
#[tokio::test]
async fn pending_repair_expires_io_before_both_authority_readers() {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    p.gate.enabled.store(true, Ordering::SeqCst);
    nack(&p.remote, 1111, 11).await;
    bounded(p.gate.entered.notified()).await;
    assert!(p.gate.alive.load(Ordering::SeqCst));
    assert_eq!(p.gate.readers.load(Ordering::SeqCst), 2);
    assert!(p.source.try_write().is_err());
    assert!(p.target.try_write().is_err());
    bounded(async {
        while p.gate.alive.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    assert_eq!(p.gate.readers_at_drop.load(Ordering::SeqCst), 2);
    assert_eq!(p.gate.readers.load(Ordering::SeqCst), 0);
    assert!(p.source.try_write().is_ok());
    assert!(p.target.try_write().is_ok());
    p.gate.enabled.store(false, Ordering::SeqCst);
    p.gate.release.notify_one();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(p.wire().len(), 2);
    assert_eq!(original.repairs.load(Ordering::SeqCst), 1);
    p.close().await;
}
#[tokio::test]
async fn substituted_repair_payload_and_foreign_requests_refuse() {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    original.substitute.store(true, Ordering::SeqCst);
    nack(&p.remote, 9999, 11).await;
    nack(&p.remote, 1111, 99).await;
    nack(&p.remote, 1111, 11).await;
    bounded(original.entered.notified()).await;
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(original.repairs.load(Ordering::SeqCst), 1);
    assert_eq!(p.wire().len(), 2);
    original.substitute.store(false, Ordering::SeqCst);
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    assert_eq!(original.repairs.load(Ordering::SeqCst), 2);
    p.close().await;
}
#[tokio::test]
async fn four_attempts_bound_explicit_peer_nacks() {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    *p.source.try_write().unwrap() = false;
    for _ in 0..4 {
        nack(&p.remote, 1111, 11).await;
        bounded(original.entered.notified()).await;
    }
    *p.source.try_write().unwrap() = true;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(original.repairs.load(Ordering::SeqCst), 4);
    assert_eq!(p.wire().len(), 2);
    p.close().await;
}
#[tokio::test]
async fn requested_send_direction_withdrawal_refuses_existing_repair() {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    p.a.get_transceivers().await[0]
        .set_direction(rtc::rtp_transceiver::RTCRtpTransceiverDirection::Recvonly)
        .await
        .unwrap();
    nack(&p.remote, 1111, 11).await;
    bounded(original.entered.notified()).await;
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(p.wire().len(), 2);
    assert_eq!(p.gate.readers.load(Ordering::SeqCst), 0);
    p.close().await;
}
#[tokio::test]
async fn repeated_rtx_uses_distinct_sender_sequence_without_renewing_expiry() {
    let p = peers(true, true).await;
    let original = p.producer(11, true);
    p.send(original.clone()).await;
    nack(&p.remote, 1111, 11).await;
    assert_eq!(next(&p.remote).await, packet(11));
    nack(&p.remote, 1111, 11).await;
    bounded(async {
        while p.wire().len() < 4 {
            tokio::task::yield_now().await
        }
    })
    .await;
    let wire = p.wire();
    assert_eq!(wire[2].0, 97);
    assert_eq!(wire[3].0, 97);
    assert_eq!(wire[2].2, wire[3].2);
    assert_ne!(wire[2].1, wire[3].1);
    tokio::time::sleep_until(tokio::time::Instant::from_std(
        original.expires + Duration::from_millis(10),
    ))
    .await;
    nack(&p.remote, 1111, 11).await;
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(p.wire().len(), 4);
    assert_eq!(original.repairs.load(Ordering::SeqCst), 2);
    p.close().await;
}
#[test]
fn custom_wrapper_does_not_inherit_guarded_video_pipeline() {
    use rtc::{
        interceptor::{Interceptor, StreamInfo, TaggedPacket, interceptor},
        sansio,
        shared::error::Error,
    };
    #[derive(Interceptor)]
    struct Unreviewed<I: Interceptor> {
        #[next]
        inner: I,
    }
    #[interceptor]
    impl<I: Interceptor> Unreviewed<I> {}
    let known = register_owned_video_interceptors(
        &mut media(true),
        rtc::interceptor::SenderClockHandle::default(),
    )
    .unwrap()
    .build();
    assert!(known.supports_owned_video_io());
    assert!(!Unreviewed { inner: known }.supports_owned_video_io());
    assert!(!rtc::interceptor::NoopInterceptor::new().supports_owned_video_io());
}
