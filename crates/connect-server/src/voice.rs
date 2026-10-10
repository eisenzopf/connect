//! Lifecycle-owned telephone workers; network effects occur after durable claims.
use connect_connector_api::{VoiceQuery, VoiceStart};
use connect_connectors::{voice::VoiceConnector, Failure, Vapi};
use connect_service::Service;
use connect_store::VoiceAction;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
pub struct Worker {
    stop: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
}
impl Worker {
    pub fn start(service: Arc<Service>) -> anyhow::Result<Self> {
        Self::with_driver(service, Arc::new(Vapi::from_env()?))
    }
    pub fn with_driver(
        service: Arc<Service>,
        driver: Arc<dyn VoiceConnector>,
    ) -> anyhow::Result<Self> {
        service.store.recover_voice_calls()?;
        let stop = CancellationToken::new();
        let discovery = crate::worker::spawn(
            service.readiness.clone(),
            "voice-discovery",
            stop.clone(),
            discover(service.clone(), driver.clone(), stop.clone()),
        );
        let control = crate::worker::spawn(
            service.readiness.clone(),
            "voice-control",
            stop.clone(),
            control(service, driver, stop.clone()),
        );
        Ok(Self {
            stop,
            tasks: vec![discovery, control],
        })
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        crate::worker::drain(std::mem::take(&mut self.tasks), Duration::from_secs(5)).await
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.cancel();
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}
async fn discover(
    service: Arc<Service>,
    driver: Arc<dyn VoiceConnector>,
    cancelled: CancellationToken,
) -> anyhow::Result<()> {
    loop {
        if cancelled.is_cancelled() {
            break;
        }
        let store = service.store.clone();
        let bindings = match tokio::task::spawn_blocking(move || store.voice_ingresses()).await {
            Ok(Ok(b)) => b,
            _ => {
                tracing::warn!("Inbound route storage unavailable; discovery stopped");
                anyhow::bail!("Inbound route storage unavailable");
            }
        };
        let mut discovery_ok = true;
        let configured = !bindings.is_empty();
        for binding in bindings {
            let query = VoiceQuery {
                assistant_id: binding.route.assistant_id.clone(),
                phone_number_id: binding.phone_number_id.clone(),
                since: binding.since,
                scanned_until: binding.scanned_until,
                scan_until: binding.scan_until,
            };
            let calls = tokio::select! {
                _ = cancelled.cancelled() => return Ok(()),
                result = tokio::time::timeout(Duration::from_secs(35), driver.inbound(&query)) => result,
            };
            match calls {
                Ok(Ok(calls)) => {
                    let store = service.store.clone();
                    let saved = tokio::task::spawn_blocking(move || {
                        for call in calls {
                            match store.import_voice_call(&binding.route.id, &call) {
                                Ok(_) => {}
                                Err(error)
                                    if ["forbidden", "unauthorized"]
                                        .contains(&error.code.as_str()) => {}
                                Err(error) => return Err(error),
                            }
                        }
                        store.complete_voice_scan(&binding)
                    })
                    .await;
                    if matches!(saved, Ok(Ok(()))) {
                        service.notify_changed();
                    } else {
                        discovery_ok = false;
                        tracing::warn!("Inbound voice batch not fully committed; scan cursor preserved for retry");
                    }
                }
                _ => {
                    discovery_ok = false;
                    tracing::warn!("Inbound voice discovery unavailable or scan budget exceeded; cursor preserved for retry");
                }
            }
        }
        service.readiness.set(
            "voice-discovery",
            if discovery_ok && configured {
                connect_protocol::ReadinessState::Ready
            } else if configured {
                connect_protocol::ReadinessState::Degraded
            } else {
                connect_protocol::ReadinessState::Uncertain
            },
            if discovery_ok && configured {
                "scan-completed"
            } else if configured {
                "scan-incomplete"
            } else {
                "no-inbound-routes"
            },
        );
        tokio::select! {
            _ = cancelled.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {},
        }
    }
    Ok(())
}
async fn control(
    service: Arc<Service>,
    driver: Arc<dyn VoiceConnector>,
    cancelled: CancellationToken,
) -> anyhow::Result<()> {
    loop {
        if cancelled.is_cancelled() {
            break;
        }
        let store = service.store.clone();
        let job = match tokio::task::spawn_blocking(move || store.claim_voice_job()).await {
            Ok(Ok(Some(j))) => j,
            Ok(Ok(None)) => {
                tokio::select! {
                    _ = cancelled.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
                continue;
            }
            _ => {
                tracing::warn!("Voice storage unavailable; control worker stopped");
                anyhow::bail!("Voice storage unavailable");
            }
        };
        let request = VoiceStart {
            assistant_id: job.route.assistant_id.clone(),
            phone_number_id: job.route.phone_number_id.clone(),
            destination: job.call.destination.clone(),
        };
        let action = async {
            match job.action {
                VoiceAction::Start => driver.start(&request).await,
                VoiceAction::Poll => match job.call.provider_id.as_deref() {
                    Some(id) => driver.status(id).await,
                    None => Err(Failure::Rejected),
                },
                VoiceAction::End => match job.call.provider_id.as_deref() {
                    Some(id) => driver.end(id, job.control_url.as_deref()).await,
                    None => Err(Failure::Rejected),
                },
            }
        };
        let result = tokio::select! {
            _ = cancelled.cancelled() => Err(Failure::Uncertain),
            result = tokio::time::timeout(Duration::from_secs(35), action) => {
                result.unwrap_or(Err(Failure::Uncertain))
            },
        };
        // Read failures cannot invent a terminal outcome or erase accepted end intent.
        if result.is_err() && job.action == VoiceAction::Poll {
            service.readiness.set(
                "voice-control",
                connect_protocol::ReadinessState::Uncertain,
                "provider-observation-unavailable",
            );
            continue;
        }
        service.readiness.set(
            "voice-control",
            if result.is_ok() {
                connect_protocol::ReadinessState::Ready
            } else {
                connect_protocol::ReadinessState::Degraded
            },
            if result.is_ok() {
                "provider-observed"
            } else {
                "provider-operation-failed-or-uncertain"
            },
        );
        let store = service.store.clone();
        let saved = tokio::task::spawn_blocking(move || match result {
            Ok(remote) => store.finish_voice_job(
                &job,
                &remote.state,
                Some(&remote.id),
                remote.control_url.as_deref(),
                remote.ended_reason.as_deref(),
            ),
            Err(f) => store.finish_voice_job(
                &job,
                if job.action == VoiceAction::Start && matches!(f, Failure::Rejected) {
                    "failed"
                } else {
                    "uncertain"
                },
                None,
                None,
                None,
            ),
        })
        .await;
        if !matches!(saved, Ok(Ok(()))) {
            tracing::warn!("Voice result not committed; control stopped for reconciliation");
            anyhow::bail!("Voice result not committed; recover uncertain work");
        }
        service.notify_changed();
    }
    Ok(())
}
