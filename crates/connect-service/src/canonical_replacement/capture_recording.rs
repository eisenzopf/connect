//! Coordinate recording of one immutable physical outcome, never its authority.
use super::*;
use connect_store::NativeReplacementCommitted;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub(super) struct KnownCapture {
    pub value: Arc<NativeReplacementCommitted>,
    pub recording: Arc<Recorder>,
}
impl KnownCapture {
    pub(super) fn new(value: NativeReplacementCommitted) -> Self {
        Self {
            value: Arc::new(value),
            recording: Arc::new(Recorder::default()),
        }
    }
}

#[derive(Default)]
pub(super) struct Recorder {
    active: AtomicBool,
    released: Notify,
}
/// The async retry owner and its actual blocking call share this exact token.
/// Cancelling the future cannot release an independently running Store call.
pub(super) struct Recording {
    recorder: Arc<Recorder>,
}
impl Drop for Recording {
    fn drop(&mut self) {
        self.recorder.active.store(false, Ordering::Release);
        self.recorder.released.notify_waiters();
    }
}
impl Recording {
    pub(super) fn call<T, F>(self: Arc<Self>, body: F) -> impl FnOnce() -> Result<T> + Send
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        move || {
            let _recording = self;
            body()
        }
    }
}

fn busy() -> Error {
    Error::new(
        "authority-busy",
        "Original captured replacement recorder is busy",
    )
}

impl Recorder {
    pub(super) fn try_acquire(self: &Arc<Self>) -> Option<Arc<Recording>> {
        self.active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| {
                Arc::new(Recording {
                    recorder: self.clone(),
                })
            })
    }

    /// Keep cleanup's existing one-call/200ms retry policy. Cancellation may
    /// end that loop only after its original historical Store attempt runs.
    /// The result is a loop decision, not a successful-settlement assertion.
    pub(super) async fn cleanup_attempt<F>(
        self: &Arc<Self>,
        stop: &CancellationToken,
        body: F,
    ) -> bool
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        if let Some(recording) = self.try_acquire() {
            let settled = store_call(recording.call(body)).await;
            settled.is_ok() || stop.is_cancelled()
        } else {
            // Another retry owner can still be between Store calls. It must
            // release actual custody before cleanup makes its final attempt.
            false
        }
    }

    async fn acquire(
        self: &Arc<Self>,
        stop: &CancellationToken,
        until: tokio::time::Instant,
    ) -> Result<Arc<Recording>> {
        loop {
            let released = self.released.notified();
            tokio::pin!(released);
            // Register before checking state: a release between the check and
            // awaiting this notification must not leave the follower asleep.
            released.as_mut().enable();
            if stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            if tokio::time::Instant::now() >= until {
                return Err(busy());
            }
            if let Some(owner) = self.try_acquire() {
                return Ok(owner);
            }
            tokio::select! { biased;
                _ = stop.cancelled() => return Err(Error::forbidden()),
                _ = tokio::time::sleep_until(until) => return Err(busy()),
                _ = &mut released => {},
            }
        }
    }

    pub(super) async fn record<T, F, G>(
        self: &Arc<Self>,
        stop: &CancellationToken,
        body: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnMut() -> G,
        G: FnOnce() -> Result<T> + Send + 'static,
    {
        self.record_until(
            stop,
            tokio::time::Instant::now() + Duration::from_secs(5),
            body,
        )
        .await
    }

    async fn record_until<T, F, G>(
        self: &Arc<Self>,
        stop: &CancellationToken,
        until: tokio::time::Instant,
        mut body: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnMut() -> G,
        G: FnOnce() -> Result<T> + Send + 'static,
    {
        // Waiting and retry sleep consume the same original five seconds.
        // No SQL/file reader or successful permission is stored in this owner.
        let owner = self.acquire(stop, until).await?;
        loop {
            if stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            if tokio::time::Instant::now() >= until {
                return Err(busy());
            }
            let call = body();
            let result = store_call(owner.clone().call(call)).await;
            match result {
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) && tokio::time::Instant::now() < until =>
                {
                    tokio::select! { biased;
                        _ = stop.cancelled() => return Err(Error::forbidden()),
                        _ = tokio::time::sleep_until(until) => return Err(error),
                        _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                    }
                }
                result => return result,
            }
        }
    }
}

#[cfg(test)]
mod tests;
