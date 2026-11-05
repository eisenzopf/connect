//! Owned bounded pause of an original already-committed replacement worker.
//! Test support only: no fake media, domain reader, writer, or completion.
use super::*;
use tokio_util::sync::CancellationToken;
pub(crate) struct Pause {
    entered: CancellationToken,
    release: CancellationToken,
    left: CancellationToken,
}
pub struct ReplacementProjectionPause {
    pause: Arc<Pause>,
}
impl ReplacementProjectionPause {
    pub async fn entered(&self) {
        self.pause.entered.cancelled().await;
    }
    pub fn release(&self) {
        self.pause.release.cancel();
    }
    pub async fn left(&self) {
        self.pause.left.cancelled().await;
    }
}
impl Drop for ReplacementProjectionPause {
    fn drop(&mut self) {
        self.pause.release.cancel();
    }
}
struct Leaving(Arc<Pause>);
impl Drop for Leaving {
    fn drop(&mut self) {
        self.0.left.cancel();
    }
}
impl Policy {
    /// Observe the actual installed pending policy at the paused boundary.
    /// This performs no packet IO and returns no authority capability. The
    /// fixture independently checks actual Core ownership, then exercises both
    /// packet directions after observing this fresh domain refusal.
    pub(crate) fn replacement_continuation_refusal_for_test(
        &self,
        record: &str,
    ) -> connect_protocol::Result<String> {
        let target = self
            .replacement_targets
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .filter_map(Weak::upgrade)
            .find(|target| target.pending.intent().operation_id == record)
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let continuation = target
            .public_continuation
            .get()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        if continuation.ordinary()
            || target
                .root
                .upgrade()
                .is_none_or(|root| !root.handle.is_committed())
        {
            return Err(connect_protocol::Error::forbidden());
        }
        match self.store.try_native_replacement_continuation_media_read(
            continuation.metadata.origin(),
            Some(&continuation.metadata),
            None,
            None,
        ) {
            Err(error) => Ok(error.code),
            Ok(_) => Err(connect_protocol::Error::new(
                "fixture-expected-refusal",
                "Actual pending continuation unexpectedly admitted this context",
            )),
        }
    }

    pub(crate) fn replacement_weak_owners_for_test(
        &self,
        record: &str,
    ) -> connect_protocol::Result<Vec<Weak<dyn Any + Send + Sync>>> {
        let target = self
            .replacement_targets
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .filter_map(Weak::upgrade)
            .find(|target| target.pending.intent().operation_id == record)
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let root = target
            .root
            .upgrade()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let mut owners: Vec<Arc<dyn Any + Send + Sync>> = vec![target.clone(), root];
        let continuation = target
            .public_continuation
            .get()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        owners.push(continuation.clone());
        owners.push(continuation.entry.clone());
        Ok(owners.iter().map(Arc::downgrade).collect())
    }
    pub(crate) fn pause_replacement_projection_for_test(
        &self,
        record: &str,
    ) -> connect_protocol::Result<ReplacementProjectionPause> {
        let mut pauses = self
            .replacement_projection_pauses
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        if pauses.len() >= self.capacity || pauses.contains_key(record) {
            return Err(connect_protocol::Error::forbidden());
        }
        let pause = Arc::new(Pause {
            entered: CancellationToken::new(),
            release: CancellationToken::new(),
            left: CancellationToken::new(),
        });
        pauses.insert(record.into(), pause.clone());
        Ok(ReplacementProjectionPause { pause })
    }
    pub(crate) async fn await_replacement_projection_for_test(
        &self,
        record: &str,
        deadline: tokio::time::Instant,
        stop: &CancellationToken,
    ) -> connect_protocol::Result<()> {
        let pause = self
            .replacement_projection_pauses
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .remove(record);
        if let Some(pause) = pause {
            let _leaving = Leaving(pause.clone());
            pause.entered.cancel();
            tokio::select! { biased;
                _=stop.cancelled()=>(),
                _=tokio::time::sleep_until(deadline)=>(),
                _=pause.release.cancelled()=>(),
            }
            // This pause must not discard the already-observed movement.
            // Cancellation/expiry release it into the original capture/cleanup.
            Ok(())
        } else {
            Ok(())
        }
    }
}
