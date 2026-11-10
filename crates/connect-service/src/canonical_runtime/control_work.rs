//! Original native control ownership, bounded by the existing runtime capacity.
use super::*;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use tokio::sync::Notify;

struct Job {
    handle: tokio::task::JoinHandle<Poll<()>>,
    waiting: bool,
}
#[derive(Default)]
struct Jobs {
    next: u64,
    closed: bool,
    active: BTreeMap<u64, Job>,
}
/// A running synchronous poll outlives cancellation of its async waiter. Its
/// actual JoinHandle remains here until joined, including during shutdown.
#[derive(Default)]
pub(super) struct Polls {
    jobs: Mutex<Jobs>,
    failed: AtomicBool,
}
impl Polls {
    pub(super) fn len(&self) -> Option<usize> {
        Some(self.jobs.try_lock().ok()?.active.len())
    }
    fn reap(&self, jobs: &mut Jobs) {
        jobs.active.retain(|_, job| {
            if job.waiting || !job.handle.is_finished() {
                return true;
            }
            match Pin::new(&mut job.handle).poll(&mut Context::from_waker(Waker::noop())) {
                Poll::Ready(result) => {
                    if result.is_err() {
                        self.failed.store(true, Ordering::Release);
                    }
                    false
                }
                Poll::Pending => true,
            }
        });
    }
    fn submit(
        self: &Arc<Self>,
        limit: usize,
        poll: impl FnOnce() -> Poll<()> + Send + 'static,
    ) -> Result<OwnedPoll> {
        let mut jobs = self.jobs.lock().map_err(|_| Error::storage())?;
        self.reap(&mut jobs);
        if jobs.closed || jobs.active.len() >= limit {
            return Err(Error::new(
                "busy",
                "Native control poll custody is unavailable",
            ));
        }
        let id = jobs.next.checked_add(1).ok_or_else(Error::storage)?;
        jobs.next = id;
        jobs.active.insert(
            id,
            Job {
                handle: crate::runtime_retention::spawn_blocking(
                    crate::runtime_retention::Kind::NativeControlPoll,
                    poll,
                ),
                waiting: true,
            },
        );
        Ok(OwnedPoll {
            owner: self.clone(),
            id,
            finished: false,
        })
    }
}
/// Polling a JoinHandle holds only the short registry mutex. The blocking call
/// never holds it. Dropping this waiter leaves the original handle registered.
struct OwnedPoll {
    owner: Arc<Polls>,
    id: u64,
    finished: bool,
}
impl Future for OwnedPoll {
    type Output = Result<Poll<()>>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = {
            let mut jobs = match self.owner.jobs.lock() {
                Ok(jobs) => jobs,
                Err(_) => {
                    self.owner.failed.store(true, Ordering::Release);
                    return Poll::Ready(Err(Error::storage()));
                }
            };
            let Some(job) = jobs.active.get_mut(&self.id) else {
                self.owner.failed.store(true, Ordering::Release);
                return Poll::Ready(Err(Error::storage()));
            };
            match Pin::new(&mut job.handle).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => {
                    jobs.active.remove(&self.id);
                    result
                }
            }
        };
        self.finished = true;
        Poll::Ready(result.map_err(|_| {
            self.owner.failed.store(true, Ordering::Release);
            Error::storage()
        }))
    }
}
impl Drop for OwnedPoll {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let mut jobs = self.owner.jobs.lock().unwrap_or_else(|p| {
            self.owner.failed.store(true, Ordering::Release);
            p.into_inner()
        });
        if let Some(job) = jobs.active.get_mut(&self.id) {
            job.waiting = false;
        }
    }
}
struct WakeControl(Arc<Notify>);
impl Wake for WakeControl {
    fn wake(self: Arc<Self>) {
        self.0.notify_one();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.notify_one();
    }
}
type OriginalControl = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Poll the same pinned control future on the blocking pool. A poll may run
/// synchronous Store/Core metadata; Pending releases that thread immediately.
/// Only the future's real waker permits another poll. No effect, attempt or
/// timeout is recreated, and no pool thread is parked across an async wait.
async fn drive(runtime: Arc<Runtime>, worker: OriginalControl) -> Result<()> {
    let original = Arc::new(Mutex::new(Some(worker)));
    let ready = Arc::new(Notify::new());
    let waker = Waker::from(Arc::new(WakeControl(ready.clone())));
    loop {
        let state = original.clone();
        let waker = waker.clone();
        let outcome = runtime
            .control_polls
            .submit(
                runtime.media.store.limits().connections.saturating_mul(2),
                move || {
                    let mut original = state.lock().unwrap_or_else(|p| p.into_inner());
                    let outcome = original
                        .as_mut()
                        .expect("one original control future")
                        .as_mut()
                        .poll(&mut Context::from_waker(&waker));
                    if outcome.is_ready() {
                        *original = None;
                    }
                    outcome
                },
            )?
            .await?;
        match outcome {
            Poll::Ready(()) => return Ok(()),
            // Notify retains a wake during the poll or before this await.
            // Join the actual poll before another can start; sending its
            // result alone would race task retirement and capacity accounting.
            Poll::Pending => ready.notified().await,
        }
    }
}
impl Runtime {
    /// Retain the actual blocking handle through waiter cancellation using the
    /// existing control-poll custody/drain. A dropped result receiver releases
    /// any returned authority inside the original closure, before it exits.
    pub(crate) async fn owned_store_call<T: Send + 'static>(
        &self,
        call: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (sent, result) = tokio::sync::oneshot::channel();
        let original = self.control_polls.submit(
            self.media.store.limits().connections.saturating_mul(2),
            move || {
                let value = call();
                // Sending to a cancelled waiter returns the value; drop it
                // here rather than retaining a late Write in the registry.
                drop(sent.send(value));
                Poll::Ready(())
            },
        )?;
        match original.await? {
            Poll::Ready(()) => result.await.map_err(|_| Error::storage())?,
            Poll::Pending => Err(Error::storage()),
        }
    }
    pub(crate) fn spawn_native_control(
        self: &Arc<Self>,
        worker: impl Future<Output = ()> + Send + 'static,
    ) -> Result<()> {
        let mut work = self.work.lock().map_err(|_| Error::storage())?;
        while let Some(result) = work.try_join_next() {
            if result.is_err() {
                self.retirement_failed.store(true, Ordering::Release);
            }
        }
        if self.stop.is_cancelled()
            || work.len() >= self.media.store.limits().connections.saturating_mul(2)
        {
            return Err(Error::new("busy", "Native control workers are at capacity"));
        }
        let runtime = self.clone();
        work.spawn(async move {
            if let Err(error) = drive(runtime.clone(), Box::pin(worker)).await {
                runtime.retirement_failed.store(true, Ordering::Release);
                tracing::error!(code=%error.code, "Original native control execution did not settle");
            }
        });
        Ok(())
    }
    pub(crate) fn reap_control_polls(&self) {
        if let Ok(mut jobs) = self.control_polls.jobs.try_lock() {
            self.control_polls.reap(&mut jobs);
        }
        // The periodic custodian must also join completed outer controls.
        // Reaping only at the next submission retains the final batch forever
        // in an otherwise idle runtime. Pending controls keep their original
        // custody; neither observation nor cleanup may cancel or replace them.
        if let Ok(mut work) = self.work.try_lock() {
            while let Some(result) = work.try_join_next() {
                if result.is_err() {
                    self.retirement_failed.store(true, Ordering::Release);
                }
            }
        }
        if self.control_polls.failed.load(Ordering::Acquire) {
            self.retirement_failed.store(true, Ordering::Release);
        }
    }
    /// Call after original async control owners have joined or been cancelled.
    /// A timeout is a failed drain, not permission to abandon a running poll.
    pub(crate) async fn drain_control_polls(&self, deadline: tokio::time::Instant) -> bool {
        let mut jobs = {
            let mut jobs = self.control_polls.jobs.lock().unwrap_or_else(|p| {
                self.control_polls.failed.store(true, Ordering::Release);
                p.into_inner()
            });
            jobs.closed = true;
            std::mem::take(&mut jobs.active)
        };
        let mut normal = !self.control_polls.failed.load(Ordering::Acquire);
        while let Some((_, mut job)) = jobs.pop_first() {
            if job.waiting {
                normal = false;
            }
            match tokio::time::timeout_at(deadline, &mut job.handle).await {
                Ok(result) => normal &= result.is_ok(),
                Err(_) => {
                    normal = false;
                    // Queued polls may be cancelled. A running poll must
                    // release its original Store/authority custody and join.
                    job.handle.abort();
                    let _ = job.handle.await;
                }
            }
        }
        normal
    }
}
#[cfg(all(test, feature = "test-support"))]
mod tests;
