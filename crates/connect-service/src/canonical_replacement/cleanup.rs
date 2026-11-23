//! Historical uncertain-Start settlement precedes the separate public close.
//! Neither observation grants Closed, Discarded, publication or OLD authority.
use super::*;
use std::{future::Future, sync::Mutex};

// Both futures retain their original owners. The public cleanup future is not
// polled unless exact historical settlement succeeds; no effect is replayed.
pub(super) async fn before_public_abort(
    settlement: impl Future<Output = Result<()>>,
    abort: impl Future<Output = Result<()>>,
) -> Result<()> {
    settlement.await?;
    abort.await
}

// Retain the original started-first priority. Holding its local slot through
// this clone prevents observing a successful Started and stale failed capture
// as two independent choices. No mutex/opaque owner crosses an await.
fn uncertain_start_only<S, U>(
    started: &Mutex<Option<S>>,
    failed: &Mutex<Option<Arc<U>>>,
) -> Result<Option<Arc<U>>> {
    let started = started.lock().map_err(|_| Error::storage())?;
    if started.is_some() {
        return Ok(None);
    }
    let failed = failed.lock().map_err(|_| Error::storage())?.clone();
    Ok(failed)
}

impl Runtime {
    pub(super) async fn settle_original_uncertain_start_before_public_abort(
        &self,
        saved: &Arc<PreparedReplacement>,
    ) -> Result<()> {
        if saved.handle.is_committed() {
            return Err(Error::forbidden());
        }
        // Root ownership is cloned and its registry mutex released BEFORE any
        // child slot is inspected, matching the original later cleanup path.
        let allocated = saved
            .allocated
            .lock()
            .map_err(|_| Error::storage())?
            .clone();
        let uncertain = match allocated {
            Some(allocated) => uncertain_start_only(&allocated.started, &allocated.failed_start)?,
            None => None,
        };
        let Some(uncertain) = uncertain else {
            return Ok(());
        };
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let uncertain = uncertain.clone();
            store_call(move || store.native_replacement_start_uncertain(&uncertain))
        })
        .await?;
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let uncertain = uncertain.clone();
            let absent = crate::canonical_native::Absent {
                natives: self.media_adapters(),
                id: saved.handle.intent().replacement.clone(),
                generation: saved.handle.prepared_binding().generation(),
            };
            store_call(move || {
                store
                    .record_native_replacement_uncertain_start_stopped(&uncertain, &absent)
                    .map(|_| ())
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests;
