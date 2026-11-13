//! Test-only real owner pause, exact cold unary slot, unchanged packet result.
//! No fabricated grant/Busy, SQL during a map lock, or policy-result substitution.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteEnrollmentBoundary {
    Pending,
    Ordinary,
}
#[derive(Clone, Copy, Debug)]
pub struct RouteEnrollmentMark {
    pub stream_local_id: u16,
    pub sequence: u32,
    pub ssrc: u32,
    pub timestamp: u32,
}
pub(super) struct State {
    pub(super) boundary: RouteEnrollmentBoundary,
    entered: CancellationToken,
    release: CancellationToken,
    left: CancellationToken,
    armed: AtomicBool,
    observed: CancellationToken,
    first: Mutex<Option<String>>,
    lazy_misses: AtomicUsize,
    expected: Mutex<Option<RouteEnrollmentMark>>,
    owner: Mutex<Option<rvoip_core::staged_streams::original_ingress::native_observation::Owner>>,
    generation: AtomicU64,
    candidate: Mutex<Option<String>>,
    worker: Mutex<Option<connect_store::MediaAuthorizationWatcherPause>>,
}
pub struct RouteEnrollmentPause {
    state: Arc<State>,
}
impl RouteEnrollmentPause {
    pub async fn entered(&self) {
        self.state.entered.cancelled().await;
    }
    pub fn arm_first_original_publish(&self, expected: RouteEnrollmentMark) {
        *self.state.first.lock().unwrap() = None;
        self.state.lazy_misses.store(0, Ordering::Release);
        *self.state.expected.lock().unwrap() = Some(expected);
        self.state.armed.store(true, Ordering::Release);
    }
    pub async fn first_original_publish(&self) -> (String, usize) {
        self.state.observed.cancelled().await;
        (
            self.state
                .first
                .lock()
                .unwrap()
                .clone()
                .expect("actual packet result"),
            self.state.lazy_misses.load(Ordering::Acquire),
        )
    }
    pub fn release(&self) {
        self.state.release.cancel();
    }
    pub fn watcher_is_paused(&self) -> bool {
        self.state.worker.lock().ok().is_some_and(|worker| {
            worker
                .as_ref()
                .is_some_and(connect_store::MediaAuthorizationWatcherPause::is_paused)
        })
    }
    pub async fn left(&self) {
        self.state.left.cancelled().await;
    }
}
impl Drop for RouteEnrollmentPause {
    fn drop(&mut self) {
        self.state.release.cancel();
        if let Ok(worker) = self.state.worker.lock() {
            if let Some(worker) = worker.as_ref() {
                worker.release();
            }
        }
    }
}
pub(crate) struct ActivePause {
    state: Arc<State>,
    candidate: String,
}
// Closed actual fixture inputs only. The arbitrary synchronous lookup callback
// is consumed before this value can enter any async state machine.
pub(crate) struct PreparedPause {
    state: Arc<State>,
    current: Arc<public_continuation::Continuation>,
    owner: rvoip_core::staged_streams::original_ingress::native_observation::Owner,
}
impl Policy {
    pub(crate) fn pause_route_enrollment_for_test(
        &self,
        record: &str,
        boundary: RouteEnrollmentBoundary,
    ) -> connect_protocol::Result<RouteEnrollmentPause> {
        let mut pauses = self
            .route_enrollment_pauses
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        let key = (record.to_owned(), boundary);
        if pauses.len() >= self.capacity || pauses.contains_key(&key) {
            return Err(connect_protocol::Error::forbidden());
        }
        let state = Arc::new(State {
            boundary,
            entered: CancellationToken::new(),
            release: CancellationToken::new(),
            left: CancellationToken::new(),
            armed: AtomicBool::new(false),
            observed: CancellationToken::new(),
            first: Mutex::new(None),
            lazy_misses: AtomicUsize::new(0),
            expected: Mutex::new(None),
            owner: Mutex::new(None),
            generation: AtomicU64::new(0),
            candidate: Mutex::new(None),
            worker: Mutex::new(None),
        });
        pauses.insert(key, state.clone());
        Ok(RouteEnrollmentPause { state })
    }
    pub(crate) fn prepare_route_enrollment_fixture(
        &self,
        record: &str,
        boundary: RouteEnrollmentBoundary,
        original: impl FnOnce() -> connect_protocol::Result<(
            Arc<public_continuation::Continuation>,
            rvoip_core::staged_streams::original_ingress::native_observation::Owner,
        )>,
    ) -> connect_protocol::Result<Option<PreparedPause>> {
        let state = self
            .route_enrollment_pauses
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .remove(&(record.to_owned(), boundary));
        let Some(state) = state else {
            return Ok(None);
        };
        // Only an explicitly armed fixture may inspect an original controller.
        // An unrelated retirement can remove it after known movement; the
        // unarmed diagnostic must not interrupt that effect's durable recording.
        // The pause-map guard is already gone before this fallible lookup.
        let (current, owner) = original()?;
        if current.metadata.origin().offer_record_id() != record
            || current.ordinary() != (boundary == RouteEnrollmentBoundary::Ordinary)
        {
            return Err(connect_protocol::Error::forbidden());
        }
        Ok(Some(PreparedPause {
            state,
            current,
            owner,
        }))
    }
    pub(crate) async fn begin_route_enrollment_fixture(
        &self,
        prepared: Option<PreparedPause>,
    ) -> connect_protocol::Result<Option<ActivePause>> {
        let Some(PreparedPause {
            state,
            current,
            owner,
        }) = prepared
        else {
            return Ok(None);
        };
        let boundary = state.boundary;
        let store = self.store.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let pause = store.pause_media_authorization_watcher_for_test()?;
            pause.wait_entered()?;
            Ok::<_, connect_protocol::Error>(pause)
        })
        .await
        .map_err(|_| connect_protocol::Error::storage())??;
        *state
            .worker
            .lock()
            .map_err(|_| connect_protocol::Error::storage())? = Some(worker);
        // Explicit cold-cache fixture: withdraw ONE actual unary registration,
        // never actor authority or any unrelated peer's route. Existing Handle
        // destruction does real epoch invalidation; no refusal is manufactured.
        current.entry.withdraw_unary_route_for_test(
            (boundary == RouteEnrollmentBoundary::Pending).then_some(&current),
        )?;
        let candidate = current.metadata.origin().runtime_connection_id().to_owned();
        *state
            .candidate
            .lock()
            .map_err(|_| connect_protocol::Error::storage())? = Some(candidate.clone());
        *state
            .owner
            .lock()
            .map_err(|_| connect_protocol::Error::storage())? = Some(owner);
        state
            .generation
            .store(current.metadata.origin().generation(), Ordering::Release);
        self.route_enrollment_observations
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .insert(candidate.clone(), state.clone());
        Ok(Some(ActivePause { state, candidate }))
    }
    pub(crate) async fn finish_route_enrollment_fixture(
        &self,
        active: Option<ActivePause>,
        until: tokio::time::Instant,
        stop: &CancellationToken,
    ) -> connect_protocol::Result<()> {
        let Some(active) = active else {
            return Ok(());
        };
        active.state.entered.cancel();
        tokio::select! { biased;
            _ = stop.cancelled() => {},
            _ = tokio::time::sleep_until(until) => {},
            _ = active.state.release.cancelled() => {},
        }
        self.route_enrollment_observations
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .remove(&active.candidate);
        let worker = active
            .state
            .worker
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .take();
        if let Some(worker) = worker {
            tokio::task::spawn_blocking(move || {
                worker.release();
                worker.wait_left()
            })
            .await
            .map_err(|_| connect_protocol::Error::storage())??;
        }
        active.state.left.cancel();
        Ok(())
    }
    pub(super) fn observe_route_original_publish_for_test(
        &self,
        scope: &MediaScope<'_>,
        original: bool,
        result: &ControlResult<Box<dyn MediaAuthorityLease>>,
    ) {
        if !original || !matches!(scope.purpose, MediaPurpose::Publish) {
            return;
        }
        let state = self
            .route_enrollment_observations
            .try_lock()
            .ok()
            .and_then(|values| values.get(scope.connection.as_str()).cloned());
        let Some(state) = state else {
            return;
        };
        if !state.matches_current_native_frame(scope.connection) {
            return;
        }
        if !state.armed.swap(false, Ordering::AcqRel) {
            return;
        }
        if let Ok(mut first) = state.first.try_lock() {
            *first = Some(match result {
                Ok(_) => "admitted".into(),
                Err(error) => format!("{error:?}"),
            });
            state.observed.cancel();
        };
    }
    pub(super) fn observe_cold_unary_miss_for_test(&self, runtime: &str) {
        let state = self
            .route_enrollment_observations
            .try_lock()
            .ok()
            .and_then(|values| values.get(runtime).cloned());
        if let Some(state) = state {
            if state.armed.load(Ordering::Acquire)
                && state.matches_current_native_frame(&ConnectionId::from_string(runtime))
            {
                state.lazy_misses.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}
impl State {
    fn matches_current_native_frame(&self, connection: &ConnectionId) -> bool {
        let Some(frame) =
            rvoip_core::staged_streams::original_ingress::native_observation::current()
        else {
            return false;
        };
        let Ok(expected) = self.expected.try_lock() else {
            return false;
        };
        let Some(expected) = *expected else {
            return false;
        };
        let Ok(owner) = self.owner.try_lock() else {
            return false;
        };
        let Some(owner) = owner.as_ref() else {
            return false;
        };
        frame.connection_generation == self.generation.load(Ordering::Acquire)
            && frame.matches(
                owner,
                connection,
                expected.stream_local_id,
                expected.sequence,
                expected.ssrc,
                expected.timestamp,
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unarmed_route_hooks_never_resolve_missing_original_controller_at_either_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap(),
        );
        let policy = Policy::new(store, 8);
        let invoked = AtomicUsize::new(0);
        for boundary in [
            RouteEnrollmentBoundary::Pending,
            RouteEnrollmentBoundary::Ordinary,
        ] {
            let result = policy
                .prepare_route_enrollment_fixture("unarmed-known-movement", boundary, || {
                    invoked.fetch_add(1, Ordering::AcqRel);
                    Err(connect_protocol::Error::new(
                        "forbidden",
                        "original controller already retired",
                    ))
                })
                .unwrap();
            assert!(result.is_none());
        }
        assert_eq!(
            invoked.load(Ordering::Acquire),
            0,
            "unarmed diagnostics evaluated a fallible retired-controller lookup"
        );
        assert!(policy
            .route_enrollment_observations
            .lock()
            .unwrap()
            .is_empty());
        assert!(policy.route_enrollment_pauses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn explicitly_armed_route_hook_still_requires_original_owner_before_starting_watcher_pause(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap(),
        );
        let policy = Policy::new(store, 8);
        let invoked = AtomicUsize::new(0);
        for boundary in [
            RouteEnrollmentBoundary::Pending,
            RouteEnrollmentBoundary::Ordinary,
        ] {
            let pause = policy
                .pause_route_enrollment_for_test("armed-missing-controller", boundary)
                .unwrap();
            let error = match policy.prepare_route_enrollment_fixture(
                "armed-missing-controller",
                boundary,
                || {
                    // SAME actual pause map is released before any owner lookup.
                    assert!(policy.route_enrollment_pauses.try_lock().is_ok());
                    invoked.fetch_add(1, Ordering::AcqRel);
                    Err(connect_protocol::Error::new(
                        "forbidden",
                        "original controller already retired",
                    ))
                },
            ) {
                Err(error) => error,
                Ok(_) => panic!("explicit fixture accepted a missing original owner"),
            };
            assert_eq!(error.code, "forbidden");
            assert_eq!(error.message, "original controller already retired");
            assert!(!pause.state.entered.is_cancelled());
            assert!(pause.state.worker.lock().unwrap().is_none());
            drop(pause);
        }
        assert_eq!(invoked.load(Ordering::Acquire), 2);
        assert!(policy
            .route_enrollment_observations
            .lock()
            .unwrap()
            .is_empty());
        assert!(policy.route_enrollment_pauses.lock().unwrap().is_empty());
    }
}
