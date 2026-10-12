//! Observe current authority during activation without resubmitting signaling.
use super::OriginationOutcome;
use connect_store::{SipLegJob, Store};
use std::{future::Future, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Cancellation owns connection cleanup. A failed or stalled cleanup cannot
/// manufacture a cancelled remote outcome. Dropping activation also releases
/// the rvoip preparation's supervisor-owned cleanup path.
pub(super) async fn wait<F, T, E, C, CF>(
    activation: F,
    store: Arc<Store>,
    job: SipLegJob,
    stopping: CancellationToken,
    cleanup: C,
) -> anyhow::Result<OriginationOutcome>
where
    F: Future<Output = Result<T, E>>,
    E: Into<anyhow::Error>,
    C: FnOnce() -> CF,
    CF: Future<Output = rvoip_core::Result<()>>,
{
    tokio::pin!(activation);
    loop {
        let cancel = tokio::select! {
            _ = stopping.cancelled() => true,
            result = &mut activation => {
                result.map_err(Into::into)?;
                return Ok(OriginationOutcome::Active);
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                let store = store.clone();
                let job = job.clone();
                tokio::task::spawn_blocking(move || store.sip_origination_cancelled(&job))
                    .await.map_err(|_| connect_protocol::Error::storage())??
            }
        };
        if cancel {
            return Ok(
                match tokio::time::timeout(CLEANUP_TIMEOUT, cleanup()).await {
                    Ok(Ok(())) => OriginationOutcome::Cancelled,
                    _ => OriginationOutcome::CleanupUncertain,
                },
            );
        }
    }
}
