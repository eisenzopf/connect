use crate::{codec, failed, Metrics, QUEUE_CAPACITY};
use async_trait::async_trait;
use bytes::Bytes;
use codec_core::codecs::g711::{ulaw_compress, ulaw_expand};
use connect_connector_api::DispatchPermit;
use rvoip_core::{
    adapter::*, error::Result, ids::*, peer_media::PeerMediaFrame, stream::*, Direction,
};
use std::sync::{
    atomic::{AtomicU8, AtomicUsize, Ordering},
    Arc, Mutex,
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub(crate) const PREPARED: u8 = 0;
pub(crate) const ACTIVE: u8 = 1;
pub(crate) const STOPPED: u8 = 2;
struct Setup {
    source: mpsc::Sender<MediaFrame>,
    output: mpsc::Receiver<MediaFrame>,
    peer: mpsc::Receiver<PeerMediaFrame>,
}
pub(crate) struct FixtureStream {
    id: StreamId,
    phase: Arc<AtomicU8>,
    permit: DispatchPermit,
    input: Arc<Mutex<Option<mpsc::Receiver<MediaFrame>>>>,
    output: mpsc::Sender<MediaFrame>,
    peer: mpsc::Sender<PeerMediaFrame>,
    setup: Mutex<Option<Setup>>,
    stop: CancellationToken,
    job: tokio::sync::Mutex<Option<JoinHandle<()>>>,
    sent: Arc<AtomicUsize>,
    received: Arc<AtomicUsize>,
    last_sample: Arc<std::sync::atomic::AtomicI32>,
}
impl FixtureStream {
    pub(crate) fn new(permit: DispatchPermit) -> Arc<Self> {
        let (source, input) = mpsc::channel(QUEUE_CAPACITY);
        let (output, out_rx) = mpsc::channel(QUEUE_CAPACITY);
        let (peer, peer_rx) = mpsc::channel(QUEUE_CAPACITY);
        Arc::new(Self {
            id: StreamId::new(),
            phase: Arc::new(AtomicU8::new(PREPARED)),
            permit,
            input: Arc::new(Mutex::new(Some(input))),
            output,
            peer,
            setup: Mutex::new(Some(Setup {
                source,
                output: out_rx,
                peer: peer_rx,
            })),
            stop: CancellationToken::new(),
            job: tokio::sync::Mutex::new(None),
            sent: Arc::new(AtomicUsize::new(0)),
            received: Arc::new(AtomicUsize::new(0)),
            last_sample: Arc::new(std::sync::atomic::AtomicI32::new(0)),
        })
    }
    pub(crate) fn alive(&self) -> bool {
        self.phase.load(Ordering::Acquire) != STOPPED
            && !self.stop.is_cancelled()
            && self.permit.check().is_ok()
    }
    pub(crate) fn current(&self) -> bool {
        self.phase.load(Ordering::Acquire) == ACTIVE
            && !self.stop.is_cancelled()
            && self.permit.check().is_ok()
    }
    pub(crate) fn phase(&self) -> u8 {
        self.phase.load(Ordering::Acquire)
    }
    pub(crate) fn metrics(&self) -> Metrics {
        Metrics {
            generated: self.sent.load(Ordering::Acquire),
            decoded: self.received.load(Ordering::Acquire),
            last_sample: self.last_sample.load(Ordering::Acquire),
        }
    }
    pub(crate) fn start(
        &self,
        connection: ConnectionId,
        events: mpsc::Sender<OrchestratorAdapterEvent>,
        sink: AdapterLifecycleSinkSlot,
        owner: Arc<()>,
    ) -> Result<()> {
        let mut job = self
            .job
            .try_lock()
            .map_err(|_| failed("fixture worker busy"))?;
        let mut setup = self
            .setup
            .lock()
            .map_err(|_| failed("fixture queue unavailable"))?;
        if setup.is_none() {
            return Err(failed("fixture route cannot restart"));
        }
        self.permit
            .admit_effect()
            .map_err(|_| failed("current creation custody required"))?;
        let setup = setup
            .take()
            .ok_or_else(|| failed("fixture route cannot restart"))?;
        self.phase.store(ACTIVE, Ordering::Release);
        let phase = self.phase.clone();
        let stop = self.stop.clone();
        let permit = self.permit.clone();
        let sent = self.sent.clone();
        let received = self.received.clone();
        let last_sample = self.last_sample.clone();
        let id = self.id.clone();
        *job = Some(tokio::spawn(async move {
            let _owner = owner;
            let Setup {
                source,
                mut output,
                mut peer,
            } = setup;
            let mut clock = tokio::time::interval(std::time::Duration::from_millis(20));
            clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut timestamp = 0u32;
            let reason = 'pump: loop {
                if stop.is_cancelled() {
                    break EndReason::Normal;
                }
                if permit.check().is_err() {
                    break EndReason::Cancelled;
                }
                tokio::select! {
                    biased;
                    _=stop.cancelled()=>break 'pump EndReason::Normal,
                    _=clock.tick()=>{
                        if permit.check().is_err() || stop.is_cancelled(){break 'pump EndReason::Cancelled}
                        // Explicit deterministic fixture speaker, encoded as actual PCMU.
                        let packet=MediaFrame{stream_id:id.clone(),kind:StreamKind::Audio,payload:Bytes::from(vec![ulaw_compress(4096);160]),
                            timestamp_rtp:timestamp,source_clock:None,captured_at:chrono::Utc::now(),payload_type:Some(0)};
                        if source.try_send(packet).is_err(){break 'pump EndReason::Failed{detail:"fixture source backpressure or reader ended".into()}}
                        timestamp=timestamp.wrapping_add(160);sent.fetch_add(1,Ordering::AcqRel);
                    }
                    packet=output.recv()=>{
                        let Some(frame)=packet else {break 'pump EndReason::Normal};
                        if !decode(&frame,&permit,&stop,&received,&last_sample){break 'pump EndReason::Failed{detail:"fixture frame or custody refused".into()}}
                    }
                    packet=peer.recv()=>{
                        let Some(packet)=packet else{break 'pump EndReason::Normal};
                        let Some((frame,_delivery_guard))=packet.into_delivery() else{continue};
                        // Keep the generation guard through the actual fixture decode.
                        if !decode(&frame,&permit,&stop,&received,&last_sample){break 'pump EndReason::Failed{detail:"fixture frame or custody refused".into()}}
                    }
                }
            };
            phase.store(STOPPED, Ordering::Release);
            stop.cancel();
            drop(source);
            drop(output);
            drop(peer);
            sink.queue_or_deliver_orchestrator_terminal(
                &events,
                AdapterEvent::Ended {
                    connection_id: connection,
                    reason,
                },
            )
            .await;
        }));
        Ok(())
    }
    pub(crate) fn cancel(&self) {
        self.phase.store(STOPPED, Ordering::Release);
        self.stop.cancel();
        if let Ok(mut setup) = self.setup.lock() {
            setup.take();
        }
    }
    pub(crate) async fn drain(&self) -> Result<()> {
        self.cancel();
        self.setup
            .lock()
            .map_err(|_| failed("fixture queues unavailable"))?
            .take();
        let mut job = self.job.lock().await;
        let result = if let Some(worker) = job.as_mut() {
            match tokio::time::timeout(std::time::Duration::from_secs(2), &mut *worker).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(_)) => Err(failed("fixture worker failed")),
                Err(_) => {
                    worker.abort();
                    let _ = worker.await;
                    Err(failed("fixture cleanup deadline"))
                }
            }
        } else {
            Ok(())
        };
        // Keep the original handle in its owner across every await. Cancellation
        // of this drain leaves it available for the next cleanup attempt.
        job.take();
        result
    }
}
fn decode(
    frame: &MediaFrame,
    permit: &DispatchPermit,
    stop: &CancellationToken,
    received: &AtomicUsize,
    last: &std::sync::atomic::AtomicI32,
) -> bool {
    if stop.is_cancelled()
        || permit.check().is_err()
        || frame.kind != StreamKind::Audio
        || frame.payload_type != Some(0)
        || frame.payload.len() != 160
    {
        return false;
    }
    let decoded: Vec<i16> = frame.payload.iter().copied().map(ulaw_expand).collect();
    last.store(i32::from(decoded[0]), Ordering::Release);
    received.fetch_add(1, Ordering::AcqRel);
    true
}
impl Drop for FixtureStream {
    fn drop(&mut self) {
        self.cancel();
    }
}
#[async_trait]
impl MediaStream for FixtureStream {
    fn id(&self) -> StreamId {
        self.id.clone()
    }
    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }
    fn codec(&self) -> rvoip_core::CodecInfo {
        codec()
    }
    fn direction(&self) -> Direction {
        Direction::Outbound
    }
    fn source_ready(&self) -> bool {
        self.current()
    }
    fn retire_delivery(&self) {
        self.cancel();
    }
    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }
    fn try_frames_in(&self) -> Result<mpsc::Receiver<MediaFrame>> {
        Ok(self.reserve_frames_in()?.commit())
    }
    fn reserve_frames_in(&self) -> Result<MediaReceiverReservation> {
        if !self.current() {
            return Err(failed("fixture source not active"));
        }
        let receiver = self
            .input
            .lock()
            .map_err(|_| failed("fixture source unavailable"))?
            .take()
            .ok_or_else(|| failed("fixture source already owned"))?;
        let slot = self.input.clone();
        let stop = self.stop.clone();
        Ok(MediaReceiverReservation::new(receiver, move |receiver| {
            if !stop.is_cancelled() {
                if let Ok(mut value) = slot.lock() {
                    if value.is_none() {
                        *value = Some(receiver)
                    }
                }
            }
        }))
    }
    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.try_frames_out().unwrap_or_else(|_| {
            let (t, r) = mpsc::channel(1);
            drop(r);
            t
        })
    }
    fn try_frames_out(&self) -> Result<mpsc::Sender<MediaFrame>> {
        if !self.current() {
            return Err(failed("fixture destination not active"));
        }
        Ok(self.output.clone())
    }
    fn try_peer_frames_out(&self) -> Result<mpsc::Sender<PeerMediaFrame>> {
        if !self.current() {
            return Err(failed("fixture destination not active"));
        }
        Ok(self.peer.clone())
    }
    fn quality_snapshot(&self) -> QualitySnapshot {
        QualitySnapshot::default()
    }
    fn has_quality_measurement(&self) -> bool {
        false
    }
    async fn close(self: Arc<Self>) -> Result<()> {
        self.drain().await
    }
}
