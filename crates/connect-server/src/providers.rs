//! Owned provider worker. One request per job, serialized chat context.
use connect_connectors::{Connector, Telnyx, Vapi};
use connect_service::Service;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub struct Worker {
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Worker {
    pub fn start(service: Arc<Service>) -> anyhow::Result<Self> {
        let mut drivers: Vec<Arc<dyn Connector>> = Vec::new();
        if std::env::var_os("TELNYX_API_KEY").is_some() {
            drivers.push(Arc::new(Telnyx::from_env()?));
        }
        if std::env::var_os("VAPI_PRIVATE_KEY").is_some() {
            drivers.push(Arc::new(Vapi::from_env()?));
        }
        anyhow::ensure!(
            !drivers.is_empty(),
            "Provider worker requires TELNYX_API_KEY or VAPI_PRIVATE_KEY"
        );
        Self::with_drivers(service, drivers)
    }
    /// Public adapter boundary also used by deterministic integration fixtures.
    pub fn with_drivers(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
    ) -> anyhow::Result<Self> {
        service.store.recover_provider_jobs()?;
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let task = tokio::spawn(async move {
            loop {
                if cancelled.is_cancelled() {
                    break;
                }
                let store = service.store.clone();
                let job = tokio::task::spawn_blocking(move || store.claim_provider_job()).await;
                match job {
                    Ok(Ok(Some(job))) => {
                        let result = if let Some(driver) =
                            drivers.iter().find(|d| d.name() == job.route.connector)
                        {
                            tokio::select! {_ = cancelled.cancelled()=>Err(connect_connectors::Failure::Uncertain), r=tokio::time::timeout(Duration::from_secs(35),driver.deliver(&job))=>r.unwrap_or(Err(connect_connectors::Failure::Uncertain))}
                        } else {
                            Err(connect_connectors::Failure::Rejected)
                        };
                        let store = service.store.clone();
                        let saved = tokio::task::spawn_blocking(move || match result {
                            Ok(done) => store.finish_provider_job(
                                &job,
                                if done.reply.is_some() {
                                    "completed"
                                } else {
                                    "accepted"
                                },
                                Some(&done.provider_id),
                                done.reply.as_deref(),
                            ),
                            Err(failure) => {
                                store.finish_provider_job(&job, failure.state(), None, None)
                            }
                        })
                        .await;
                        if !matches!(saved, Ok(Ok(()))) {
                            tracing::warn!(
                                "Provider result not committed; stopping worker with uncertain job"
                            );
                            break;
                        }
                        service.notify_changed();
                    }
                    Ok(Ok(None)) => {
                        tokio::select! {_ = cancelled.cancelled()=>break,_ = tokio::time::sleep(Duration::from_millis(100))=>{}}
                    }
                    _ => {
                        tracing::warn!("Provider storage unavailable; stopping worker");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            stop,
            task: Some(task),
        })
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
                anyhow::bail!(
                    "Provider worker shutdown timed out; interrupted jobs require recovery"
                );
            }
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
