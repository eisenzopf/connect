//! Once-only owned provider initialization from an admitted public consultation.
use super::*;
use connect_service::{ConsultationInitialization, ConsultationInitializer};
use std::{
    collections::BTreeMap,
    sync::{Mutex, Weak},
};

/// Protected host configuration. No participant command chooses peers, keys,
/// provider credentials, model prompts, transport origins or a context owner.
pub struct PrivateVapiTarget {
    pub native_peer: CanonicalClientConfig,
    pub native_endpoint: String,
    pub provider_endpoint: String,
    pub profile: PrivateVapiProfile,
}
struct Job {
    ready: watch::Receiver<Option<Result<()>>>,
    state: watch::Receiver<PrivateVapiStatus>,
    task: JoinHandle<()>,
}
pub struct PrivateVapiCoordinator {
    service: Weak<Service>,
    targets: BTreeMap<String, Arc<PrivateVapiTarget>>,
    secrets: ProviderSecrets,
    stop: CancellationToken,
    jobs: Mutex<BTreeMap<String, Job>>,
    draining: tokio::sync::Mutex<()>,
    capacity: usize,
}
impl PrivateVapiCoordinator {
    pub fn install(
        service: &Arc<Service>,
        targets: Vec<PrivateVapiTarget>,
        secrets: ProviderSecrets,
    ) -> Result<Arc<Self>> {
        let capacity = service.store.limits().connections;
        if targets.is_empty() || targets.len() > capacity {
            return Err(problem("unsupported-media-profile"));
        }
        let mut configured = BTreeMap::new();
        for target in targets {
            target.profile.validate()?;
            target
                .native_peer
                .validate()
                .map_err(|_| problem("private-native-peer-failed"))?;
            if target.native_peer.timeout_ms > 5000
                || target.native_endpoint.is_empty()
                || target.provider_endpoint.is_empty()
            {
                return Err(problem("unsupported-media-profile"));
            }
            let identity = target.native_peer.identity_id.clone();
            if configured.insert(identity, Arc::new(target)).is_some() {
                return Err(problem("conflict"));
            }
        }
        let host = Arc::new(Self {
            service: Arc::downgrade(service),
            targets: configured,
            secrets,
            stop: CancellationToken::new(),
            jobs: Mutex::new(BTreeMap::new()),
            draining: tokio::sync::Mutex::new(()),
            capacity,
        });
        service.install_consultation_initializer(host.clone())?;
        Ok(host)
    }
    /// Safe per-record observation; no model context, credentials or media.
    pub fn job_count(&self) -> usize {
        self.jobs.lock().map(|j| j.len()).unwrap_or(self.capacity)
    }
    pub fn status(&self, record: &str) -> Option<PrivateVapiStatus> {
        self.jobs.lock().ok()?.get(record).map(|j| {
            let status = j.state.borrow().clone();
            if j.task.is_finished() && !matches!(status, PrivateVapiStatus::Stopped { .. }) {
                PrivateVapiStatus::Stopped {
                    problem: Some("storage-unavailable".into()),
                }
            } else {
                status
            }
        })
    }
}
impl Drop for PrivateVapiCoordinator {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[async_trait::async_trait]
impl ConsultationInitializer for PrivateVapiCoordinator {
    async fn initialize(&self, request: ConsultationInitialization) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(problem("cancelled"));
        }
        let service = self.service.upgrade().ok_or_else(Error::forbidden)?;
        let store = service.store.clone();
        let credential = request.credential_id.clone();
        let command = request.command.clone();
        let current =
            store_call(move || store.native_consultation_origin(&credential, command)).await?;
        if current.record_id() != request.origin.record_id()
            || current.request() != request.origin.request()
        {
            return Err(Error::forbidden());
        }
        let store = service.store.clone();
        let origin = request.origin.clone();
        let target = store_call(move || store.native_consultation_target(&origin)).await?;
        // Human peers continue through the signed native invitation path.
        if target.kind != "ai" {
            return Ok(());
        }
        let profile = self
            .targets
            .get(&target.identity_id)
            .cloned()
            .ok_or_else(|| problem("unsupported-media-profile"))?;
        if !target.present {
            let s = service.store.clone();
            let origin = request.origin.clone();
            let configured = profile.clone();
            store_call(move || {
                let delegate = s.authenticate(&configured.native_peer.credential)?;
                if delegate.identity_id != configured.native_peer.identity_id {
                    return Err(Error::forbidden());
                }
                s.join_configured_native_consultant(
                    &origin,
                    &delegate.credential_id,
                    &configured.native_endpoint,
                )
            })
            .await?;
        }
        let record = request.origin.record_id().to_owned();
        let mut ready = {
            let mut jobs = self.jobs.lock().map_err(|_| Error::storage())?;
            if self.stop.is_cancelled() {
                return Err(problem("cancelled"));
            }
            if let Some(job) = jobs.get(&record) {
                job.ready.clone()
            } else {
                jobs.retain(|_, job| {
                    !job.task.is_finished()
                        || !matches!(*job.state.borrow(), PrivateVapiStatus::Stopped { .. })
                });
                // Completed process-local cache entries may be pruned, but an
                // already registered durable leg can never start another job.
                if target.registered {
                    return Err(problem("reconciliation-required"));
                }
                if jobs.len() >= self.capacity {
                    return Err(problem("capacity-exceeded"));
                }
                let (tx, rx) = watch::channel(None);
                let (state, state_rx) = watch::channel(PrivateVapiStatus::Preparing);
                let stop = self.stop.child_token();
                let secrets = self.secrets.clone();
                let task = tokio::spawn(async move {
                    let origin = request.origin;
                    let store = service.store.clone();
                    let invocation = PrivateVapiInvocation {
                        command: request.command,
                        execution_credential: request.credential_id.clone(),
                        context_credential: request.credential_id,
                        native_peer: profile.native_peer.clone(),
                        native_endpoint: profile.native_endpoint.clone(),
                        provider_endpoint: profile.provider_endpoint.clone(),
                    };
                    match PrivateVapiBridge::start_with_stop(
                        service,
                        invocation,
                        profile.profile.clone(),
                        secrets,
                        stop,
                    )
                    .await
                    {
                        Ok(mut bridge) => {
                            let result = bridge.context_ready().await;
                            state.send_replace(bridge.status());
                            tx.send_replace(Some(result));
                            let result = bridge.finish().await;
                            let code = result
                                .as_ref()
                                .err()
                                .map(|e| e.code.clone())
                                .unwrap_or_else(|| "cancelled".into());
                            let cleanup = CancellationToken::new();
                            let _ = preflight(
                                || {
                                    let store = store.clone();
                                    let origin = origin.clone();
                                    let code = code.clone();
                                    store_call(move || {
                                        store
                                            .fail_native_consultation_initialization(&origin, &code)
                                    })
                                },
                                &cleanup,
                            )
                            .await;
                            state.send_replace(PrivateVapiStatus::Stopped {
                                problem: result.err().map(|e| e.code),
                            });
                        }
                        Err(error) => {
                            state.send_replace(PrivateVapiStatus::Stopped {
                                problem: Some(error.code.clone()),
                            });
                            tx.send_replace(Some(Err(error)));
                        }
                    }
                });
                jobs.insert(
                    record,
                    Job {
                        ready: rx.clone(),
                        state: state_rx,
                        task,
                    },
                );
                rx
            }
        };
        loop {
            if let Some(result) = ready.borrow().clone() {
                return result;
            }
            tokio::select! {_=self.stop.cancelled()=>return Err(problem("cancelled")),value=ready.changed()=>{value.map_err(|_|Error::storage())?;}}
        }
    }
    fn cancel(&self) {
        self.stop.cancel();
    }
    async fn drain(&self) -> Result<()> {
        self.stop.cancel();
        let _drain = self.draining.lock().await;
        let mut jobs = std::mem::take(&mut *self.jobs.lock().map_err(|_| Error::storage())?);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut failed = false;
        while let Some((record, mut job)) = jobs.pop_first() {
            match tokio::time::timeout_at(deadline, &mut job.task).await {
                Ok(result) => {
                    failed |= result.is_err();
                }
                Err(_) => {
                    jobs.insert(record, job);
                    self.jobs.lock().map_err(|_| Error::storage())?.extend(jobs);
                    return Err(problem("timeout"));
                }
            }
        }
        if failed {
            Err(Error::storage())
        } else {
            Ok(())
        }
    }
}
