//! G6-051 local production-library probe. Run with the exact ordinary libraries
//! emitted by the Connect server build (see webrtc-owned-rtp.json).
//! The permission policy here is an instrumented RwLock fixture. This checks
//! driver callback/IO/receipt custody; it does not qualify Core QueuedMedia,
//! Store media authorization, canonical media readiness, or browser fallback.
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
struct Producer {
    policy: Arc<RwLock<bool>>,
    gate: Arc<Gate>,
    packet: rtp::Packet,
    calls: AtomicUsize,
}
struct Reader(Arc<Gate>);
impl Drop for Reader {
    fn drop(&mut self) {
        self.0.readers.fetch_sub(1, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl OwnedRtpProducer for Producer {
    async fn write(&self, io: &mut dyn OwnedRtpIo) -> OwnedRtpResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let source = self.policy.read().await;
        let target = self.policy.read().await;
        if !*source || !*target {
            return Err(OwnedRtpError::Refused);
        }
        self.gate.readers.fetch_add(2, Ordering::SeqCst);
        let _s = Reader(self.gate.clone());
        let _t = Reader(self.gate.clone());
        io.write(self.packet.clone()).await.map_err(|e| {
            eprintln!("actual sink error: {e:?}");
            OwnedRtpError::Refused
        })
    }
}
struct Noop;
#[async_trait::async_trait]
impl OwnedRtpProducer for Noop {
    async fn write(&self, _: &mut dyn OwnedRtpIo) -> OwnedRtpResult {
        Ok(())
    }
}
struct Barrier {
    entered: Notify,
    release: Notify,
}
#[async_trait::async_trait]
impl OwnedRtpProducer for Barrier {
    async fn write(&self, _: &mut dyn OwnedRtpIo) -> OwnedRtpResult {
        self.entered.notify_one();
        self.release.notified().await;
        Err(OwnedRtpError::Refused)
    }
}
async fn bounded<T>(f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), f)
        .await
        .expect("bounded actual RTC event")
}
async fn exercise() {
    let gate = Arc::new(Gate::default());
    let policy = Arc::new(RwLock::new(true));
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
    let producer = |seq| {
        Arc::new(Producer {
            policy: policy.clone(),
            gate: gate.clone(),
            packet: packet(seq),
            calls: AtomicUsize::new(0),
        })
    };
    let owner = t.owned_rtp(producer(1), Instant::now() + Duration::from_millis(200));
    assert_eq!(bounded(owner.result()).await, Ok(()));
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
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    println!(
        "PASS actual ICE/DTLS/SRTP G.711 plaintext delivery, exact SSRC/PT/sequence/timestamp/payload"
    );
    let no = t.owned_rtp(Arc::new(Noop), Instant::now() + Duration::from_millis(200));
    assert_eq!(no.result().await, Err(OwnedRtpError::Refused));
    no.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 1);
    println!("PASS no-IO success cannot forge a delivery receipt");
    for (seq, ssrc, pt) in [(2, 9999, 0), (3, 1111, 111)] {
        let mut p = producer(seq);
        Arc::get_mut(&mut p).unwrap().packet.header.ssrc = ssrc;
        Arc::get_mut(&mut p).unwrap().packet.header.payload_type = pt;
        let o = t.owned_rtp(p, Instant::now() + Duration::from_millis(200));
        assert_eq!(o.result().await, Err(OwnedRtpError::Refused));
        o.cancel_and_drain().await.unwrap();
    }
    assert_eq!(gate.sends.load(Ordering::SeqCst), 1);
    println!("PASS mismatched sender SSRC and unnegotiated payload refuse before UDP");
    let barrier = Arc::new(Barrier {
        entered: Notify::new(),
        release: Notify::new(),
    });
    let hold = t.owned_rtp(barrier.clone(), Instant::now() + Duration::from_millis(200));
    barrier.entered.notified().await;
    let p = producer(4);
    let expired = t.owned_rtp(p.clone(), Instant::now() + Duration::from_millis(5));
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(gate.readers.load(Ordering::SeqCst), 0);
    barrier.release.notify_one();
    assert_eq!(hold.result().await, Err(OwnedRtpError::Refused));
    assert_eq!(
        expired.result().await,
        Err(OwnedRtpError::DeadlineBeforeWrite)
    );
    assert_eq!(p.calls.load(Ordering::SeqCst), 0);
    expired.cancel_and_drain().await.unwrap();
    println!("PASS queued expiry invokes no producer and retains no permission reader");
    let barrier = Arc::new(Barrier {
        entered: Notify::new(),
        release: Notify::new(),
    });
    let hold = t.owned_rtp(barrier.clone(), Instant::now() + Duration::from_millis(200));
    barrier.entered.notified().await;
    let p = producer(5);
    let revoked = t.owned_rtp(p.clone(), Instant::now() + Duration::from_millis(200));
    *policy.try_write().expect("no enqueued permission lease") = false;
    barrier.release.notify_one();
    assert_eq!(hold.result().await, Err(OwnedRtpError::Refused));
    assert_eq!(revoked.result().await, Err(OwnedRtpError::Refused));
    assert_eq!(p.calls.load(Ordering::SeqCst), 1);
    assert_eq!(gate.sends.load(Ordering::SeqCst), 1);
    *policy.write().await = true;
    println!("PASS revocation between enqueue and callback refuses without wire submission");
    gate.enabled.store(true, Ordering::SeqCst);
    let owner = t.owned_rtp(producer(6), Instant::now() + Duration::from_millis(200));
    bounded(gate.entered.notified()).await;
    assert!(gate.alive.load(Ordering::SeqCst));
    assert_eq!(gate.readers.load(Ordering::SeqCst), 2);
    assert!(policy.try_write().is_err());
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
    assert!(policy.try_write().is_ok());
    gate.enabled.store(false, Ordering::SeqCst);
    gate.release.notify_one();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(gate.sends.load(Ordering::SeqCst), 1);
    println!(
        "PASS cancelled observer preserves original IO; actual UDP future drops before both permission readers; original drain acknowledges; no late packet"
    );
    let next = t.owned_rtp(producer(7), Instant::now() + Duration::from_millis(200));
    assert_eq!(next.result().await, Ok(()));
    next.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 2);
    println!("PASS same RTC peer remains healthy after cancellation");
    let tr = a.get_transceivers().await;
    tr[0]
        .set_direction(rtc::rtp_transceiver::RTCRtpTransceiverDirection::Recvonly)
        .await
        .unwrap();
    let stopped = t.owned_rtp(producer(8), Instant::now() + Duration::from_millis(200));
    assert_eq!(stopped.result().await, Err(OwnedRtpError::Refused));
    stopped.cancel_and_drain().await.unwrap();
    assert_eq!(gate.sends.load(Ordering::SeqCst), 2);
    println!("PASS immediate requested send-direction withdrawal refuses output");
    bounded(a.close()).await.unwrap();
    bounded(b.close()).await.unwrap();
}
fn main() {
    webrtc::runtime::block_on(exercise());
}
