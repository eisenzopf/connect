//! One CONTROL admission budget and owner, never a retry of an effect.
use super::*;
use connect_store::{AuthorityWait, Store};
use std::{
    sync::atomic::{AtomicI64, Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(super) struct Admission {
    deadline: tokio::time::Instant,
    until: Arc<AtomicI64>,
    cancelled: CancellationToken,
    wait: AuthorityWait,
}
impl Drop for Admission {
    fn drop(&mut self) {
        self.wait.cancel();
    }
}
impl Admission {
    pub(super) fn new(
        deadline: tokio::time::Instant,
        cancelled: CancellationToken,
        until: i64,
    ) -> Self {
        let deadline = deadline.min(tokio::time::Instant::now() + Duration::from_secs(5));
        Self {
            deadline,
            cancelled,
            until: Arc::new(AtomicI64::new(until)),
            wait: AuthorityWait::new(
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ),
        }
    }
    fn current(&self, runtime: &Runtime) -> bool {
        !runtime.stop.is_cancelled()
            && !self.cancelled.is_cancelled()
            && tokio::time::Instant::now() < self.deadline
            && chrono::Utc::now().timestamp() < self.until.load(Ordering::Acquire)
    }
    /// Read-only errors are retryable only for admission contention. No SQL
    /// statement/storage failure or current-policy denial is reclassified.
    pub(super) async fn read<T: Send + 'static>(
        &self,
        runtime: &Arc<Runtime>,
        call: impl Fn(&Store, &mut i64, &AuthorityWait) -> Result<T> + Send + Sync + 'static,
    ) -> Result<T> {
        let value = self
            .run(runtime, move |store, until, wait| {
                match call(store, until, wait) {
                    Ok(value) => Ok(Some(value)),
                    Err(error) if error.code == "authority-busy" => Ok(None),
                    Err(error) => Err(error),
                }
            })
            .await?;
        if !self.current(runtime) {
            return Err(Error::forbidden());
        }
        Ok(value)
    }
    /// Only typed None permits another pre-DML admission. A late Some still
    /// returns original attempt custody after cancellation/deadline and joining.
    pub(super) async fn run<T: Send + 'static>(
        &self,
        runtime: &Arc<Runtime>,
        call: impl Fn(&Store, &mut i64, &AuthorityWait) -> Result<Option<T>> + Send + Sync + 'static,
    ) -> Result<T> {
        let call = Arc::new(call);
        loop {
            if !self.current(runtime) {
                return Err(Error::forbidden());
            }
            let store = runtime.media.store.clone();
            let call = call.clone();
            let until = self.until.clone();
            let wait = self.wait.clone();
            let stop = runtime.stop.clone();
            let cancel = self.cancelled.clone();
            let work = runtime.owned_store_call(move || {
                if stop.is_cancelled() || cancel.is_cancelled() {
                    return Err(Error::forbidden());
                }
                let mut cap = until.load(Ordering::Acquire);
                if chrono::Utc::now().timestamp() >= cap {
                    return Err(Error::forbidden());
                }
                let result = call(&store, &mut cap, &wait);
                until.fetch_min(cap, Ordering::AcqRel);
                result
            });
            tokio::pin!(work);
            let result = tokio::select! { biased;
                _ = runtime.stop.cancelled() => None,
                _ = self.cancelled.cancelled() => None,
                _ = tokio::time::sleep_until(self.deadline) => None,
                result = &mut work => Some(result),
            };
            let result = match result {
                Some(result) => result,
                None => {
                    self.wait.cancel();
                    work.await
                }
            }?;
            if let Some(value) = result {
                return Ok(value);
            }
            if !self.current(runtime) {
                return Err(Error::forbidden());
            }
            tokio::select! { biased;
                _ = runtime.stop.cancelled() => return Err(Error::forbidden()),
                _ = self.cancelled.cancelled() => return Err(Error::forbidden()),
                _ = tokio::time::sleep_until(self.deadline) => return Err(Error::forbidden()),
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    }
}
