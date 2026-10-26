//! An owned local device on an already negotiated public native audio peer.
use super::PublicNativeAudio;
use crate::{ClientError, Result};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

const FRAME_BYTES: usize = 160;
const DEVICE_WRITE: Duration = Duration::from_millis(100);
const CLEANUP: Duration = Duration::from_secs(5);

/// Local mono PCMU device: 8 kHz, 160 encoded bytes per 20 ms frame.
///
/// Receive and send must be cancellation safe. Cancel must be idempotent and synchronously fence
/// admission and request worker stop without blocking. Drain observes the same
/// owned workers after cancellation; cancelling its future must retain cleanup
/// custody for another attempt. This device supplies no Conversation authority.
#[async_trait::async_trait]
pub trait PcmuDevice: Send + Sync + 'static {
    async fn receive(&self) -> Result<Vec<u8>>;
    async fn send(&self, frame: &[u8]) -> Result<()>;
    fn cancel(&self);
    async fn drain(&self) -> Result<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcmuBridgeExit {
    Stopped,
    PeerEnded,
    DeviceEnded,
    InvalidFrame,
}

/// Submission counters describe accepted IO calls, not remote decoded delivery.
/// A stopped worker alone does not prove either resource drained successfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmuBridgeReport {
    pub exit: PcmuBridgeExit,
    pub native_submissions: u64,
    pub device_submissions: u64,
    pub native_drained: bool,
    pub device_drained: bool,
}
impl PcmuBridgeReport {
    pub fn cleanup_confirmed(&self) -> bool {
        self.native_drained && self.device_drained
    }
}

struct Drain {
    job: Option<JoinHandle<PcmuBridgeReport>>,
    report: Option<PcmuBridgeReport>,
    worker_failed: bool,
}

/// Owns one actual signed public media peer and one explicitly selected device.
/// There is at most one buffered device frame. Every wire exchange uses the
/// existing SDK peer, whose server applies current media policy. This bridge
/// never provisions an Endpoint, negotiates another route or retries a command.
pub struct PublicPcmuBridge {
    device: Arc<dyn PcmuDevice>,
    stop: CancellationToken,
    drain: Mutex<Drain>,
}
impl PublicPcmuBridge {
    pub fn start(audio: PublicNativeAudio, device: Arc<dyn PcmuDevice>) -> Self {
        let stop = CancellationToken::new();
        let job = tokio::spawn(run(audio, device.clone(), stop.clone()));
        Self {
            device,
            stop,
            drain: Mutex::new(Drain {
                job: Some(job),
                report: None,
                worker_failed: false,
            }),
        }
    }
    pub fn cancel(&self) {
        self.stop.cancel();
        self.device.cancel();
    }
    /// Cancelling this wait leaves the original join handle owned here. A later
    /// drain observes that same worker and replays its terminal report. Inspect
    /// cleanup_confirmed even when the worker itself returned a report.
    pub async fn drain(&self) -> Result<PcmuBridgeReport> {
        self.cancel();
        let mut state = self.drain.lock().await;
        if let Some(report) = state.report {
            return Ok(report);
        }
        if state.worker_failed {
            return Err(ClientError::Transport);
        }
        let job = state.job.as_mut().ok_or(ClientError::Protocol)?;
        let joined = tokio::time::timeout(CLEANUP + Duration::from_secs(1), job)
            .await
            .map_err(|_| ClientError::Timeout)?;
        let report = match joined {
            Ok(report) => report,
            Err(_) => {
                state.job = None;
                state.worker_failed = true;
                return Err(ClientError::Transport);
            }
        };
        state.job = None;
        state.report = Some(report);
        Ok(report)
    }
}
impl Drop for PublicPcmuBridge {
    fn drop(&mut self) {
        // Dropping a Tokio handle detaches, so the requested worker keeps owning
        // and closing both resources. Only an observed report proves cleanup.
        self.cancel();
    }
}

struct StopDevice(Arc<dyn PcmuDevice>);
impl Drop for StopDevice {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn run(
    mut audio: PublicNativeAudio,
    device: Arc<dyn PcmuDevice>,
    stop: CancellationToken,
) -> PcmuBridgeReport {
    let _cleanup = StopDevice(device.clone());
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pending: Option<Vec<u8>> = None;
    let mut native_submissions = 0;
    let mut device_submissions = 0;
    let exit = loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break PcmuBridgeExit::Stopped,
            _ = tick.tick() => {
                if let Some(frame) = pending.take() {
                    if audio.send_pcmu(&frame).is_err() { break PcmuBridgeExit::PeerEnded; }
                    native_submissions += 1;
                }
            },
            frame = device.receive(), if pending.is_none() => {
                match frame {
                    Ok(frame) if frame.len() == FRAME_BYTES => pending = Some(frame),
                    Ok(_) => break PcmuBridgeExit::InvalidFrame,
                    Err(_) => break PcmuBridgeExit::DeviceEnded,
                }
            },
            frame = audio.receive_pcmu() => {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(ClientError::Timeout) => continue,
                    Err(_) => break PcmuBridgeExit::PeerEnded,
                };
                if frame.len() != FRAME_BYTES { break PcmuBridgeExit::InvalidFrame; }
                let sent = tokio::select! {
                    biased;
                    _ = stop.cancelled() => break PcmuBridgeExit::Stopped,
                    sent = tokio::time::timeout(DEVICE_WRITE, device.send(&frame)) => sent,
                };
                if !matches!(sent, Ok(Ok(()))) { break PcmuBridgeExit::DeviceEnded; }
                device_submissions += 1;
            }
        }
    };
    stop.cancel();
    device.cancel();
    audio.close();
    let (native, local) = tokio::join!(
        tokio::time::timeout(CLEANUP, audio.shutdown()),
        tokio::time::timeout(CLEANUP, device.drain()),
    );
    PcmuBridgeReport {
        exit,
        native_submissions,
        device_submissions,
        native_drained: matches!(native, Ok(Ok(()))),
        device_drained: matches!(local, Ok(Ok(()))),
    }
}
