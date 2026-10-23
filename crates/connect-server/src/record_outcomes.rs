//! Bounded supervised finalization. The Store terminal intent is authoritative;
//! this worker can stop/restart without inventing a new document or losing it.
use connect_service::Service;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
pub(crate) struct Worker {
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}
impl Worker {
    pub(crate) fn start(service: Arc<Service>) -> Self {
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let task = crate::worker::spawn(
            service.readiness.clone(),
            "session-records",
            stop.clone(),
            async move {
                loop {
                    if cancelled.is_cancelled() {
                        break;
                    }
                    let finalized = service.finalize_record_outcomes(32).await;
                    let expired = service.expire_record_outcomes(32).await;
                    match (finalized, expired) {
                        (Ok(batch), Ok(retention))
                            if batch.failed == 0 && retention.failed == 0 =>
                        {
                            service.readiness.set(
                                "session-records",
                                connect_protocol::ReadinessState::Ready,
                                "record-worker-running",
                            )
                        }
                        _ => service.readiness.set(
                            "session-records",
                            connect_protocol::ReadinessState::Degraded,
                            "record-store-unconfirmed",
                        ),
                    }
                    tokio::select! {biased;_=cancelled.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
                }
                Ok(())
            },
        );
        Self {
            stop,
            task: Some(task),
        }
    }
    pub(crate) async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        crate::worker::drain(
            self.task.take().into_iter().collect(),
            Duration::from_secs(5),
        )
        .await
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
