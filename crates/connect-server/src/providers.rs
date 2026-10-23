//! Owned provider worker. One request per job, serialized chat context.
use connect_connector_api::DeliveryRequest;
use connect_connectors::{Connector, Failure, Telnyx, Vapi};
use connect_service::Service;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

pub struct Worker {
    stop: CancellationToken,
    role: Arc<connect_store::ProviderWorkerOwnership>,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}
impl Worker {
    pub async fn start(service: Arc<Service>) -> anyhow::Result<Self> {
        let mut drivers: Vec<Arc<dyn Connector>> = Vec::new();
        if let Some(key) = crate::secrets::optional("TELNYX_API_KEY")? {
            drivers.push(Arc::new(Telnyx::new(
                key.expose(),
                "https://api.telnyx.com/v2",
            )?));
        }
        if let Some(key) = crate::secrets::optional("VAPI_PRIVATE_KEY")? {
            drivers.push(Arc::new(Vapi::new(
                key.expose(),
                connect_connectors::vapi_api_base()?.as_str(),
            )?));
        }
        anyhow::ensure!(
            !drivers.is_empty(),
            "Provider worker requires TELNYX_API_KEY or VAPI_PRIVATE_KEY"
        );
        let authority = crate::sms_authority::Authority::from_env()?;
        Self::compose(
            service,
            drivers,
            authority.map(|policy| Arc::new(policy) as Arc<dyn crate::SmsPolicy>),
            true,
            None,
        )
        .await
    }
    /// Public adapter boundary also used by deterministic integration fixtures.
    pub async fn with_drivers(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
    ) -> anyhow::Result<Self> {
        Self::compose(service, drivers, None, false, None).await
    }
    /// Compose another authenticated enrollment service without provider-specific
    /// consent branches. Agents cannot invoke this protected composition boundary.
    pub async fn with_policy(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
        authority: Option<Arc<dyn crate::SmsPolicy>>,
    ) -> anyhow::Result<Self> {
        let expire = authority.is_some();
        Self::compose(service, drivers, authority, expire, None).await
    }
    pub async fn with_endpoint_routes(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
        operator: String,
        bindings: Vec<connect_store::ProviderMessageEndpoint>,
        authority: Option<Arc<dyn crate::SmsPolicy>>,
        expire: bool,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !bindings.is_empty(),
            "Message hosting requires explicit Endpoint routes"
        );
        let mut seen = std::collections::BTreeSet::new();
        for driver in &drivers {
            anyhow::ensure!(
                seen.insert(driver.name().to_owned()),
                "One message account per connector must be selected explicitly"
            );
            anyhow::ensure!(
                driver.manifest().messaging
                    && driver.manifest().operations.contains(
                        &connect_connector_api::manifest::ConnectorOperation::MessageSend
                    ),
                "Canonical message routes require sending capability"
            );
        }
        for p in &bindings {
            let driver = drivers
                .iter()
                .find(|d| d.name() == p.connector)
                .ok_or_else(|| {
                    anyhow::anyhow!("Endpoint route has no configured message account")
                })?;
            let field = if p.connector == "vapi" {
                "private-key"
            } else {
                "api-key"
            };
            anyhow::ensure!(
                driver.configuration().get(field)
                    == Some(&format!("secret:{}", p.secret_reference)),
                "Endpoint route differs from selected account reference"
            );
        }
        Self::compose(
            service,
            drivers,
            authority,
            expire,
            Some((operator, bindings)),
        )
        .await
    }
    async fn compose(
        service: Arc<Service>,
        drivers: Vec<Arc<dyn Connector>>,
        authority: Option<Arc<dyn crate::SmsPolicy>>,
        expire: bool,
        endpoint_routes: Option<(String, Vec<connect_store::ProviderMessageEndpoint>)>,
    ) -> anyhow::Result<Self> {
        let mut registry = connect_connector_api::manifest::Registry::default();
        let mut contracts = Vec::new();
        // Validate the entire composition before changing registration or claiming work.
        for driver in &drivers {
            let manifest = driver.manifest();
            let descriptor = driver.descriptor();
            manifest.validate_descriptor(descriptor)?;
            anyhow::ensure!(
                driver.name() == descriptor.name,
                "Connector name differs from descriptor"
            );
            manifest.validate_configuration(&driver.configuration())?;
            anyhow::ensure!(
                driver.readiness() == connect_connector_api::manifest::Readiness::Ready,
                "Connector is not ready"
            );
            registry.register(manifest.clone())?;
            contracts.push((descriptor, manifest));
        }
        let setup_store = service.store.clone();
        let setup_contracts = contracts.clone();
        let selected_routes = endpoint_routes
            .as_ref()
            .map(|(_, p)| p.iter().map(|p| p.route_id.clone()).collect::<Vec<_>>());
        let role = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let role = setup_store.acquire_provider_worker()?;
            if expire {
                setup_store.expire_sms_authorities()?;
            }
            for (descriptor, manifest) in &setup_contracts {
                setup_store.register_connector_manifest(*descriptor, manifest)?;
            }
            if let Some((operator, bindings)) = &endpoint_routes {
                setup_store.bind_provider_message_endpoints(operator, bindings)?;
            }
            setup_store.recover_provider_jobs()?;
            Ok(role)
        })
        .await??;
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let owned_role = role.clone();
        let task = crate::worker::spawn(
            service.readiness.clone(),
            "provider-delivery",
            stop.clone(),
            async move {
                let mut next_refresh = tokio::time::Instant::now();
                loop {
                    if cancelled.is_cancelled() {
                        break;
                    }
                    if let Some(authority) = &authority {
                        if tokio::time::Instant::now() >= next_refresh {
                            let result = tokio::select! {
                                _ = cancelled.cancelled() => break,
                                value = crate::synchronize_sms_policy(authority.as_ref(), service.store.clone()) => value,
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
                    let routes = selected_routes.clone();
                    let job = tokio::task::spawn_blocking(move || match routes {
                        Some(ids) => store.claim_provider_job_for_routes(&ids),
                        None => store.claim_provider_job(),
                    })
                    .await;
                    match job {
                        Ok(Ok(Some(job))) => {
                            let admission_store = service.store.clone();
                            let admission_job = job.clone();
                            let admission_role = owned_role.clone();
                            let configuration = drivers
                                .iter()
                                .find(|d| d.name() == job.route.connector)
                                .map(|d| d.configuration())
                                .unwrap_or_default();
                            let permit = tokio::task::spawn_blocking(move || {
                                admission_store.admit_configured_provider_dispatch(
                                    &admission_role,
                                    &admission_job,
                                    &connect_store::AuthorityWait::new(Duration::from_secs(2)),
                                    &configuration,
                                )
                            })
                            .await;
                            let permit=match permit {
                                Ok(Ok(permit))=>permit,
                                Ok(Err(error)) if matches!(error.code.as_str(),"forbidden"|"unauthorized"|"conflict")=>{
                                    let rejected_store=service.store.clone();let rejected_job=job.clone();
                                    tokio::task::spawn_blocking(move||rejected_store.reject_provider_claim(&rejected_job)).await??;
                                    service.notify_changed(); continue;
                                },
                                _=>anyhow::bail!("Provider dispatch authority unavailable; retained claim requires recovery"),
                            };

                            let request = DeliveryRequest {
                                target: job.route.target.clone(),
                                source: job.route.source.clone(),
                                body: job.body.clone(),
                                previous_chat_id: job.previous_chat_id.clone(),
                                context: job.context.clone(),
                            };
                            let selected = drivers.iter().zip(contracts.iter()).find(
                                |(_, (descriptor, _))| descriptor.name == job.route.connector,
                            );
                            let result = async { if let Some((driver, (_, manifest))) = selected {
                                if !manifest.operations.contains(
                                    &connect_connector_api::manifest::ConnectorOperation::MessageSend,
                                ) || manifest
                                    .validate_address(&request.target, request.source.as_deref())
                                    .is_err()
                                    || request.body.len() > manifest.limits.message_bytes
                                    || request.context.as_ref().is_some_and(|v| {
                                        !manifest.context || v.len() > manifest.limits.context_bytes
                                    })
                                {
                                    Err(Failure::Rejected)
                                } else {
                                    tokio::select! {
                                        _ = cancelled.cancelled() => Err(Failure::Uncertain),
                                        result = tokio::time::timeout(Duration::from_secs(35), driver.deliver_observed(&job.id, &request, permit)) => {
                                            result.unwrap_or(Err(Failure::Uncertain))
                                        },
                                    }
                                }
                            } else {
                                Err(Failure::Rejected)
                            } }.await;
                            // An adapter response belongs only to the claimed attempt.
                            // Do not let a malformed receipt select another durable
                            // job. Dispatch may already have succeeded, so retain the
                            // original ID as uncertain for read-only reconciliation.
                            let result = result.and_then(|receipt| {
                                if receipt.observation.mutation_id == job.id {
                                    Ok(receipt)
                                } else {
                                    Err(Failure::Uncertain)
                                }
                            });
                            let observed = result.unwrap_or_else(|failure| {
                                connect_connector_api::lifecycle::message_attempt(
                                    &job.id,
                                    Err(failure),
                                )
                            });
                            let store = service.store.clone();
                            let saved = tokio::task::spawn_blocking(move || {
                                store
                                    .apply_connector_receipt(&job.route.connector, &observed)
                                    .map(|_| ())
                            })
                            .await;
                            if !matches!(saved, Ok(Ok(()))) {
                                tracing::warn!(
                                "Provider result not committed; stopping worker with uncertain job"
                            );
                                anyhow::bail!(
                                    "Provider result not committed; recover uncertain work"
                                );
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
            },
        );
        Ok(Self {
            stop,
            role,
            task: Some(task),
        })
    }
    /// Explicit read-only reconciliation of an existing logical attempt. Callers
    /// use the same trusted adapter composition as dispatch; uncertain sends are
    /// never resubmitted to discover their outcome.
    pub async fn reconcile(
        service: &Service,
        driver: &dyn Connector,
        mutation_id: &str,
    ) -> anyhow::Result<connect_connector_api::lifecycle::Apply> {
        use connect_connector_api::{lifecycle::Verification, manifest::ConnectorOperation};
        let (descriptor, manifest) = driver_contract(driver)?;
        anyhow::ensure!(
            manifest
                .operations
                .contains(&ConnectorOperation::MessageStatus),
            "Connector has no status operation"
        );
        let extension = driver
            .message_extension()
            .ok_or_else(|| anyhow::anyhow!("Connector has no normalized message extension"))?;
        let admission_store = service.store.clone();
        let id = mutation_id.to_owned();
        let connector = driver.name().to_owned();
        let configuration = driver.configuration();
        let admission = tokio::task::spawn_blocking(move || {
            admission_store.register_connector_manifest(descriptor, &manifest)?;
            admission_store.admit_configured_provider_reconciliation(
                &id,
                &connector,
                &connect_store::AuthorityWait::new(Duration::from_secs(2)),
                &configuration,
            )
        })
        .await??;
        let id = admission.job.id;
        let deadline = admission.permit.deadline();
        let receipt = tokio::time::timeout_at(
            deadline.into(),
            extension.message_status(&id, admission.permit),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Connector reconciliation timed out"))?
        .map_err(|_| anyhow::anyhow!("Connector reconciliation unavailable"))?;
        anyhow::ensure!(
            receipt.observation.mutation_id == id
                && receipt.observation.verification == Verification::Reconciliation,
            "Invalid connector reconciliation"
        );
        let settlement_store = service.store.clone();
        let connector = driver.name().to_owned();
        let applied = tokio::task::spawn_blocking(move || {
            settlement_store.apply_connector_receipt(&connector, &receipt)
        })
        .await??;
        service.notify_changed();
        Ok(applied)
    }
    /// Protected ingress for an adapter-authenticated independent contact.
    /// Route configuration establishes the principal; provider IDs do not.
    pub fn receive(
        service: &Service,
        driver: &dyn Connector,
        route_id: &str,
        message: &connect_connector_api::lifecycle::IncomingMessage,
    ) -> anyhow::Result<String> {
        use connect_connector_api::manifest::ConnectorOperation;
        validate_driver(service, driver)?;
        service
            .store
            .require_provider_message_configuration(route_id, &driver.configuration())?;
        let manifest = driver.manifest();
        anyhow::ensure!(
            manifest
                .operations
                .contains(&ConnectorOperation::MessageReceive)
                && message.body.len() <= manifest.limits.message_bytes,
            "Connector incoming message is unsupported or exceeds bounds"
        );
        let id = service
            .store
            .receive_connector_message(driver.name(), route_id, message)?;
        service.notify_changed();
        Ok(id)
    }
    /// Apply a lifecycle callback only after the adapter verifies its exact raw
    /// bytes and binds it to the original logical attempt. Policy, source bounds
    /// and replay handling remain inside the transactional host boundary.
    pub fn observe(
        service: &Service,
        driver: &dyn Connector,
        receipt: &connect_connector_api::lifecycle::MessageReceipt,
    ) -> anyhow::Result<connect_connector_api::lifecycle::Apply> {
        use connect_connector_api::lifecycle::Verification;
        validate_driver(service, driver)?;
        service.store.require_provider_observation_configuration(
            &receipt.observation.mutation_id,
            driver.name(),
            &driver.configuration(),
        )?;
        anyhow::ensure!(
            driver.message_extension().is_some()
                && driver.manifest().messaging
                && receipt.observation.verification == Verification::VerifiedCallback,
            "Connector callback has not been verified"
        );
        let applied = service
            .store
            .apply_connector_receipt(driver.name(), receipt)?;
        service.notify_changed();
        Ok(applied)
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.role.retire();
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
        self.role.retire();
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn validate_driver(service: &Service, driver: &dyn Connector) -> anyhow::Result<()> {
    let (descriptor, manifest) = driver_contract(driver)?;
    service
        .store
        .register_connector_manifest(descriptor, &manifest)?;
    Ok(())
}
fn driver_contract(
    driver: &dyn Connector,
) -> anyhow::Result<(
    connect_connector_api::Descriptor,
    connect_connector_api::manifest::Manifest,
)> {
    let manifest = driver.manifest();
    let descriptor = driver.descriptor();
    manifest.validate_descriptor(descriptor)?;
    anyhow::ensure!(
        driver.name() == descriptor.name,
        "Connector name differs from descriptor"
    );
    manifest.validate_configuration(&driver.configuration())?;
    anyhow::ensure!(
        driver.readiness() == connect_connector_api::manifest::Readiness::Ready,
        "Connector is not ready"
    );
    Ok((descriptor, manifest))
}
