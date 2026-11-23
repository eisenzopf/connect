//! Coordinate recording of one immutable physical outcome, never its authority.
use super::*;
use connect_store::NativeReplacementCommitted;
use std::future::Future;
use tokio::{sync::Notify, time::Instant};
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

fn retry_delay(code: &str, video_cutover: bool, elapsed: Duration) -> Option<Duration> {
    let millis = match code {
        "authority-busy" if video_cutover && elapsed < Duration::from_millis(60) => 2,
        "authority-busy" if video_cutover && elapsed < Duration::from_millis(250) => 8,
        "authority-busy" | "storage-unavailable" => 20,
        _ => return None,
    };
    Some(Duration::from_millis(millis))
}

/// The old video is already paused during public cutover. Retry transient
/// admission promptly at first, then back off under sustained contention.
/// Only immutable metadata belongs here. Every Store call has returned and
/// released its guards before the next wait.
pub(super) async fn retry_captured_metadata<T, F, Fut>(
    stop: &CancellationToken,
    until: Instant,
    video_cutover: bool,
    mut call: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let began = Instant::now();
    loop {
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if Instant::now() >= until {
            return Err(Error::new(
                "authority-busy",
                "Original replacement metadata cutoff elapsed",
            ));
        }
        let error = match call().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        let Some(delay) = retry_delay(&error.code, video_cutover, began.elapsed()) else {
            return Err(error);
        };
        tokio::select! { biased;
            _ = stop.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until) => return Err(error),
            _ = tokio::time::sleep(delay) => {},
        }
    }
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
        video_cutover: bool,
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
            video_cutover,
            body,
        )
        .await
    }

    async fn record_until<T, F, G>(
        self: &Arc<Self>,
        stop: &CancellationToken,
        until: tokio::time::Instant,
        video_cutover: bool,
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
        retry_captured_metadata(stop, until, video_cutover, || {
            store_call(owner.clone().call(body()))
        })
        .await
    }
}

#[cfg(test)]
mod tests;
