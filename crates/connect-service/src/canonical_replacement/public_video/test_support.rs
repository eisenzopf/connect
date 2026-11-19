//! Test-support-only rendezvous AFTER actual signed B Ready capture and all
//! originating Readers release, BEFORE the first DecodedReady metadata call.
//! No native/Store observation, error, grant or opaque resource is manufactured.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

struct Captured {
    operation_record_id: String,
    operation_id: String,
    marker: wire::ReadyMarker,
    deadline: tokio::time::Instant,
}
pub(super) struct Probe {
    entered: CancellationToken,
    release: CancellationToken,
    left: CancellationToken,
    entries: AtomicUsize,
    attempts: AtomicUsize,
    busy: AtomicUsize,
    successes: AtomicUsize,
    captured: OnceLock<Captured>,
    pause_capture: bool,
    capture_cutoff: OnceLock<tokio::time::Instant>,
    capture_entered: CancellationToken,
    capture_release: CancellationToken,
    capture_left: CancellationToken,
}
/// The caller owns this test rendezvous. Only a weak pointer is installed in
/// the actual original Preparation; the probe retains no Root/runtime/Reader.
/// Drop releases the same original owner, including after a fixture failure.
pub struct PublicVideoDecodedMetadataProbe {
    state: Arc<Probe>,
    owner: std::sync::Weak<Preparation>,
}
impl PublicVideoDecodedMetadataProbe {
    /// Optional original-winner-only pause while the real admitted signed
    /// Readers are held. It is distinct from entered()'s post-release gate.
    pub async fn capture_entered(&self) {
        self.state.capture_entered.cancelled().await;
    }
    pub fn release_capture(&self) {
        self.state.capture_release.cancel();
    }
    pub async fn capture_left(&self) {
        self.state.capture_left.cancelled().await;
    }
    pub fn capture_deadline(&self) -> Option<tokio::time::Instant> {
        self.state.capture_cutoff.get().copied()
    }
    pub fn readers_released(&self) -> Option<bool> {
        self.owner
            .upgrade()
            .map(|p| p.ready_readers_released.load(Ordering::Acquire))
    }
    pub async fn entered(&self) {
        self.state.entered.cancelled().await;
    }
    pub fn release(&self) {
        self.state.release.cancel();
    }
    /// Means the rendezvous wait ended, not that metadata SQL committed.
    pub async fn left(&self) {
        self.state.left.cancelled().await;
    }
    /// Actual (stage entries, phase producer calls, returned authority-busy
    /// refusals, successful durable producer results). No simulated error.
    pub fn observed(&self) -> (usize, usize, usize, usize) {
        (
            self.state.entries.load(Ordering::Acquire),
            self.state.attempts.load(Ordering::Acquire),
            self.state.busy.load(Ordering::Acquire),
            self.state.successes.load(Ordering::Acquire),
        )
    }
    /// Immutable copy of the SAME actual captured signed marker. This is
    /// unbranded metadata; it cannot create a Ready lease or media authority.
    pub fn captured_ready(&self) -> Option<wire::ReadyMarker> {
        self.state.captured.get().map(|c| c.marker.clone())
    }
    pub fn ready_present(&self) -> bool {
        self.state.captured.get().is_some()
    }
    pub fn operation_record_id(&self) -> Option<&str> {
        self.state
            .captured
            .get()
            .map(|c| c.operation_record_id.as_str())
    }
    pub fn operation_id(&self) -> Option<&str> {
        self.state.captured.get().map(|c| c.operation_id.as_str())
    }
    pub fn original_deadline(&self) -> Option<tokio::time::Instant> {
        self.state.captured.get().map(|c| c.deadline)
    }
}
impl Drop for PublicVideoDecodedMetadataProbe {
    fn drop(&mut self) {
        self.state.capture_release.cancel();
        self.state.release.cancel();
        if let Some(owner) = self.owner.upgrade() {
            if let Ok(mut slot) = owner.decoded_metadata_probe.lock() {
                if slot
                    .as_ref()
                    .is_some_and(|old| std::sync::Weak::ptr_eq(old, &Arc::downgrade(&self.state)))
                {
                    *slot = None;
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
    pub(super) fn record_attempt(&self) {
        self.attempts.fetch_add(1, Ordering::AcqRel);
    }
    pub(super) fn record_result<T>(&self, result: &Result<T>) {
        match result {
            Ok(_) => {
                self.successes.fetch_add(1, Ordering::AcqRel);
            }
            Err(error) if error.code == "authority-busy" => {
                self.busy.fetch_add(1, Ordering::AcqRel);
            }
            Err(_) => {}
        }
    }
    fn capture_original(&self, root: &Arc<PreparedReplacement>) -> Result<()> {
        let marker = root
            .public_video
            .ready
            .get()
            .ok_or_else(Error::forbidden)?
            .clone();
        if let Some(original) = self.captured.get() {
            if original.operation_record_id != root.origin.record_id()
                || original.operation_id != root.origin.request().operation_id
                || original.marker != marker
                || original.deadline != root.deadline
            {
                return Err(Error::forbidden());
            }
            return Ok(());
        }
        self.captured
            .set(Captured {
                operation_record_id: root.origin.record_id().into(),
                operation_id: root.origin.request().operation_id.clone(),
                marker,
                deadline: root.deadline,
            })
            .map_err(|_| Error::forbidden())
    }
    async fn pause_capture(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        until: tokio::time::Instant,
        cancellation: [&CancellationToken; 4],
    ) -> Result<()> {
        if !self.pause_capture {
            return Ok(());
        }
        struct CaptureLeaving(Arc<Probe>);
        impl Drop for CaptureLeaving {
            fn drop(&mut self) {
                self.0.capture_left.cancel();
            }
        }
        let _leaving = CaptureLeaving(self.clone());
        let cutoff = until.min(root.deadline);
        if cancellation.iter().any(|token| token.is_cancelled())
            || tokio::time::Instant::now() >= cutoff
        {
            return Err(Error::forbidden());
        }
        if root
            .public_video
            .ready_readers_released
            .load(Ordering::Acquire)
        {
            return Err(Error::forbidden());
        }
        self.capture_original(root)?;
        self.capture_cutoff
            .set(cutoff)
            .map_err(|_| Error::forbidden())?;
        self.capture_entered.cancel();
        tokio::select! { biased;
            _ = cancellation[0].cancelled() => Err(Error::forbidden()),
            _ = cancellation[1].cancelled() => Err(Error::forbidden()),
            _ = cancellation[2].cancelled() => Err(Error::forbidden()),
            _ = cancellation[3].cancelled() => Err(Error::forbidden()),
            _ = tokio::time::sleep_until(cutoff) => Err(Error::new("authority-busy", "Original Ready capture probe cutoff elapsed")),
            _ = self.capture_release.cancelled() => Ok(()),
        }
    }
    async fn pause(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        stop: &CancellationToken,
    ) -> Result<()> {
        let _leaving = Leaving(self.clone());
        let cancel = root.handle.cancellation();
        let cutoff = root.deadline;
        if stop.is_cancelled()
            || cancel.is_cancelled()
            || tokio::time::Instant::now() >= cutoff
            || !root
                .public_video
                .ready_readers_released
                .load(Ordering::Acquire)
        {
            return Err(Error::forbidden());
        }
        if self.entries.fetch_add(1, Ordering::AcqRel) != 0 {
            return Err(Error::forbidden());
        }
        self.capture_original(root)?;
        self.entered.cancel();
        tokio::select! { biased;
            _ = stop.cancelled() => Err(Error::forbidden()),
            _ = cancel.cancelled() => Err(Error::forbidden()),
            _ = tokio::time::sleep_until(cutoff) => Err(Error::new("authority-busy", "Original decoded metadata probe cutoff elapsed")),
            _ = self.release.cancelled() => Ok(()),
        }
    }
}
impl Preparation {
    pub(super) async fn pause_captured_ready_probe_for_test(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        until: tokio::time::Instant,
        cancellation: [&CancellationToken; 4],
    ) -> Result<()> {
        if !Arc::ptr_eq(self, &root.public_video) {
            return Err(Error::forbidden());
        }
        let probe = self
            .decoded_metadata_probe
            .lock()
            .map_err(|_| Error::storage())?
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        if let Some(probe) = probe {
            probe.pause_capture(root, until, cancellation).await?;
        }
        Ok(())
    }
    pub(super) async fn pause_decoded_metadata_probe_for_test(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        stop: &CancellationToken,
    ) -> Result<Option<Arc<Probe>>> {
        if !Arc::ptr_eq(self, &root.public_video) {
            return Err(Error::forbidden());
        }
        let probe = self
            .decoded_metadata_probe
            .lock()
            .map_err(|_| Error::storage())?
            .take()
            .and_then(|weak| weak.upgrade());
        if let Some(probe) = &probe {
            probe.pause(root, stop).await?;
        }
        Ok(probe)
    }
}
impl Runtime {
    fn arm_public_video_decoded_metadata_probe_for_test(
        &self,
        operation_id: &str,
        pause_capture: bool,
    ) -> Result<PublicVideoDecodedMetadataProbe> {
        if operation_id.is_empty() || operation_id.len() > 256 {
            return Err(Error::forbidden());
        }
        // Fixture arms at the genuine pre-public-marker checkpoint: this
        // original Root already exists. No global/pre-Root probe registry.
        let root = {
            let roots = self.replacements.lock().map_err(|_| Error::storage())?;
            let mut matching = roots
                .values()
                .filter(|r| r.origin.request().operation_id == operation_id);
            let root = matching.next().cloned().ok_or_else(Error::forbidden)?;
            if matching.next().is_some() {
                return Err(Error::forbidden());
            }
            root
        };
        let p = &root.public_video;
        let mut slot = p
            .decoded_metadata_probe
            .lock()
            .map_err(|_| Error::storage())?;
        if !p.required()
            || p.ready.get().is_some()
            || p.ready_recorded.load(Ordering::Acquire)
            || root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || self.stop.is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
            || slot.as_ref().is_some_and(|old| old.strong_count() != 0)
        {
            return Err(Error::forbidden());
        }
        let state = Arc::new(Probe {
            entered: CancellationToken::new(),
            release: CancellationToken::new(),
            left: CancellationToken::new(),
            entries: AtomicUsize::new(0),
            attempts: AtomicUsize::new(0),
            busy: AtomicUsize::new(0),
            successes: AtomicUsize::new(0),
            captured: OnceLock::new(),
            pause_capture,
            capture_cutoff: OnceLock::new(),
            capture_entered: CancellationToken::new(),
            capture_release: CancellationToken::new(),
            capture_left: CancellationToken::new(),
        });
        *slot = Some(Arc::downgrade(&state));
        Ok(PublicVideoDecodedMetadataProbe {
            state,
            owner: Arc::downgrade(p),
        })
    }
}
impl crate::Service {
    /// Same probe plus an optional FIRST-capture pause holding actual signed
    /// Readers. Replays cannot release that handler's publication predicate.
    pub fn arm_canonical_public_video_reader_release_probe_for_test(
        &self,
        operation_id: &str,
    ) -> Result<PublicVideoDecodedMetadataProbe> {
        self.canonical_runtime
            .get()
            .ok_or_else(Error::forbidden)?
            .arm_public_video_decoded_metadata_probe_for_test(operation_id, true)
    }
    /// Test support only. Requires the actual opted-in original Root to exist
    /// and not yet have captured public Ready. The returned owner is inferred;
    /// no opaque native/Core constructor or readiness grant is exposed.
    pub fn arm_canonical_public_video_decoded_metadata_probe_for_test(
        &self,
        operation_id: &str,
    ) -> Result<PublicVideoDecodedMetadataProbe> {
        self.canonical_runtime
            .get()
            .ok_or_else(Error::forbidden)?
            .arm_public_video_decoded_metadata_probe_for_test(operation_id, false)
    }
}
