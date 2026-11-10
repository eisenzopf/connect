//! One original codec reservation, retained independently of its async waiter.
use connect_protocol::{Error, Result};
use rvoip_uctp::v2::auth_dispatch::DispatchError;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

struct State<T> {
    claimed: bool,
    pending: bool,
    outcome: Option<Result<T>>,
}
pub(super) struct Slot<T> {
    state: Mutex<State<T>>,
    completed: Notify,
}
impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                claimed: false,
                pending: false,
                outcome: None,
            }),
            completed: Notify::new(),
        }
    }
}
impl<T> Slot<T> {
    pub(super) fn claim(self: &Arc<Self>) -> std::result::Result<Pending<T>, DispatchError> {
        let mut state = self.state.try_lock().map_err(|_| DispatchError::Capacity)?;
        if state.claimed {
            return Err(DispatchError::Authority);
        }
        state.claimed = true;
        state.pending = true;
        Ok(Pending {
            slot: self.clone(),
            finished: false,
        })
    }
    pub(super) fn with_ready<R>(
        &self,
        f: impl FnOnce(&T) -> std::result::Result<R, DispatchError>,
    ) -> std::result::Result<R, DispatchError> {
        let state = self.state.try_lock().map_err(|_| DispatchError::Capacity)?;
        let value = state
            .outcome
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .ok_or(DispatchError::Authority)?;
        f(value)
    }
    pub(super) async fn take_when_done(&self) -> Option<T> {
        loop {
            // notify_one retains a completion racing the subsequent await.
            let ready = self.completed.notified();
            {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                if !state.pending {
                    return state.outcome.take().and_then(Result::ok);
                }
            }
            ready.await;
        }
    }
}
pub(super) struct Pending<T> {
    slot: Arc<Slot<T>>,
    finished: bool,
}
impl<T> Pending<T> {
    pub(super) fn run(mut self, call: impl FnOnce() -> Result<T>) -> Result<()> {
        // No slot mutex is held through SQL admission/file waits.
        let outcome = call();
        let result = outcome.as_ref().map(|_| ()).map_err(Clone::clone);
        {
            let mut state = self.slot.state.lock().unwrap_or_else(|p| p.into_inner());
            state.outcome = Some(outcome);
            state.pending = false;
        }
        self.finished = true;
        self.slot.completed.notify_one();
        result
    }
}
impl<T> Drop for Pending<T> {
    fn drop(&mut self) {
        if !self.finished {
            let mut state = self.slot.state.lock().unwrap_or_else(|p| p.into_inner());
            state.outcome = Some(Err(Error::storage()));
            state.pending = false;
            self.slot.completed.notify_one();
        }
    }
}
#[cfg(all(test, feature = "test-support"))]
mod tests;
