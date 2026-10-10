//! Owned provider worker. One request per job, serialized chat context.
use connect_connector_api::DeliveryRequest;
use connect_connectors::{Connector, Failure, Telnyx, Vapi};
use connect_service::Service;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub struct Worker {
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
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
        service.store.expire_sms_authorities()?;
        let authority = crate::sms_authority::Authority::from_env()?;
        Self::with_authority(service, drivers, authority)
    }
    /// Public adapter boundary also used by deterministic integration fixtures.
    pub fn with_drivers(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
    ) -> anyhow::Result<Self> {
        Self::with_authority(service, drivers, None)
    }
    fn with_authority(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
        authority: Option<crate::sms_authority::Authority>,
    ) -> anyhow::Result<Self> {
        service.store.recover_provider_jobs()?;
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let task = tokio::spawn(async move {
            let mut next_refresh = tokio::time::Instant::now();
            loop {
                if cancelled.is_cancelled() {
                    break;
                }
                if let Some(authority) = &authority {
                    if tokio::time::Instant::now() >= next_refresh {
                        let result = tokio::select! {
                            _ = cancelled.cancelled() => break,
                            value = authority.refresh(service.store.clone()) => value,
                        };
                        if result.is_err() {
                            tracing::warn!(
                                "SMS enrollment refresh failed; stale grants expire automatically"
                            );
                        }
                        next_refresh = tokio::time::Instant::now() + Duration::from_secs(30);
                    }
                }
                let store = service.store.clone();
                let job = tokio::task::spawn_blocking(move || store.claim_provider_job()).await;
                match job {
                    Ok(Ok(Some(job))) => {
                        let request = DeliveryRequest {
                            target: job.route.target.clone(),
                            source: job.route.source.clone(),
                            body: job.body.clone(),
                            previous_chat_id: job.previous_chat_id.clone(),
                            context: job.context.clone(),
                        };
                        let result = if let Some(driver) =
                            drivers.iter().find(|d| d.name() == job.route.connector)
                        {
                            tokio::select! {
                                _ = cancelled.cancelled() => Err(Failure::Uncertain),
                                result = tokio::time::timeout(Duration::from_secs(35), driver.deliver(&request)) => {
                                    result.unwrap_or(Err(Failure::Uncertain))
                                },
                            }
                        } else {
                            Err(Failure::Rejected)
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
                            anyhow::bail!("Provider result not committed; recover uncertain work");
                        }
                        service.notify_changed();
                    }
                    Ok(Ok(None)) => {
                        tokio::select! {
                            _ = cancelled.cancelled() => break,
                            _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                        }
                    }
                    _ => {
                        tracing::warn!("Provider storage unavailable; stopping worker");
                        anyhow::bail!("Provider storage unavailable");
                    }
                }
            }
            Ok(())
        });
        Ok(Self {
            stop,
            task: Some(task),
        })
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
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
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
