//! Owned canonical Session host for normalized provider telephone control.
//! Physical media/bridge admission remains with the corresponding real adapter.
use connect_connector_api::{
    manifest::{ConnectorOperation, Manifest},
    VoiceConnector,
};
use connect_protocol::{events::ProviderCallState, session_control::Action, Error, Result};
use connect_service::{Service, SessionInitialization, SessionInitializer};
use connect_store::{
    ProviderSessionVoiceAction as ActionKind, ProviderSessionVoiceDispatch as Dispatch,
    ProviderSessionVoiceOrigin as Origin, ProviderSessionVoicePolicy as Policy,
    ProviderSessionVoiceWorker as Role, ProviderVoiceIngressPolicy, Store,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const NAME: &str = "provider-voice";
mod incoming;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalProviderVoiceConfig {
    #[serde(default)]
    pub routes: Vec<Policy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ingress: Vec<ProviderVoiceIngressPolicy>,
}
impl CanonicalProviderVoiceConfig {
    pub(crate) fn validate(&self, capacity: usize) -> anyhow::Result<()> {
        anyhow::ensure!(
            (!self.routes.is_empty() || !self.ingress.is_empty())
                && self.routes.len() <= capacity
                && self.ingress.len() <= capacity,
            "Invalid provider voice route capacity"
        );
        for policy in self
            .routes
            .iter()
            .chain(self.ingress.iter().map(|p| &p.policy))
        {
            policy.validate()?;
            anyhow::ensure!(
                policy.provider == "vapi-telephone",
                "No constructed provider driver for this route"
            );
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct ProviderVoiceRoute {
    pub policy: Policy,
    pub driver: Arc<dyn VoiceConnector>,
    pub automatic_outbound: bool,
}
struct Profile {
    policy: Policy,
    driver: Arc<dyn VoiceConnector>,
    manifest: Manifest,
    speaker: String,
    target: String,
    automatic_outbound: bool,
}
type Key = (String, String, String);
struct State {
    closed: bool,
    failed: bool,
    role: Option<Arc<Role>>,
    jobs: tokio::task::JoinSet<Result<()>>,
    scans: tokio::task::JoinSet<Result<()>>,
    active: BTreeMap<Key, (String, CancellationToken)>,
    resolutions: tokio::task::JoinSet<Result<()>>,
    resolving: BTreeSet<Key>,
}
struct Host {
    store: Arc<Store>,
    profiles: Vec<Arc<Profile>>,
    stop: CancellationToken,
    state: Mutex<State>,
    draining: tokio::sync::Mutex<()>,
    capacity: usize,
}
struct Active {
    host: Weak<Host>,
    key: Key,
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Some(h) = self.host.upgrade() {
            h.state
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .active
                .remove(&self.key);
        }
    }
}
pub struct Worker {
    host: Arc<Host>,
    finished: bool,
    handle: tokio::runtime::Handle,
}
async fn database<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        Error::new(
            "storage-unavailable",
            "Provider Session metadata job failed",
        )
    })?
}
impl Worker {
    pub(crate) fn start(
        config: &CanonicalProviderVoiceConfig,
        service: &Arc<Service>,
    ) -> anyhow::Result<Self> {
        config.validate(service.store.limits().connections)?;
        let mut drivers: BTreeMap<String, Arc<dyn VoiceConnector>> = BTreeMap::new();
        let mut routes = vec![];
        let mut selected: Vec<(&Policy, bool)> = config.routes.iter().map(|p| (p, true)).collect();
        for ingress in &config.ingress {
            if !selected.iter().any(|(p, _)| **p == ingress.policy) {
                selected.push((&ingress.policy, false));
            }
        }
        for (policy, automatic_outbound) in selected {
            let driver = if let Some(driver) = drivers.get(&policy.route_reference) {
                driver.clone()
            } else {
                let key = crate::secrets::resolve(&policy.route_reference)?;
                let driver: Arc<dyn VoiceConnector> =
                    Arc::new(connect_connectors::Vapi::with_reference(
                        key.expose(),
                        connect_connectors::vapi_api_base()?.as_str(),
                        &policy.route_reference,
                    )?);
                drivers.insert(policy.route_reference.clone(), driver.clone());
                driver
            };
            routes.push(ProviderVoiceRoute {
                policy: policy.clone(),
                driver,
                automatic_outbound,
            });
        }
        Self::with_routes_and_ingress(service, routes, config.ingress.clone()).map_err(Into::into)
    }
    /// Protected composition; adapters receive only attributed provider requests.
    /// Ordinary callers use public Session commands, never this configuration.
    pub fn with_routes(service: &Arc<Service>, routes: Vec<ProviderVoiceRoute>) -> Result<Self> {
        Self::with_routes_and_ingress(service, routes, vec![])
    }
    pub fn with_routes_and_ingress(
        service: &Arc<Service>,
        routes: Vec<ProviderVoiceRoute>,
        ingress: Vec<ProviderVoiceIngressPolicy>,
    ) -> Result<Self> {
        let store = service.store.clone();
        if routes.is_empty() || routes.len() > store.limits().connections {
            return Err(Error::new(
                "capacity-exceeded",
                "Invalid provider voice host capacity",
            ));
        }
        let mut profiles = vec![];
        let mut unique = BTreeSet::new();
        for route in routes {
            let manifest = route.driver.voice_manifest();
            let configuration = route.driver.voice_configuration();
            manifest
                .validate_configuration(&configuration)
                .map_err(|_| {
                    Error::new("invalid-request", "Provider voice configuration is invalid")
                })?;
            if manifest.name != route.policy.provider
                || manifest
                    .operations
                    .contains(&ConnectorOperation::MediaExchange)
                || manifest.media != connect_connector_api::media::MediaSupport::Unsupported
                || configuration.get("private-key")
                    != Some(&format!("secret:{}", route.policy.route_reference))
            {
                return Err(Error::new("unsupported-feature","Provider telephone control requires its actual configured key reference and separate media adapter"));
            }
            let (speaker, target) = store.provider_session_voice_binding(&route.policy)?;
            if !unique.insert((speaker.clone(), target.clone())) {
                return Err(Error::new(
                    "conflict",
                    "Provider voice target has ambiguous configured accounts",
                ));
            }
            profiles.push(Arc::new(Profile {
                policy: route.policy,
                driver: route.driver,
                manifest,
                speaker,
                target,
                automatic_outbound: route.automatic_outbound,
            }));
        }
        if ingress.len() > store.limits().connections {
            return Err(Error::new(
                "capacity-exceeded",
                "Incoming provider profile capacity exhausted",
            ));
        }
        let mut incoming_ids = BTreeSet::new();
        for incoming in &ingress {
            if !incoming_ids.insert(incoming.id.clone()) {
                return Err(Error::new(
                    "conflict",
                    "Duplicate incoming provider binding",
                ));
            }
            let profile = profiles
                .iter()
                .find(|p| p.policy == incoming.policy)
                .ok_or_else(|| {
                    Error::new(
                        "invalid-request",
                        "Incoming provider binding has no configured adapter",
                    )
                })?;
            // Configuration does not invent a capability. A profile without
            // discovery keeps its explicit ingress dormant and still controls
            // original known resources through independent status/end support.
            let _ = profile;
        }
        // Register only after the complete composition validates. Recovery must
        // acquire the same exclusive role retained by old transports/captures.
        for p in &profiles {
            store.register_extension_manifest(&p.manifest)?;
        }
        let role = store.acquire_provider_session_voice_worker()?;
        store.recover_provider_session_voice(&role)?;
        for incoming in &ingress {
            store.configure_provider_voice_ingress(&role, incoming.clone())?;
        }
        let pending = store.pending_provider_session_voice(&role)?;
        let host = Arc::new(Host {
            capacity: store.limits().connections,
            store,
            profiles,
            stop: CancellationToken::new(),
            state: Mutex::new(State {
                closed: false,
                failed: false,
                role: Some(role),
                jobs: tokio::task::JoinSet::new(),
                scans: tokio::task::JoinSet::new(),
                active: BTreeMap::new(),
                resolutions: tokio::task::JoinSet::new(),
                resolving: BTreeSet::new(),
            }),
            draining: tokio::sync::Mutex::new(()),
        });
        for origin in pending {
            if let Some(profile) = host
                .profiles
                .iter()
                .find(|p| &p.policy == origin.policy())
                .cloned()
            {
                host.queue_origin(origin, profile, true)?;
            } else {
                host.state
                    .lock()
                    .map_err(|_| {
                        Error::new("storage-unavailable", "Provider host state unavailable")
                    })?
                    .failed = true;
            }
        }
        let initializer: Arc<dyn SessionInitializer> = Arc::new(Initializer(host.clone()));
        if let Err(error) = service.install_or_replace_session_initializer(NAME, initializer) {
            host.cancel();
            let held = host.clone();
            tokio::spawn(async move {
                let _ = held.drain().await;
            });
            return Err(error);
        }
        for incoming in ingress {
            let profile = host
                .profiles
                .iter()
                .find(|p| p.policy == incoming.policy)
                .cloned()
                .ok_or_else(Error::forbidden)?;
            if profile
                .manifest
                .operations
                .contains(&ConnectorOperation::VoiceDiscover)
            {
                host.queue_scanner(incoming, profile)?;
            }
        }
        Ok(Self {
            host,
            finished: false,
            handle: tokio::runtime::Handle::current(),
        })
    }
    pub async fn shutdown(mut self) -> Result<()> {
        let result = self.host.drain().await;
        self.finished = true;
        result
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if !self.finished {
            self.host.cancel();
            let host = self.host.clone();
            self.handle.spawn(async move {
                let _ = host.drain().await;
            });
        }
    }
}
impl Host {
    fn reap(state: &mut State) {
        while let Some(result) = state.jobs.try_join_next() {
            state.failed |= !matches!(result, Ok(Ok(())));
        }
        while let Some(result) = state.scans.try_join_next() {
            state.failed |= !matches!(result, Ok(Ok(())));
        }
        while let Some(result) = state.resolutions.try_join_next() {
            state.failed |= !matches!(result, Ok(Ok(())));
        }
    }
    fn queue_scanner(
        self: &Arc<Self>,
        ingress: ProviderVoiceIngressPolicy,
        profile: Arc<Profile>,
    ) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| Error::storage())?;
        if state.closed {
            return Err(Error::forbidden());
        }
        let role = state.role.as_ref().cloned().ok_or_else(Error::forbidden)?;
        let store = self.store.clone();
        let weak = Arc::downgrade(self);
        let stop = self.stop.child_token();
        state
            .scans
            .spawn(async move { scan_loop(weak, store, role, ingress, profile, stop).await });
        Ok(())
    }
    fn queue_origin(
        self: &Arc<Self>,
        origin: Origin,
        profile: Arc<Profile>,
        recovered: bool,
    ) -> Result<()> {
        let key = (
            profile.speaker.clone(),
            origin.operation_id().into(),
            origin.participant_id().into(),
        );
        self.queue(
            key,
            origin.session_id().into(),
            move |store, role, cancel| async move {
                run(store, role, origin, profile, cancel, recovered).await
            },
        )
    }
    fn queue<F, Fut>(self: &Arc<Self>, key: Key, sid: String, build: F) -> Result<()>
    where
        F: FnOnce(Arc<Store>, Arc<Role>, CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<()>> + Send + 'static,
    {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("storage-unavailable", "Provider host state unavailable"))?;
        Self::reap(&mut state);
        if state.closed || self.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if state.active.contains_key(&key) {
            return Ok(());
        }
        if state.active.len() >= self.capacity {
            return Err(Error::new(
                "capacity-exceeded",
                "Provider voice job capacity exhausted",
            ));
        }
        let role = state.role.as_ref().cloned().ok_or_else(Error::forbidden)?;
        let cancel = self.stop.child_token();
        state.active.insert(key.clone(), (sid, cancel.clone()));
        let active = Active {
            host: Arc::downgrade(self),
            key,
        };
        let store = self.store.clone();
        state.jobs.spawn(async move{let _active=active;let result=build(store,role,cancel).await;if let Err(error)=&result{tracing::debug!(target:"connect_server::canonical_provider_voice",code=%error.code,"Provider Session job retained unresolved work");}result});
        Ok(())
    }
}
impl Host {
    fn cancel(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        self.stop.cancel();
    }
    async fn drain(&self) -> Result<()> {
        self.cancel();
        let _serial = self.draining.lock().await;
        let (mut jobs, mut scans, mut resolutions, mut failed) = {
            let mut state = self.state.lock().map_err(|_| {
                Error::new("storage-unavailable", "Provider host state unavailable")
            })?;
            (
                std::mem::replace(&mut state.jobs, tokio::task::JoinSet::new()),
                std::mem::replace(&mut state.scans, tokio::task::JoinSet::new()),
                std::mem::replace(&mut state.resolutions, tokio::task::JoinSet::new()),
                state.failed,
            )
        };
        while let Some(result) = scans.join_next().await {
            failed |= !matches!(result, Ok(Ok(())));
        }
        while let Some(result) = resolutions.join_next().await {
            failed |= !matches!(result, Ok(Ok(())));
        }
        while let Some(result) = jobs.join_next().await {
            failed |= !matches!(result, Ok(Ok(())));
        }
        let role = {
            let mut state = self.state.lock().map_err(|_| {
                Error::new("storage-unavailable", "Provider host state unavailable")
            })?;
            state.failed |= failed;
            state.role.take()
        };
        if let Some(role) = role {
            role.retire();
        }
        if failed {
            Err(Error::new(
                "reconciliation-required",
                "Provider Session drain includes unresolved effects",
            ))
        } else {
            Ok(())
        }
    }
    fn replacement_ready(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed
            && state.role.is_none()
            && state.jobs.is_empty()
            && state.scans.is_empty()
            && state.resolutions.is_empty()
    }
}
struct Initializer(Arc<Host>);
#[async_trait::async_trait]
impl SessionInitializer for Initializer {
    async fn resolve_incoming_voice(
        &self,
        credential: String,
        selection: connect_protocol::incoming_contacts::IncomingContactSelection,
    ) -> Option<Result<connect_protocol::incoming_contacts::IncomingContact>> {
        Some(self.0.resolve_incoming(credential, selection).await)
    }
    async fn initialize(&self, invocation: SessionInitialization) -> Result<()> {
        let host = &self.0;
        if matches!(
            invocation.request.action,
            Action::End { .. } | Action::Cancel { .. }
        ) {
            let state = host.state.lock().map_err(|_| {
                Error::new("storage-unavailable", "Provider host state unavailable")
            })?;
            for (sid, cancel) in state.active.values() {
                if sid == &invocation.request.session_id {
                    cancel.cancel();
                }
            }
            return Ok(());
        }
        if !matches!(invocation.request.action, Action::Invite { .. }) {
            return Ok(());
        }
        if host.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        for profile in &host.profiles {
            if !profile.automatic_outbound {
                continue;
            }
            let store = host.store.clone();
            let request = invocation.request.clone();
            let credential = invocation.credential_id.clone();
            let speaker = profile.speaker.clone();
            let target = profile.target.clone();
            let Some(pid) = database(move || {
                store.provider_session_voice_invited_target(
                    &credential,
                    &request,
                    &speaker,
                    &target,
                )
            })
            .await?
            else {
                continue;
            };
            let key = (
                profile.speaker.clone(),
                invocation.request.operation_id.clone(),
                pid.clone(),
            );
            let sid = invocation.request.session_id.clone();
            let profile = profile.clone();
            let invocation = invocation.clone();
            host.queue(key,sid,move|store,role,cancel|async move{
                let prepare_store=store.clone();let policy=profile.policy.clone();
                let origin=match database(move||prepare_store.prepare_provider_session_voice(&invocation.credential_id,invocation.request,&pid,policy)).await {Ok(o)=>o,Err(error)=>{tracing::debug!(target:"connect_server::canonical_provider_voice",code=%error.code,"Provider invitation refused before dispatch");return Ok(());}};
                run(store,role,origin,profile,cancel,false).await
            })?;
        }
        Ok(())
    }
    fn cancel(&self) {
        self.0.cancel();
    }
    async fn drain(&self) -> Result<()> {
        self.0.drain().await
    }
    fn replacement_ready(&self) -> bool {
        self.0.replacement_ready()
    }
}
async fn begin(
    store: Arc<Store>,
    role: Arc<Role>,
    origin: Origin,
    profile: Arc<Profile>,
    action: ActionKind,
) -> Result<Option<Dispatch>> {
    database(move || store.begin_provider_session_voice(&role, &origin, action, &profile.manifest))
        .await
}
async fn observed(
    store: Arc<Store>,
    role: Arc<Role>,
    origin: Origin,
) -> Result<connect_store::ProviderSessionVoiceView> {
    database(move || store.provider_session_voice_observed(&role, &origin)).await
}
async fn attempt(
    store: Arc<Store>,
    dispatch: Dispatch,
    profile: Arc<Profile>,
    cancel: CancellationToken,
    creating: bool,
) -> Result<connect_store::ProviderSessionVoiceView> {
    let remaining = dispatch
        .expires_at()
        .saturating_sub(chrono::Utc::now().timestamp());
    let result = if remaining <= 0 || !dispatch.current() {
        Err(connect_connector_api::Failure::Rejected)
    } else {
        let request = async {
            if !dispatch.current() {
                return Err(connect_connector_api::Failure::Rejected);
            }
            match dispatch.action() {
                ActionKind::Start => {
                    let request = dispatch
                        .start_request()
                        .map_err(|_| connect_connector_api::Failure::Rejected)?;
                    profile
                        .driver
                        .start_receipt(dispatch.mutation_id(), &request)
                        .await
                }
                ActionKind::Status => {
                    let (id, _) = dispatch
                        .existing_resource()
                        .map_err(|_| connect_connector_api::Failure::Rejected)?;
                    profile
                        .driver
                        .status_receipt(dispatch.mutation_id(), id)
                        .await
                }
                ActionKind::Reconcile | ActionKind::ResolveQuarantine => {
                    let query = dispatch
                        .reconcile_query()
                        .map_err(|_| connect_connector_api::Failure::Rejected)?;
                    profile
                        .driver
                        .reconcile_receipt(dispatch.mutation_id(), &query)
                        .await
                }
                ActionKind::End => {
                    let (id, control) = dispatch
                        .existing_resource()
                        .map_err(|_| connect_connector_api::Failure::Rejected)?;
                    profile
                        .driver
                        .end_receipt(dispatch.mutation_id(), id, control)
                        .await
                }
            }
        };
        if creating {
            tokio::select! {biased;_ =cancel.cancelled()=>Err(connect_connector_api::Failure::Uncertain),result=tokio::time::timeout(Duration::from_secs(remaining.min(2) as u64),request)=>result.unwrap_or(Err(connect_connector_api::Failure::Uncertain))}
        } else {
            tokio::time::timeout(Duration::from_secs(remaining.min(2) as u64), request)
                .await
                .unwrap_or(Err(connect_connector_api::Failure::Uncertain))
        }
    };
    let capture = dispatch.capture(result);
    database(move || store.record_provider_session_voice(&capture)).await
}
async fn refresh(
    store: Arc<Store>,
    role: Arc<Role>,
    origin: Origin,
    profile: Arc<Profile>,
) -> Result<connect_store::ProviderSessionVoiceView> {
    let dispatch = match begin(
        store.clone(),
        role.clone(),
        origin.clone(),
        profile.clone(),
        ActionKind::Status,
    )
    .await
    {
        Err(error)
            if error.code == "reconciliation-required"
                && profile
                    .manifest
                    .operations
                    .contains(&ConnectorOperation::VoiceReconcile) =>
        {
            match begin(
                store.clone(),
                role.clone(),
                origin.clone(),
                profile.clone(),
                ActionKind::Reconcile,
            )
            .await
            {
                Err(error) if error.code == "reconciliation-required" => {
                    begin(
                        store.clone(),
                        role.clone(),
                        origin.clone(),
                        profile.clone(),
                        ActionKind::ResolveQuarantine,
                    )
                    .await?
                }
                other => other?,
            }
        }
        other => other?,
    };
    if let Some(dispatch) = dispatch {
        attempt(store, dispatch, profile, CancellationToken::new(), false).await
    } else {
        observed(store, role, origin).await
    }
}
async fn cleanup(
    store: Arc<Store>,
    role: Arc<Role>,
    origin: Origin,
    profile: Arc<Profile>,
) -> Result<()> {
    let view = observed(store.clone(), role.clone(), origin.clone()).await?;
    if view.closed_at.is_some() {
        return Ok(());
    }
    let dispatch = match begin(
        store.clone(),
        role.clone(),
        origin.clone(),
        profile.clone(),
        ActionKind::End,
    )
    .await
    {
        Err(error)
            if error.code == "reconciliation-required"
                && profile
                    .manifest
                    .operations
                    .contains(&ConnectorOperation::VoiceReconcile) =>
        {
            let current =
                refresh(store.clone(), role.clone(), origin.clone(), profile.clone()).await?;
            if current.closed_at.is_some() {
                return Ok(());
            }
            begin(
                store.clone(),
                role.clone(),
                origin.clone(),
                profile.clone(),
                ActionKind::End,
            )
            .await?
        }
        other => other?,
    };
    if let Some(dispatch) = dispatch {
        attempt(
            store.clone(),
            dispatch,
            profile.clone(),
            CancellationToken::new(),
            false,
        )
        .await?;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let view = observed(store.clone(), role.clone(), origin.clone()).await?;
        if view.closed_at.is_some() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::new(
                "reconciliation-required",
                "Provider cleanup is not terminal",
            ));
        }
        if let Some(dispatch) = begin(
            store.clone(),
            role.clone(),
            origin.clone(),
            profile.clone(),
            ActionKind::Status,
        )
        .await?
        {
            attempt(
                store.clone(),
                dispatch,
                profile.clone(),
                CancellationToken::new(),
                false,
            )
            .await?;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
async fn run(
    store: Arc<Store>,
    role: Arc<Role>,
    origin: Origin,
    profile: Arc<Profile>,
    cancel: CancellationToken,
    _recovered: bool,
) -> Result<()> {
    let s = store.clone();
    let r = role.clone();
    let o = origin.clone();
    let (started, ending) = database(move || s.provider_session_voice_started(&r, &o)).await?;
    if ending {
        return cleanup(store, role, origin, profile).await;
    }
    // Both fresh and recovered intents keep their original binding/deadline.
    if !started {
        if cancel.is_cancelled() {
            let s = store.clone();
            let o = origin.clone();
            database(move || s.refuse_provider_session_voice_unstarted(&o, "cancelled")).await?;
            return Ok(());
        }
        match begin(
            store.clone(),
            role.clone(),
            origin.clone(),
            profile.clone(),
            ActionKind::Start,
        )
        .await
        {
            Ok(Some(dispatch)) => {
                attempt(
                    store.clone(),
                    dispatch,
                    profile.clone(),
                    cancel.clone(),
                    true,
                )
                .await?;
            }
            Ok(None) => (),
            Err(error) => {
                let code = if matches!(
                    error.code.as_str(),
                    "unsupported-connector-operation"
                        | "unsupported-feature"
                        | "unauthorized"
                        | "forbidden"
                ) {
                    error.code.clone()
                } else {
                    "not-started".into()
                };
                let s = store.clone();
                let o = origin.clone();
                if database(move || s.refuse_provider_session_voice_unstarted(&o, &code)).await? {
                    return Ok(());
                }
                return Err(error);
            }
        }
    }
    let mut established =
        origin.direction() == connect_protocol::events::ProviderCallDirection::Inbound;
    let mut next_status = tokio::time::Instant::now();
    loop {
        let view = observed(store.clone(), role.clone(), origin.clone()).await?;
        if view.closed_at.is_some() {
            return Ok(());
        }
        established |= view.state == ProviderCallState::Active;
        if cancel.is_cancelled()
            || (!established && chrono::Utc::now().timestamp() >= origin.expires_at())
        {
            return cleanup(store, role, origin, profile).await;
        }
        let s = store.clone();
        let o = origin.clone();
        match database(move || s.provider_session_voice_current(&o, established)).await {
            Ok(true) => (),
            Ok(false) => return cleanup(store, role, origin, profile).await,
            Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {
                return cleanup(store, role, origin, profile).await
            }
            Err(_) => {
                tokio::select! {_ =cancel.cancelled()=>(),_ =tokio::time::sleep(Duration::from_millis(100))=>()};
                continue;
            }
        }
        if tokio::time::Instant::now() >= next_status {
            refresh(store.clone(), role.clone(), origin.clone(), profile.clone()).await?;
            next_status = tokio::time::Instant::now() + Duration::from_secs(2);
        }
        tokio::select! {_ =cancel.cancelled()=>(),_ =tokio::time::sleep(Duration::from_millis(100))=>()}
    }
}

async fn scan_loop(
    host: Weak<Host>,
    store: Arc<Store>,
    role: Arc<Role>,
    ingress: ProviderVoiceIngressPolicy,
    profile: Arc<Profile>,
    stop: CancellationToken,
) -> Result<()> {
    loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        let s = store.clone();
        let r = role.clone();
        let p = ingress.clone();
        let manifest = profile.manifest.clone();
        let scan = match database(move || s.begin_provider_voice_scan(&r, &p, &manifest)).await {
            Ok(scan) => Some(scan),
            Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => {
                return Ok(())
            }
            Err(error) => {
                tracing::debug!(code=%error.code,"Provider incoming scan refused");
                None
            }
        };
        if let Some(scan) = scan {
            let Ok(query) = scan.query() else {
                return Ok(());
            };
            let remaining = scan
                .expires_at()
                .saturating_sub(chrono::Utc::now().timestamp());
            if remaining > 0 {
                let receipts = tokio::select! {biased;_=stop.cancelled()=>return Ok(()), result=tokio::time::timeout(Duration::from_secs(remaining as u64),profile.driver.inbound_receipts(&query))=>result};
                if !stop.is_cancelled() {
                    if let Ok(Ok(receipts)) = receipts {
                        match scan.capture(receipts) {
                            Ok(captured) => {
                                let s = store.clone();
                                match database(move || s.record_provider_voice_scan(&captured))
                                    .await
                                {
                                    Ok(batch) => {
                                        for origin in batch.calls {
                                            let queued = host.upgrade().map(|h| {
                                                h.queue_origin(
                                                    origin.clone(),
                                                    profile.clone(),
                                                    true,
                                                )
                                            });
                                            if !matches!(queued, Some(Ok(()))) {
                                                cleanup(
                                                    store.clone(),
                                                    role.clone(),
                                                    origin,
                                                    profile.clone(),
                                                )
                                                .await?;
                                            }
                                        }
                                    }
                                    Err(error)
                                        if matches!(
                                            error.code.as_str(),
                                            "forbidden" | "unauthorized"
                                        ) =>
                                    {
                                        return Ok(())
                                    }
                                    Err(error) => {
                                        tracing::debug!(code=%error.code,"Provider incoming batch retained its original cursor")
                                    }
                                }
                            }
                            Err(error) => {
                                tracing::debug!(code=%error.code,"Provider incoming receipts refused")
                            }
                        }
                    }
                }
            }
        }
        tokio::select! {biased;_=stop.cancelled()=>return Ok(()),_=tokio::time::sleep(Duration::from_secs(2))=>()}
    }
}
