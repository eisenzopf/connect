//! One exact original video's genuine mutex exclusion, with no media grant.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

pub(crate) struct Probe {
    entered: CancellationToken,
    release: CancellationToken,
    left: CancellationToken,
    calls: AtomicUsize,
    busy: AtomicBool,
    before: AtomicUsize,
    after: AtomicUsize,
}
/// Drop releases the same original preparation owner, including on assertion
/// failure. The registry holds only a weak identity, never this owner's grant.
pub struct VideoPreparationProbe {
    probe: Arc<Probe>,
    owner: Weak<Policy>,
    operation: String,
}
impl VideoPreparationProbe {
    pub async fn entered(&self) {
        self.probe.entered.cancelled().await;
    }
    pub fn release(&self) {
        self.probe.release.cancel();
    }
    pub async fn left(&self) {
        self.probe.left.cancelled().await;
    }
    pub fn observed(&self) -> (usize, bool, usize, usize) {
        (
            self.probe.calls.load(Ordering::Acquire),
            self.probe.busy.load(Ordering::Acquire),
            self.probe.before.load(Ordering::Acquire),
            self.probe.after.load(Ordering::Acquire),
        )
    }
}
impl Drop for VideoPreparationProbe {
    fn drop(&mut self) {
        self.probe.release.cancel();
        if let Some(owner) = self.owner.upgrade() {
            if let Ok(mut probes) = owner.video_preparation_probes.lock() {
                if probes
                    .get(&self.operation)
                    .is_some_and(|old| Weak::ptr_eq(old, &Arc::downgrade(&self.probe)))
                {
                    probes.remove(&self.operation);
                }
            }
        }
    }
}
struct Leaving(Arc<Probe>);
impl Drop for Leaving {
    fn drop(&mut self) {
        self.0.left.cancel();
    }
}
impl Probe {
    pub(crate) fn record(
        &self,
        busy: bool,
        before: usize,
        after: usize,
    ) -> connect_protocol::Result<()> {
        if self.calls.fetch_add(1, Ordering::AcqRel) != 0 {
            return Err(connect_protocol::Error::forbidden());
        }
        self.busy.store(busy, Ordering::Release);
        self.before.store(before, Ordering::Release);
        self.after.store(after, Ordering::Release);
        if !busy || before != 0 || after != 0 {
            return Err(connect_protocol::Error::forbidden());
        }
        Ok(())
    }
    /// Called only after Core guards, the fresh capture and role coordinator
    /// have all dropped. Uses the same original task/cancel/cutoff, no worker.
    pub(crate) async fn pause(
        self: &Arc<Self>,
        stop: &CancellationToken,
        cancel: &CancellationToken,
        cutoff: tokio::time::Instant,
    ) -> connect_protocol::Result<()> {
        let _leaving = Leaving(self.clone());
        self.entered.cancel();
        tokio::select! { biased;
            _ = stop.cancelled() => Err(connect_protocol::Error::forbidden()),
            _ = cancel.cancelled() => Err(connect_protocol::Error::forbidden()),
            _ = tokio::time::sleep_until(cutoff) => Err(connect_protocol::Error::new("authority-busy", "Original video probe cutoff elapsed")),
            _ = self.release.cancelled() => Ok(()),
        }
    }
}
impl Policy {
    pub(crate) fn arm_video_preparation_probe_for_test(
        self: &Arc<Self>,
        operation: &str,
    ) -> connect_protocol::Result<VideoPreparationProbe> {
        if operation.is_empty() || operation.len() > 256 {
            return Err(connect_protocol::Error::forbidden());
        }
        let mut probes = self
            .video_preparation_probes
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        probes.retain(|_, old| old.strong_count() != 0);
        if probes.len() >= self.capacity || probes.contains_key(operation) {
            return Err(connect_protocol::Error::forbidden());
        }
        let probe = Arc::new(Probe {
            entered: CancellationToken::new(),
            release: CancellationToken::new(),
            left: CancellationToken::new(),
            calls: AtomicUsize::new(0),
            busy: AtomicBool::new(false),
            before: AtomicUsize::new(usize::MAX),
            after: AtomicUsize::new(usize::MAX),
        });
        probes.insert(operation.into(), Arc::downgrade(&probe));
        Ok(VideoPreparationProbe {
            probe,
            owner: Arc::downgrade(self),
            operation: operation.into(),
        })
    }
    pub(crate) fn take_video_preparation_probe_for_test(
        &self,
        operation: &str,
    ) -> Option<Arc<Probe>> {
        self.video_preparation_probes
            .lock()
            .ok()?
            .remove(operation)?
            .upgrade()
    }
}
