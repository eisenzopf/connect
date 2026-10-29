//! Local ordinary-offer custody. Completion means the worker's cleanup returned.
//! Closing a lane prevents later starts while an exact candidate is retired.
use connect_protocol::{Error, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use tokio::{sync::Notify, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub(crate) struct Workers(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    next: u64,
    lanes: BTreeMap<String, Lane>,
}
struct Lane {
    cancellation: CancellationToken,
    active: BTreeSet<u64>,
    closed: bool,
    failed: bool,
    changed: Arc<Notify>,
}
pub(crate) struct Registration {
    workers: Workers,
    connection: String,
    invocation: u64,
    cancellation: CancellationToken,
    completed: bool,
}
/// Kept closed on drop or expiry. Only attributed metadata settlement may release it.
pub(crate) struct Retirement {
    workers: Workers,
    connection: String,
    changed: Arc<Notify>,
}
fn uncertain() -> Error {
    Error::new(
        "reconciliation-required",
        "Original native worker cleanup is not joined",
    )
}
impl Workers {
    /// Register before Store can mark an ordinary native effect started.
    /// A closed lane still permits the caller's separate metadata-only replay.
    pub(crate) fn register(
        &self,
        connection: &str,
        limit: usize,
        stop: &CancellationToken,
    ) -> Result<Option<Registration>> {
        let mut state = self.0.lock().map_err(|_| Error::storage())?;
        if state.lanes.get(connection).is_some_and(|lane| lane.closed) {
            return Ok(None);
        }
        if !state.lanes.contains_key(connection) && state.lanes.len() >= limit {
            return Err(Error::new("busy", "Native offer custody is at capacity"));
        }
        state.next = state.next.checked_add(1).ok_or_else(Error::storage)?;
        let invocation = state.next;
        let lane = state
            .lanes
            .entry(connection.into())
            .or_insert_with(|| Lane {
                cancellation: stop.child_token(),
                active: Default::default(),
                closed: false,
                failed: false,
                changed: Arc::new(Notify::new()),
            });
        lane.active.insert(invocation);
        Ok(Some(Registration {
            workers: self.clone(),
            connection: connection.into(),
            invocation,
            cancellation: lane.cancellation.clone(),
            completed: false,
        }))
    }
    /// Cancel and join all already registered invocations, including one that
    /// has not yet reached its first Store/Core call. No task is aborted/detached.
    pub(crate) async fn retire(
        &self,
        connection: &str,
        limit: usize,
        stop: &CancellationToken,
        deadline: Instant,
    ) -> Result<Retirement> {
        let changed = {
            let mut state = self.0.lock().map_err(|_| Error::storage())?;
            if !state.lanes.contains_key(connection) && state.lanes.len() >= limit {
                return Err(Error::new("busy", "Native offer custody is at capacity"));
            }
            let lane = state
                .lanes
                .entry(connection.into())
                .or_insert_with(|| Lane {
                    cancellation: stop.child_token(),
                    active: Default::default(),
                    closed: true,
                    failed: false,
                    changed: Arc::new(Notify::new()),
                });
            lane.closed = true;
            lane.cancellation.cancel();
            lane.changed.clone()
        };
        loop {
            // Register the notification before observing the last worker, so a
            // completion between that observation and await cannot be lost.
            let notified = changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.0.lock().map_err(|_| Error::storage())?;
                let lane = state.lanes.get(connection).ok_or_else(uncertain)?;
                if !Arc::ptr_eq(&lane.changed, &changed) || lane.failed {
                    return Err(uncertain());
                }
                if stop.is_cancelled() {
                    return Err(Error::forbidden());
                }
                if lane.active.is_empty() {
                    return Ok(Retirement {
                        workers: self.clone(),
                        connection: connection.into(),
                        changed: changed.clone(),
                    });
                }
            }
            tokio::select! {
                biased;
                _ = stop.cancelled() => return Err(Error::forbidden()),
                _ = tokio::time::sleep_until(deadline) => return Err(uncertain()),
                _ = &mut notified => {},
            }
        }
    }
}
impl Registration {
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
    /// Call only after the entire original worker, including cleanup, returns.
    pub(crate) fn complete(mut self) {
        self.completed = true;
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut state = self
            .workers
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let Some(lane) = state.lanes.get_mut(&self.connection) else {
            return;
        };
        if !lane.active.remove(&self.invocation) {
            return;
        }
        if !self.completed {
            lane.failed = true;
            lane.closed = true;
            lane.cancellation.cancel();
        }
        lane.changed.notify_waiters();
        if lane.active.is_empty() && !lane.closed && !lane.failed {
            state.lanes.remove(&self.connection);
        }
    }
}
impl Retirement {
    /// Release only after the Store records the original route's attributed
    /// retirement. Dropping this value leaves the ordinary-offer lane closed.
    pub(crate) fn release(self) -> Result<()> {
        let mut state = self.workers.0.lock().map_err(|_| Error::storage())?;
        let lane = state.lanes.get(&self.connection).ok_or_else(uncertain)?;
        if !Arc::ptr_eq(&lane.changed, &self.changed)
            || !lane.closed
            || lane.failed
            || !lane.active.is_empty()
        {
            return Err(uncertain());
        }
        state.lanes.remove(&self.connection);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[tokio::test]
    async fn retirement_waits_for_every_original_cleanup_and_preserves_other_lanes() {
        let workers = Workers::default();
        let stop = CancellationToken::new();
        let one = workers.register("candidate", 3, &stop).unwrap().unwrap();
        let two = workers.register("candidate", 3, &stop).unwrap().unwrap();
        let other = workers.register("unrelated", 3, &stop).unwrap().unwrap();
        let cancelled = one.cancellation();
        let pending = tokio::spawn({
            let workers = workers.clone();
            let stop = stop.clone();
            async move {
                workers
                    .retire(
                        "candidate",
                        3,
                        &stop,
                        Instant::now() + Duration::from_secs(2),
                    )
                    .await
            }
        });
        cancelled.cancelled().await;
        assert!(two.cancellation().is_cancelled());
        assert!(!other.cancellation().is_cancelled());
        assert!(workers.register("candidate", 3, &stop).unwrap().is_none());
        one.complete();
        assert!(!pending.is_finished());
        two.complete();
        let retired = pending.await.unwrap().unwrap();
        assert!(workers.register("candidate", 3, &stop).unwrap().is_none());
        retired.release().unwrap();
        workers
            .register("candidate", 3, &stop)
            .unwrap()
            .unwrap()
            .complete();
        other.complete();
        assert!(workers.0.lock().unwrap().lanes.is_empty());
    }
    #[tokio::test]
    async fn interrupted_worker_never_counts_as_successful_retirement() {
        let workers = Workers::default();
        let stop = CancellationToken::new();
        let original = workers.register("candidate", 1, &stop).unwrap().unwrap();
        drop(original);
        assert!(workers
            .retire("candidate", 1, &stop, Instant::now())
            .await
            .is_err());
        assert!(workers.register("candidate", 1, &stop).unwrap().is_none());
    }
    #[tokio::test]
    async fn deadline_keeps_original_custody_and_closed_lane_until_join_and_settlement() {
        let workers = Workers::default();
        let stop = CancellationToken::new();
        let original = workers.register("candidate", 1, &stop).unwrap().unwrap();
        assert!(workers
            .retire("candidate", 1, &stop, Instant::now())
            .await
            .is_err());
        assert!(original.cancellation().is_cancelled());
        assert!(workers.register("candidate", 1, &stop).unwrap().is_none());
        original.complete();
        let retired = workers
            .retire("candidate", 1, &stop, Instant::now())
            .await
            .unwrap();
        drop(retired);
        assert!(workers.register("candidate", 1, &stop).unwrap().is_none());
        workers
            .retire("candidate", 1, &stop, Instant::now())
            .await
            .unwrap()
            .release()
            .unwrap();
        workers
            .register("candidate", 1, &stop)
            .unwrap()
            .unwrap()
            .complete();
    }
}
