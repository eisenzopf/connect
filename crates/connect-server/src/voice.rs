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
        Self::with_contract(service, driver, false)
    }
    /// Explicit normalized lifecycle composition. Existing telephone startup and
    /// with_driver retain their carrier-qualified compatibility dispatch path.
    pub fn with_observed_driver(
        service: Arc<Service>,
        driver: Arc<dyn VoiceConnector>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            driver.voice_manifest().is_some(),
            "Normalized voice driver requires a manifest"
        );
        Self::with_contract(service, driver, true)
    }
    fn with_contract(
        service: Arc<Service>,
        driver: Arc<dyn VoiceConnector>,
        normalized: bool,
    ) -> anyhow::Result<Self> {
        let manifest = driver.voice_manifest();
        if let Some(manifest) = &manifest {
            manifest.validate_configuration(&driver.voice_configuration())?;
            service.store.register_extension_manifest(manifest)?;
        }
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
                discover(service.clone(), driver.clone(), stop.clone()),
            ));
        } else {
            service.readiness.unconfigured("voice-discovery");
        }
        let control = crate::worker::spawn(
            service.readiness.clone(),
            "voice-control",
            stop.clone(),
            control(service, driver, stop.clone(), normalized, manifest),
        );
        tasks.push(control);
        Ok(Self { stop, tasks })
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
    manifest: &Option<connect_connector_api::manifest::Manifest>,
    operation: connect_connector_api::manifest::ConnectorOperation,
) -> connect_protocol::Result<()> {
    if manifest
        .as_ref()
        .is_some_and(|manifest| !manifest.operations.contains(&operation))
    {
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
    normalized: bool,
    manifest: Option<connect_connector_api::manifest::Manifest>,
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
        use connect_connector_api::manifest::ConnectorOperation;
        let operation = match job.action {
            VoiceAction::Start => ConnectorOperation::VoiceStart,
            VoiceAction::Poll => ConnectorOperation::VoiceStatus,
            VoiceAction::End => ConnectorOperation::VoiceEnd,
        };
        if supports(&manifest, operation).is_err() {
            // No possible physical dispatch is admitted for an undeclared
            // operation. Known starts fail locally; unavailable status/end
            // cannot invent termination of an already-owned remote call.
            let store = service.store.clone();
            tokio::task::spawn_blocking(move || {
                let state = if job.action == VoiceAction::Start {
                    "failed"
                } else {
                    "uncertain"
                };
                if normalized {
                    use connect_connector_api::lifecycle::{
                        Observation, State, Verification, VoiceState,
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
                } else {
                    store.finish_voice_job(
                        &job,
                        state,
                        job.call.provider_id.as_deref(),
                        job.control_url.as_deref(),
                        Some("unsupported-connector-operation"),
                    )
                }
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
        let admitted = tokio::task::spawn_blocking(move || {
            let admitted = check_store.voice_dispatch_allowed(&job)?;
            Ok::<_, connect_protocol::Error>((job, admitted))
        })
        .await
        .map_err(|_| anyhow::anyhow!("Voice dispatch admission task failed"))??;
        let (job, admitted) = admitted;
        if !admitted {
            continue;
        }
        let request = VoiceStart {
            assistant_id: job.route.assistant_id.clone(),
            phone_number_id: job.route.phone_number_id.clone(),
            destination: job.call.destination.clone(),
        };
        let action = async {
            if normalized {
                let receipt = match job.action {
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
                }?;
                Ok((receipt.call, Some(receipt.observation)))
            } else {
                let call = match job.action {
                    VoiceAction::Start => driver.start(&request).await,
                    VoiceAction::Poll => match job.call.provider_id.as_deref() {
                        Some(id) => driver.status(id).await,
                        None => Err(Failure::Rejected),
                    },
                    VoiceAction::End => match job.call.provider_id.as_deref() {
                        Some(id) => driver.end(id, job.control_url.as_deref()).await,
                        None => Err(Failure::Rejected),
                    },
                }?;
                Ok((call, None))
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
            Ok((remote, observation)) => match observation {
                Some(observation) => store.finish_voice_observation(
                    &job,
                    &remote.state,
                    Some(&remote.id),
                    remote.control_url.as_deref(),
                    remote.ended_reason.as_deref(),
                    &observation,
                ),
                None => store.finish_voice_job(
                    &job,
                    &remote.state,
                    Some(&remote.id),
                    remote.control_url.as_deref(),
                    remote.ended_reason.as_deref(),
                ),
            },
            Err(f) => {
                let state = if job.action == VoiceAction::Start && matches!(f, Failure::Rejected) {
                    "failed"
                } else {
                    "uncertain"
                };
                if normalized {
                    use connect_connector_api::lifecycle::{
                        Observation, State, Verification, VoiceState,
                    };
                    let observation = Observation {
                        mutation_id: job.call.id.clone(),
                        external_id: None,
                        event_id: format!("attempt:{}:{:?}:{state}", job.call.id, job.action),
                        state: State::Voice(if state == "failed" {
                            VoiceState::Failed
                        } else {
                            VoiceState::Uncertain
                        }),
                        verification: Verification::LocalAttempt,
                    };
                    store.finish_voice_observation(&job, state, None, None, None, &observation)
                } else {
                    store.finish_voice_job(&job, state, None, None, None)
                }
            }
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
