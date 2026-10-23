//! Owned provider jobs after an AI accepts an ordinary canonical Session.
use super::*;
use connect_client::canonical::{CanonicalClient, CanonicalClientConfig, CommandScope};
use connect_protocol::{routing::ReachabilityPolicy, session_control::Action};
use connect_service::{SessionInitialization, SessionInitializer};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    collections::BTreeMap,
    sync::{Mutex, Weak},
};

/// Protected host configuration. The accepting identity must match this exact
/// native peer. Runtime profiles describe constructed adapters/secret bindings;
/// they cannot create a verified Endpoint or fabricate reachability.
pub struct PublicVapiTarget {
    pub native_peer: CanonicalClientConfig,
    pub provider_endpoint: String,
    pub profile: AssistantVapiProfile,
    pub routing: connect_store::DiscoveryRuntime,
    pub reachability: ReachabilityPolicy,
}
struct Job {
    state: watch::Receiver<AssistantVapiStatus>,
    task: JoinHandle<Result<()>>,
    cleanup_unconfirmed: Arc<AtomicBool>,
}
pub struct PublicVapiCoordinator {
    service: Weak<Service>,
    targets: BTreeMap<String, Arc<PublicVapiTarget>>,
    secrets: ProviderSecrets,
    stop: CancellationToken,
    jobs: Mutex<BTreeMap<String, Job>>,
    draining: tokio::sync::Mutex<()>,
    cleanup_unconfirmed: AtomicBool,
    capacity: usize,
}
impl PublicVapiCoordinator {
    /// Install before accepting public Sessions. Session acceptance remains an
    /// explicit current participant decision; installation never auto-accepts.
    pub fn install(
        service: &Arc<Service>,
        targets: Vec<PublicVapiTarget>,
        secrets: ProviderSecrets,
    ) -> Result<Arc<Self>> {
        let capacity = service.store.limits().connections;
        if targets.is_empty() || targets.len() > capacity {
            return Err(Error::new(
                "capacity-exceeded",
                "Invalid public assistant capacity",
            ));
        }
        let mut configured = BTreeMap::new();
        for target in targets {
            target.profile.validate()?;
            target
                .native_peer
                .validate()
                .map_err(|_| Error::new("invalid-request", "Invalid configured assistant peer"))?;
            rvoip_uctp::v2::Identifier::try_from(target.provider_endpoint.clone()).map_err(
                |_| Error::new("invalid-request", "Invalid configured assistant endpoint"),
            )?;
            if target.native_peer.timeout_ms > 5000
                || configured
                    .insert(target.native_peer.identity_id.clone(), Arc::new(target))
                    .is_some()
            {
                return Err(Error::new(
                    "invalid-request",
                    "Invalid or duplicate configured assistant",
                ));
            }
        }
        let host = Arc::new(Self {
            service: Arc::downgrade(service),
            targets: configured,
            secrets,
            stop: CancellationToken::new(),
            jobs: Mutex::new(BTreeMap::new()),
            draining: tokio::sync::Mutex::new(()),
            cleanup_unconfirmed: AtomicBool::new(false),
            capacity,
        });
        service.install_session_initializer("public-vapi", host.clone())?;
        Ok(host)
    }
    pub fn job_count(&self) -> usize {
        self.jobs.lock().map(|j| j.len()).unwrap_or(self.capacity)
    }
    pub fn status(
        &self,
        cid: &str,
        sid: &str,
        identity: &str,
        epoch: u64,
    ) -> Option<AssistantVapiStatus> {
        self.jobs
            .lock()
            .ok()?
            .get(&key(cid, sid, identity, epoch))
            .map(|j| {
                let state = j.state.borrow().clone();
                if j.task.is_finished() && !matches!(state, AssistantVapiStatus::Stopped { .. }) {
                    AssistantVapiStatus::Stopped {
                        problem: Some("storage-unavailable".into()),
                    }
                } else {
                    state
                }
            })
    }
}
impl Drop for PublicVapiCoordinator {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[async_trait::async_trait]
impl SessionInitializer for PublicVapiCoordinator {
    async fn initialize(&self, invocation: SessionInitialization) -> Result<()> {
        if !matches!(invocation.request.action, Action::Accept { .. }) {
            return Ok(());
        }
        if self.stop.is_cancelled() {
            return Err(Error::new("cancelled", "Public assistant host stopped"));
        }
        let service = self.service.upgrade().ok_or_else(Error::forbidden)?;
        let store = service.store.clone();
        let credential = invocation.credential_id.clone();
        let actor = blocking(move || store.revalidate(&credential)).await?;
        let Some(target) = self.targets.get(&actor.identity_id).cloned() else {
            return Ok(());
        };
        let store = service.store.clone();
        let credential = invocation.credential_id.clone();
        let cid = invocation.request.conversation_id.clone();
        let membership = blocking(move || store.membership_snapshot(&credential, &cid)).await?;
        let member = membership
            .participants
            .iter()
            .find(|p| {
                p.active
                    && p.identity_id == actor.identity_id
                    && p.kind == connect_protocol::provisioning::IdentityKind::Ai
            })
            .ok_or_else(Error::forbidden)?;
        let id = key(
            &invocation.request.conversation_id,
            &invocation.request.session_id,
            &actor.identity_id,
            member.membership_epoch,
        );
        let mut jobs = self.jobs.lock().map_err(|_| Error::storage())?;
        if self.stop.is_cancelled() {
            return Err(Error::new("cancelled", "Public assistant host stopped"));
        }
        if jobs.contains_key(&id) {
            return Ok(());
        }
        jobs.retain(|_, j| !j.task.is_finished() || j.cleanup_unconfirmed.load(Ordering::Acquire));
        if jobs.len() >= self.capacity {
            return Err(Error::new(
                "capacity-exceeded",
                "Public assistant capacity exhausted",
            ));
        }
        let (state, rx) = watch::channel(AssistantVapiStatus::Preparing);
        let secrets = self.secrets.clone();
        let stop = self.stop.child_token();
        let job_id = id.clone();
        let cleanup_unconfirmed = Arc::new(AtomicBool::new(false));
        let cleanup = cleanup_unconfirmed.clone();
        let task = tokio::spawn(async move {
            let result = run_job(
                service, invocation, target, secrets, stop, &state, &job_id, &cleanup,
            )
            .await;
            state.send_replace(AssistantVapiStatus::Stopped {
                problem: result.as_ref().err().map(|e| e.code.clone()),
            });
            result
        });
        jobs.insert(
            id,
            Job {
                state: rx,
                task,
                cleanup_unconfirmed,
            },
        );
        Ok(())
    }
    fn cancel(&self) {
        self.stop.cancel();
    }
    async fn drain(&self) -> Result<()> {
        self.stop.cancel();
        let _held = self.draining.lock().await;
        let mut jobs = std::mem::take(&mut *self.jobs.lock().map_err(|_| Error::storage())?);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut failed = self.cleanup_unconfirmed.load(Ordering::Acquire);
        let mut problems = Vec::new();
        while let Some((id, mut job)) = jobs.pop_first() {
            match tokio::time::timeout_at(deadline, &mut job.task).await {
                Ok(result) => {
                    let unconfirmed =
                        result.is_err() || job.cleanup_unconfirmed.load(Ordering::Acquire);
                    failed |= unconfirmed;
                    if unconfirmed {
                        problems.push(
                            match &result {
                                Ok(Err(error)) => error.code.as_str(),
                                Err(_) => "public-assistant-job-failed",
                                _ => "public-provider-unconfirmed",
                            }
                            .to_owned(),
                        );
                    }
                }
                Err(_) => {
                    jobs.insert(id, job);
                    self.jobs.lock().map_err(|_| Error::storage())?.extend(jobs);
                    return Err(Error::new(
                        "timeout",
                        "Public assistant drain remains pending",
                    ));
                }
            }
        }
        if failed {
            self.cleanup_unconfirmed.store(true, Ordering::Release);
            return Err(Error::new(
                "public-provider-unconfirmed",
                &format!(
                    "Public assistant cleanup remains unconfirmed: {}",
                    problems.join(", ")
                ),
            ));
        }
        Ok(())
    }
}
fn key(cid: &str, sid: &str, identity: &str, epoch: u64) -> String {
    let bytes =
        serde_json::to_vec(&(cid, sid, identity, epoch)).expect("string tuple serialization");
    format!("public_ai_{}", hex::encode(Sha256::digest(bytes)))
}
async fn selection(
    service: &Arc<Service>,
    target: &PublicVapiTarget,
    credential: &str,
    connection: &str,
    stop: &CancellationToken,
) -> Result<AssistantContextRead> {
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if stop.is_cancelled() {
            return Err(Error::new(
                "cancelled",
                "Public assistant initialization cancelled",
            ));
        }
        let service = service.clone();
        let endpoint = target.provider_endpoint.clone();
        let routing = target.routing.clone();
        let reachability = target.reachability;
        let credential = credential.to_owned();
        let connection = connection.to_owned();
        match blocking(move || {
            service.try_routed_assistant_context_read(
                &credential,
                &connection,
                1,
                &endpoint,
                &routing,
                reachability,
            )
        })
        .await
        {
            Ok(read) => return Ok(read),
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "authority-busy" | "storage-unavailable" | "not-ready"
                ) && tokio::time::Instant::now() < until =>
            {
                tokio::select! {_=stop.cancelled()=>return Err(Error::new("cancelled","Public assistant initialization cancelled")),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
            }
            Err(e) => return Err(e),
        }
    }
}
async fn run_job(
    service: Arc<Service>,
    invocation: SessionInitialization,
    target: Arc<PublicVapiTarget>,
    secrets: ProviderSecrets,
    stop: CancellationToken,
    state: &watch::Sender<AssistantVapiStatus>,
    id: &str,
    cleanup: &AtomicBool,
) -> Result<()> {
    let connection = format!("{id}_conn");
    let scope = CommandScope {
        conversation_id: Some(invocation.request.conversation_id),
        session_id: Some(invocation.request.session_id),
        connection_id: Some(connection.clone()),
        operation_id: Some(format!("{id}_offer")),
        expected_revision: Some(0),
    };
    let offer = json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":format!("{id}_voice"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let prepare = async {
        let client = CanonicalClient::open(target.native_peer.clone())
            .await
            .map_err(|_| {
                Error::new(
                    "public-native-peer-failed",
                    "Configured public assistant peer failed",
                )
            })?;
        client
            .prepare_public_native_audio(scope, offer)
            .await
            .map_err(|_| {
                Error::new(
                    "public-native-peer-failed",
                    "Configured public assistant audio failed",
                )
            })
    };
    let mut audio =
        tokio::select! {biased;_=stop.cancelled()=>return Ok(()),result=prepare=>result?};
    let prepared = async {
        let selected = selection(
            &service,
            &target,
            &invocation.credential_id,
            &connection,
            &stop,
        )
        .await?;
        let own = selected.target().connection.participant_id.clone();
        let subscriptions = selected
            .audience()
            .iter()
            .filter(|(_, pid, _)| pid != &own)
            .map(|(_, pid, _)| {
                serde_json::from_value(json!({"from_participant":pid,"kinds":["audio"]}))
                    .map_err(|_| Error::storage())
            })
            .collect::<Result<Vec<_>>>()?;
        drop(selected); // No media/context reader may cover a stream mutation.
        if !subscriptions.is_empty() {
            let revision = audio.stream_revision();
            audio
                .subscribe_audio(&format!("{id}_hear"), revision, subscriptions)
                .await
                .map_err(|_| {
                    Error::new(
                        "public-native-subscription-failed",
                        "Configured public assistant subscription failed",
                    )
                })?;
        }
        selection(
            &service,
            &target,
            &invocation.credential_id,
            &connection,
            &stop,
        )
        .await
    }
    .await;
    let selected = match prepared {
        Ok(s) => s,
        Err(e) => {
            audio.close();
            if audio.shutdown().await.is_err() {
                cleanup.store(true, Ordering::Release);
            }
            return Err(e);
        }
    };
    if stop.is_cancelled() {
        drop(selected);
        audio.close();
        if audio.shutdown().await.is_err() {
            cleanup.store(true, Ordering::Release);
        }
        return Ok(());
    }
    let mut job = match AssistantVapiContextSession::start_with_audio(
        service,
        format!("{id}_context"),
        selected,
        target.profile.clone(),
        secrets,
        audio,
    )
    .await
    {
        Ok(job) => job,
        Err(error) => {
            if error.code == "public-native-drain-unconfirmed" {
                cleanup.store(true, Ordering::Release);
            }
            return Err(error);
        }
    };
    let ready =
        tokio::select! {biased;_=stop.cancelled()=>None,result=job.context_ready()=>Some(result)};
    if let Some(ready) = ready {
        if let Err(error) = ready {
            let (_, confirmed) = job.finish().await;
            cleanup.store(!confirmed, Ordering::Release);
            return Err(error);
        }
        state.send_replace(AssistantVapiStatus::ContextReady);
        loop {
            if matches!(job.status(), AssistantVapiStatus::Stopped { .. }) {
                break;
            }
            tokio::select! {biased;_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(20))=>{}}
        }
    }
    let (result, confirmed) = job.finish().await;
    cleanup.store(!confirmed, Ordering::Release);
    result
}
