//! TEST SUPPORT ONLY. Owns no registry, Store, SQL connection or authority guard.
use super::*;

pub struct MediaAuthorizationWatcherPause {
    state: Arc<State>,
}
pub(super) struct State {
    entered: AtomicBool,
    released: AtomicBool,
    left: AtomicBool,
    failed: AtomicBool,
    gate: Mutex<()>,
    changed: Condvar,
    until: Instant,
}
impl State {
    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.changed.notify_all();
    }
}
impl MediaAuthorizationWatcherPause {
    pub fn wait_entered(&self) -> Result<()> {
        let mut guard = self.state.gate.lock().map_err(|_| Error::storage())?;
        while !self.state.entered.load(Ordering::Acquire) {
            if self.state.failed.load(Ordering::Acquire) || self.state.left.load(Ordering::Acquire)
            {
                return Err(Error::storage());
            }
            if Instant::now() >= self.state.until {
                return Err(Error::storage());
            }
            guard = self
                .state
                .changed
                .wait_timeout(guard, Duration::from_millis(5))
                .map_err(|_| Error::storage())?
                .0;
        }
        if !self.is_paused() {
            return Err(Error::storage());
        }
        Ok(())
    }
    pub fn release(&self) {
        self.state.release();
    }
    pub fn is_paused(&self) -> bool {
        self.state.entered.load(Ordering::Acquire)
            && !self.state.released.load(Ordering::Acquire)
            && !self.state.left.load(Ordering::Acquire)
            && Instant::now() < self.state.until
    }
    pub fn wait_left(&self) -> Result<()> {
        let mut guard = self.state.gate.lock().map_err(|_| Error::storage())?;
        while !self.state.left.load(Ordering::Acquire) {
            if Instant::now() >= self.state.until {
                return Err(Error::storage());
            }
            guard = self
                .state
                .changed
                .wait_timeout(guard, Duration::from_millis(5))
                .map_err(|_| Error::storage())?
                .0;
        }
        Ok(())
    }
}
impl Drop for MediaAuthorizationWatcherPause {
    fn drop(&mut self) {
        self.state.release();
    }
}
impl Registry {
    pub(crate) fn pause_worker_for_test(&self) -> Result<MediaAuthorizationWatcherPause> {
        let state = Arc::new(State {
            entered: AtomicBool::new(false),
            released: AtomicBool::new(false),
            left: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            gate: Mutex::new(()),
            changed: Condvar::new(),
            until: Instant::now() + Duration::from_secs(10),
        });
        let mut slot = self.worker_pause.lock().map_err(|_| Error::storage())?;
        if self.stopped() || !self.watcher_started.load(Ordering::Acquire) || slot.is_some() {
            return Err(Error::forbidden());
        }
        *slot = Some(state.clone());
        drop(slot);
        self.changed.notify_all();
        Ok(MediaAuthorizationWatcherPause { state })
    }
    pub(super) fn take_worker_pause_for_test(&self) -> Option<Arc<State>> {
        self.worker_pause
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }
    pub(super) fn pause_worker_turn_for_test(
        &self,
        pause: Option<Arc<State>>,
        clear: bool,
        failed: bool,
    ) {
        let Some(state) = pause else {
            return;
        };
        if failed {
            state.failed.store(true, Ordering::Release);
            state.left.store(true, Ordering::Release);
            state.changed.notify_all();
            return;
        }
        if state.released.load(Ordering::Acquire) || self.stopped() || Instant::now() >= state.until
        {
            state.left.store(true, Ordering::Release);
            state.changed.notify_all();
            return;
        }
        if !clear {
            // Deferred preserves old metadata; real Blocked is not Clear.
            // Keep the original request/deadline for another REAL owner probe.
            if state.released.load(Ordering::Acquire)
                || self.stopped()
                || Instant::now() >= state.until
            {
                state.left.store(true, Ordering::Release);
                state.changed.notify_all();
                return;
            }
            if let Ok(mut slot) = self.worker_pause.lock() {
                if slot.is_none() {
                    *slot = Some(state);
                    return;
                }
            }
            state.failed.store(true, Ordering::Release);
            state.left.store(true, Ordering::Release);
            state.changed.notify_all();
            return;
        }
        // Request was captured at this turn's start. Its REAL external Writer
        // probe has now completed, after the request, before any refresh. No
        // Registry/control/map/Core/authority guard survives into this pause.
        state.entered.store(true, Ordering::Release);
        state.changed.notify_all();
        if let Ok(mut guard) = state.gate.lock() {
            while !state.released.load(Ordering::Acquire)
                && !self.stopped()
                && Instant::now() < state.until
            {
                let Ok(next) = state.changed.wait_timeout(guard, Duration::from_millis(5)) else {
                    break;
                };
                guard = next.0;
            }
        }
        state.left.store(true, Ordering::Release);
        state.changed.notify_all();
    }
}
