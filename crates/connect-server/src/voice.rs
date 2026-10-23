//! Lifecycle-owned telephone workers; network effects occur after durable claims.
use connect_connector_api::{VoiceQuery, VoiceStart};
use connect_connectors::{voice::VoiceConnector, Failure, Vapi};
use connect_service::Service;
use connect_store::VoiceAction;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
// Recovery must never reinterpret another live supervisor's pending request.
// The runtime already excludes other processes; this lease excludes overlapping
// workers across Service/Store handles of the same owned runtime.
struct Lease;
fn acquire_lease(store: &connect_store::Store) -> anyhow::Result<Arc<Lease>> {
    type Key = (PathBuf, String);
    static LEASES: OnceLock<Mutex<BTreeMap<Key, Weak<Lease>>>> = OnceLock::new();
    let mut leases = LEASES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Voice worker ownership is unavailable"))?;
    leases.retain(|_, lease| lease.strong_count() > 0);
    let key = (store.path().to_owned(), store.runtime_epoch().to_owned());
    anyhow::ensure!(
        !leases.contains_key(&key),
        "Voice worker already owns this runtime; drain it before recovery"
    );
    anyhow::ensure!(
        leases.len() < 1024,
        "Voice worker ownership capacity exceeded"
    );
    let lease = Arc::new(Lease);
    leases.insert(key, Arc::downgrade(&lease));
    Ok(lease)
}
pub struct Worker {
    _lease: Arc<Lease>,
    stop: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
}
impl Worker {
    pub fn start(service: Arc<Service>) -> anyhow::Result<Self> {
        Self::with_driver(
            service,
            Arc::new(Vapi::new(
                crate::secrets::resolve("VAPI_PRIVATE_KEY")?.expose(),
                connect_connectors::vapi_api_base()?.as_str(),
            )?),
        )
    }
    pub fn with_driver(
        service: Arc<Service>,
        driver: Arc<dyn VoiceConnector>,
    ) -> anyhow::Result<Self> {
        let lease = acquire_lease(&service.store)?;
        let manifest = driver.voice_manifest();
        manifest.validate_configuration(&driver.voice_configuration())?;
        service.store.register_extension_manifest(&manifest)?;
        service.store.recover_voice_calls()?;
        let stop = CancellationToken::new();
        let mut tasks = Vec::new();
        if supports(
            &manifest,
            connect_connector_api::manifest::ConnectorOperation::VoiceDiscover,
        )
        .is_ok()
        {
            tasks.push(crate::worker::spawn(
                service.readiness.clone(),
                "voice-discovery",
                stop.clone(),
                discover(service.clone(), driver.clone(), stop.clone(), lease.clone()),
            ));
        } else {
            service.readiness.unconfigured("voice-discovery");
        }
        tasks.push(crate::worker::spawn(
            service.readiness.clone(),
            "voice-control",
            stop.clone(),
            control(service, driver, stop.clone(), manifest, lease.clone()),
        ));
        Ok(Self {
            _lease: lease,
            stop,
            tasks,
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
fn supports(
    manifest: &connect_connector_api::manifest::Manifest,
    operation: connect_connector_api::manifest::ConnectorOperation,
) -> connect_protocol::Result<()> {
    if !manifest.operations.contains(&operation) {
        return Err(connect_protocol::Error::new(
            "unsupported-connector-operation",
            "Connector operation is not declared",
        ));
    }
    Ok(())
}
async fn discover(
    service: Arc<Service>,
    driver: Arc<dyn VoiceConnector>,
    cancelled: CancellationToken,
    lease: Arc<Lease>,
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
                result = tokio::time::timeout(Duration::from_secs(35), driver.inbound_receipts(&query)) => result,
            };
            match calls {
                Ok(Ok(calls)) => {
                    let store = service.store.clone();
                    let held = lease.clone();
                    let saved = tokio::task::spawn_blocking(move || {
                        let _lease = held;
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
    manifest: connect_connector_api::manifest::Manifest,
    lease: Arc<Lease>,
) -> anyhow::Result<()> {
    loop {
        if cancelled.is_cancelled() {
            break;
        }
        let store = service.store.clone();
        let held = lease.clone();
        let job = match tokio::task::spawn_blocking(move || {
            let _lease = held;
            store.claim_voice_job()
        })
        .await
        {
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
        use connect_connector_api::manifest::ConnectorOperation;
        let operation = match job.action {
            VoiceAction::Start => ConnectorOperation::VoiceStart,
            VoiceAction::Poll => ConnectorOperation::VoiceStatus,
            VoiceAction::End => ConnectorOperation::VoiceEnd,
        };
        if supports(&manifest, operation).is_err() {
            let store = service.store.clone();
            let held = lease.clone();
            tokio::task::spawn_blocking(move || {
                let _lease = held;
                use connect_connector_api::lifecycle::{
                    Observation, State, Verification, VoiceState,
                };
                let state = if job.action == VoiceAction::Start {
                    "failed"
                } else {
                    "uncertain"
                };
                let observation = Observation {
                    mutation_id: job.call.id.clone(),
                    external_id: job.call.provider_id.clone(),
                    event_id: format!("unsupported:{}:{operation:?}", job.call.id),
                    state: State::Voice(if state == "failed" {
                        VoiceState::Failed
                    } else {
                        VoiceState::Uncertain
                    }),
                    verification: Verification::LocalAttempt,
                };
                store.finish_voice_observation(
                    &job,
                    state,
                    job.call.provider_id.as_deref(),
                    job.control_url.as_deref(),
                    Some("unsupported-connector-operation"),
                    &observation,
                )
            })
            .await
            .map_err(|_| anyhow::anyhow!("Voice capability admission task failed"))??;
            service.readiness.set(
                "voice-control",
                connect_protocol::ReadinessState::Degraded,
                "unsupported-connector-operation",
            );
            service.notify_changed();
            continue;
        }
        let check_store = service.store.clone();
        let held = lease.clone();
        let dispatch = tokio::task::spawn_blocking(move || {
            let _lease = held;
            check_store.begin_voice_dispatch(&job)
        })
        .await
        .map_err(|_| anyhow::anyhow!("Voice dispatch admission task failed"))??;
        let Some(dispatch) = dispatch else {
            continue;
        };
        let action_kind = dispatch.job()?.action;
        let budget = (std::time::UNIX_EPOCH + Duration::from_secs(dispatch.expires_at() as u64))
            .duration_since(std::time::SystemTime::now())
            .unwrap_or_default();
        let action = async {
            let job = dispatch.job().map_err(|_| Failure::Rejected)?;
            let request = VoiceStart {
                assistant_id: job.route.assistant_id.clone(),
                phone_number_id: job.route.phone_number_id.clone(),
                destination: job.call.destination.clone(),
            };
            match job.action {
                VoiceAction::Start => driver.start_receipt(&job.call.id, &request).await,
                VoiceAction::Poll => match job.call.provider_id.as_deref() {
                    Some(id) => driver.status_receipt(&job.call.id, id).await,
                    None => Err(Failure::Rejected),
                },
                VoiceAction::End => match job.call.provider_id.as_deref() {
                    Some(id) => {
                        driver
                            .end_receipt(&job.call.id, id, job.control_url.as_deref())
                            .await
                    }
                    None => Err(Failure::Rejected),
                },
            }
        };
        let withdrawn = async {
            while dispatch.current() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancelled.cancelled() => Err(Failure::Uncertain),
            _ = withdrawn => Err(Failure::Uncertain),
            result = tokio::time::timeout(budget, action) => result.unwrap_or(Err(Failure::Uncertain)),
        };
        // Read failures cannot invent a terminal outcome or erase accepted end intent.
        if result.is_err() && action_kind == VoiceAction::Poll {
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
        // Capture before dropping the exact decoded outcome. Transport authority
        // is released here, before a result writer can wait behind revocation.
        let captured = Arc::new(dispatch.capture(result));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        loop {
            let store = service.store.clone();
            let captured = captured.clone();
            let held = lease.clone();
            let saved = tokio::task::spawn_blocking(move || {
                let _lease = held;
                store.record_voice_dispatch(&captured)
            })
            .await;
            match saved {
                Ok(Ok(())) => break,
                Ok(Err(error))
                    if matches!(
                        error.code.as_str(),
                        "storage-unavailable" | "authority-busy" | "authority-unavailable"
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                _ => {
                    tracing::warn!(
                        "Captured voice result not committed; control stopped for reconciliation"
                    );
                    anyhow::bail!("Captured voice result not committed; recover uncertain work");
                }
            }
        }
        service.notify_changed();
    }
    Ok(())
}
