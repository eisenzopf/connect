//! Own configured telephone agent, using the original canonical Session invite.
pub(crate) mod policy;
mod write;
use crate::{canonical_runtime::Runtime, Service, SessionInitialization, SessionInitializer};
use connect_protocol::{session_control::Action, Error, Result};
use connect_store::InitialTelephonePolicy;
use rvoip_core::{
    adapter::*, ids::*, orchestrator::participant_control::*, participant_transitions::*, Transport,
};
use rvoip_sip::{SipAdapter, SipInitialCallObservation, SipOriginateContext};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub struct InitialTelephoneRoute {
    pub policy: InitialTelephonePolicy,
    pub originate: SipOriginateContext,
}
impl std::fmt::Debug for InitialTelephoneRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InitialTelephoneRoute(redacted)")
    }
}
struct Profile {
    identity: String,
    route: InitialTelephoneRoute,
}
struct State {
    closed: bool,
    unconfirmed: bool,
    jobs: tokio::task::JoinSet<Result<()>>,
    active: BTreeMap<String, (String, CancellationToken)>,
}
struct Host {
    runtime: Weak<Runtime>,
    adapter: Arc<SipAdapter>,
    profiles: Vec<Profile>,
    gateways: Arc<connect_store::InitialTelephoneGateways>,
    stop: CancellationToken,
    state: Mutex<State>,
    capacity: usize,
}
struct Active {
    host: Weak<Host>,
    key: String,
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
impl Service {
    pub fn install_initial_telephone_host(
        &self,
        adapter: Arc<SipAdapter>,
        routes: Vec<InitialTelephoneRoute>,
    ) -> Result<()> {
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let closed = runtime
            .host_configuration_closed
            .lock()
            .map_err(|_| Error::storage())?;
        if *closed
            || runtime.stop.is_cancelled()
            || routes.is_empty()
            || routes.len() > self.store.limits().connections
        {
            return Err(Error::forbidden());
        }
        let mut profiles = vec![];
        let mut identities = std::collections::BTreeSet::new();
        for route in routes {
            route.policy.validate()?;
            route.originate.validate().map_err(|_| Error::forbidden())?;
            let actor = self
                .store
                .revalidate(&route.policy.endpoint_credential_id)?;
            actor.require("connection.offer")?;
            actor.require("session.accept")?;
            self.store
                .revalidate(&route.policy.route_credential_id)?
                .require("sip_dial")?;
            if !identities.insert(actor.identity_id.clone()) {
                return Err(Error::new(
                    "conflict",
                    "Initial telephone target has ambiguous configured Endpoints",
                ));
            }
            profiles.push(Profile {
                identity: actor.identity_id,
                route,
            });
        }
        let mut installed = runtime.telephone.lock().map_err(|_| Error::storage())?;
        if let Some(old) = installed.as_ref() {
            if !Arc::ptr_eq(&old.adapter, &adapter) {
                return Err(Error::new(
                    "conflict",
                    "Canonical telephone adapter differs",
                ));
            }
        } else {
            runtime
                .core
                .register(adapter.clone())
                .map_err(|_| Error::new("conflict", "Canonical SIP registration failed"))?;
            *installed = Some(Arc::new(crate::telephone_replacement::TelephoneRuntime {
                adapter: adapter.clone(),
                routes: BTreeMap::new(),
            }));
        }
        let gateways = self.store.install_initial_telephone_gateways(
            profiles
                .iter()
                .map(|profile| profile.route.policy.clone())
                .collect(),
        )?;
        let host = Arc::new(Host {
            runtime: Arc::downgrade(runtime),
            adapter,
            profiles,
            gateways,
            stop: runtime.stop.child_token(),
            state: Mutex::new(State {
                closed: false,
                unconfirmed: false,
                jobs: tokio::task::JoinSet::new(),
                active: BTreeMap::new(),
            }),
            capacity: self.store.limits().connections,
        });
        self.install_session_initializer("initial-telephone", Arc::new(Initializer(host)))
    }
}
struct Initializer(Arc<Host>);
#[async_trait::async_trait]
impl SessionInitializer for Initializer {
    async fn initialize(&self, invocation: SessionInitialization) -> Result<()> {
        let host = &self.0;
        let runtime = host.runtime.upgrade().ok_or_else(Error::forbidden)?;
        if matches!(
            invocation.request.action,
            Action::End { .. } | Action::Cancel { .. }
        ) {
            let state = host.state.lock().map_err(|_| Error::storage())?;
            for (sid, cancel) in state.active.values() {
                if sid == &invocation.request.session_id {
                    cancel.cancel();
                }
            }
            return Ok(());
        }
        let Action::Invite { .. } = &invocation.request.action else {
            return Ok(());
        };
        let members = runtime
            .media
            .store
            .initial_telephone_invitees(&invocation.credential_id, &invocation.request)?;
        for profile in &host.profiles {
            let Some(member) = members.iter().find(|m| m.identity_id == profile.identity) else {
                continue;
            };
            let key = format!(
                "{}:{}:{}",
                invocation.credential_id, invocation.request.operation_id, member.participant_id
            );
            let mut state = host.state.lock().map_err(|_| Error::storage())?;
            if state.closed || host.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            while let Some(result) = state.jobs.try_join_next() {
                state.unconfirmed |= !matches!(result, Ok(Ok(())));
            }
            if state.active.contains_key(&key) {
                continue;
            }
            if state.active.len() >= host.capacity {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Initial telephone job capacity exhausted",
                ));
            }
            let cancel = host.stop.child_token();
            state.active.insert(
                key.clone(),
                (invocation.request.session_id.clone(), cancel.clone()),
            );
            let active = Active {
                host: Arc::downgrade(host),
                key,
            };
            let route = profile.route.clone();
            let request = invocation.clone();
            let target = member.participant_id.clone();
            let runner = Runner {
                runtime: host.runtime.clone(),
                adapter: host.adapter.clone(),
            };
            state.jobs.spawn(async move {let _active=active;let result=runner.run(request,target,route,cancel).await;if let Err(error)=&result{tracing::debug!(target:"connect_service::initial_telephone",code=%error.code,"Initial telephone job stopped");}result});
        }
        Ok(())
    }
    fn cancel(&self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.closed = true;
        self.0.gateways.close();
        self.0.stop.cancel();
    }
    async fn drain(&self) -> Result<()> {
        self.cancel();
        let (mut jobs, mut failed) = {
            let mut state = self.0.state.lock().map_err(|_| Error::storage())?;
            (
                std::mem::replace(&mut state.jobs, tokio::task::JoinSet::new()),
                state.unconfirmed,
            )
        };
        while let Some(result) = jobs.join_next().await {
            failed |= !matches!(result, Ok(Ok(())));
        }
        self.0
            .state
            .lock()
            .map_err(|_| Error::storage())?
            .unconfirmed |= failed;
        if failed {
            Err(Error::new(
                "reconciliation-required",
                "Initial telephone drain includes unconfirmed work",
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone)]
struct Runner {
    runtime: Weak<Runtime>,
    adapter: Arc<SipAdapter>,
}
impl Runner {
    async fn run(
        &self,
        invocation: SessionInitialization,
        target_pid: String,
        route: InitialTelephoneRoute,
        cancel: CancellationToken,
    ) -> Result<()> {
        let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
        let store = runtime.media.store.clone();
        let credential = invocation.credential_id.clone();
        let request = invocation.request.clone();
        let selected = route.policy.clone();
        let origin = match crate::canonical_consultation::store_call(move || {
            store.prepare_initial_telephone(&credential, request, &target_pid, selected)
        })
        .await
        {
            Ok(origin) => origin,
            Err(error) => {
                // This notification has not allocated any physical resource.
                // Refusal is distinct from uncertainty about an owned call.
                tracing::debug!(target:"connect_service::initial_telephone",code=%error.code,"Initial telephone invitation refused before allocation");
                return Ok(());
            }
        };
        let result = self
            .run_allocated(&runtime, origin.clone(), route, cancel)
            .await;
        if result.is_err() {
            let store = runtime.media.store.clone();
            let original = origin.clone();
            let _ = crate::canonical_consultation::store_call(move || {
                store.abandon_initial_telephone_unstarted(&original)
            })
            .await;
        }
        result
    }
    async fn run_allocated(
        &self,
        runtime: &Arc<Runtime>,
        origin: connect_store::InitialTelephoneOrigin,
        route: InitialTelephoneRoute,
        cancel: CancellationToken,
    ) -> Result<()> {
        // Keep every fallible authority check before Core allocation in one
        // phase. A terminal replay or withdrawn grant owns no new call and
        // must not poison the host's physical-resource drain.
        let allocation: Result<_> = async {
            let store = runtime.media.store.clone();
            let start = origin.clone();
            let Some(attempt) = crate::canonical_consultation::store_call(move || {
                store.begin_initial_telephone_allocation(&start)
            })
            .await?
            else {
                return Ok(None);
            };
            let attempt = Arc::new(attempt);
            let read = runtime
                .media
                .store
                .initial_telephone_allocation_read(&attempt)?;
            let target = policy::target(runtime, &read)?;
            Ok(Some((attempt, target)))
        }
        .await;
        let (attempt, target) = match allocation {
            Ok(Some(prepared)) => prepared,
            Ok(None) => return Ok(()),
            Err(error) => {
                tracing::debug!(target:"connect_service::initial_telephone",code=%error.code,"Initial telephone allocation refused before Core work");
                let store = runtime.media.store.clone();
                let original = origin.clone();
                // Only an unstarted attempt may be retired here. An existing
                // submitted attempt remains untouched and never redials.
                let _ = crate::canonical_consultation::store_call(move || {
                    store.abandon_initial_telephone_unstarted(&original)
                })
                .await;
                return Ok(());
            }
        };
        let sid = SessionId::from_string(origin.source().request.session_id.clone());
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(
                (origin.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
            );
        let writer = Arc::new(write::InviteAuthority {
            runtime: Arc::downgrade(&runtime),
            signal: OnceLock::new(),
            media: OnceLock::new(),
            cancel: cancel.clone(),
            deadline,
        });
        let context = route.originate.with_request_write_authority(writer.clone());
        let prepared = runtime
            .core
            .prepare_outbound_connection(
                OriginateRequest::new(
                    sid.clone(),
                    ParticipantId::from_string(origin.source().target.participant_id.clone()),
                    origin.source().destination.clone(),
                    rvoip_core::connection::Direction::Outbound,
                    rvoip_core::capability::CapabilityDescriptor::default(),
                )
                .with_transport(Transport::Sip)
                .with_context(context),
            )
            .await
            .map_err(|_| Error::forbidden())?;
        let observer = Arc::new(
            self.adapter
                .initial_participant_observation(&prepared)
                .map_err(|_| Error::forbidden())?,
        );
        let store = runtime.media.store.clone();
        let a = attempt.clone();
        let core = prepared.connection_id().as_str().to_owned();
        let call = observer.planned_reference().expose_secret().to_owned();
        crate::canonical_consultation::store_call(move || {
            store.record_initial_telephone_allocation(&a, &core, &call)
        })
        .await?;
        let store = runtime.media.store.clone();
        let Some(signal) = crate::canonical_consultation::store_call(move || {
            store.begin_initial_telephone_signaling(attempt)
        })
        .await?
        else {
            return Ok(());
        };
        let signal = Arc::new(signal);
        writer
            .signal
            .set(signal.clone())
            .map_err(|_| Error::forbidden())?;
        let leg = Arc::new_cyclic(|weak| policy::Leg {
            self_ref: weak.clone(),
            runtime: Arc::downgrade(&runtime),
            signal: signal.clone(),
            target,
            cancel: cancel.clone(),
            deadline,
            prepared: OnceLock::new(),
            media: OnceLock::new(),
            context: OnceLock::new(),
            controller: OnceLock::new(),
            codec: OnceLock::new(),
            binding: OnceLock::new(),
            proof: Mutex::new(None),
            admitted: OnceLock::new(),
            dtmf_registration: OnceLock::new(),
        });
        runtime
            .media
            .register_initial_telephone(&leg)
            .map_err(|_| Error::forbidden())?;
        let result = self
            .call(
                &runtime,
                prepared,
                observer.clone(),
                leg.clone(),
                writer,
                deadline,
                cancel.clone(),
            )
            .await;
        let cleanup = self.adapter.end(leg.connection(), EndReason::Normal).await;
        let _ = runtime
            .core
            .end_connection(leg.connection(), EndReason::Normal)
            .await;
        let absent = Absence {
            observer: observer.clone(),
        };
        let store = runtime.media.store.clone();
        let signal = signal.clone();
        let recorded = crate::canonical_consultation::store_call(move || {
            store.settle_initial_telephone_cleanup(&signal, &absent)
        })
        .await;
        runtime.media.remove_initial_telephone(&leg.connection());
        if cleanup.is_err() && !observer.confirmed_local_absence() {
            return Err(Error::new(
                "reconciliation-required",
                "Initial SIP cleanup unconfirmed",
            ));
        }
        recorded?;
        if let Err(error) = result {
            tracing::debug!(target:"connect_service::initial_telephone",code=%error.code,"Initial telephone call ended before ready");
        }
        Ok(())
    }
    async fn call(
        &self,
        runtime: &Arc<Runtime>,
        prepared: rvoip_core::orchestrator::PreparedOutboundConnection,
        observer: Arc<SipInitialCallObservation>,
        leg: Arc<policy::Leg>,
        writer: Arc<write::InviteAuthority>,
        deadline: tokio::time::Instant,
        cancel: CancellationToken,
    ) -> Result<()> {
        let sid = SessionId::from_string(leg.signal.origin().source().request.session_id.clone());
        let mutation = |revision| ParticipantMutation {
            operation_id: leg.signal.origin().operation_id().into(),
            correlation_id: leg.signal.origin().operation_id().into(),
            expected_revision: revision,
            participant: leg.target.member.participant().clone(),
            change: ParticipantChange::Admit {
                connection: leg.connection(),
                role_override: None,
            },
        };
        let revision = runtime
            .core
            .participant_revision(&sid)
            .map_err(|_| Error::forbidden())?;
        let active = prepared
            .activate_initial_participant_with_held(
                leg.target.principal.clone(),
                mutation(revision),
                deadline,
                cancel.clone(),
                Box::new(policy::DialInvocation {
                    leg: leg.clone(),
                    revision,
                    media: None,
                }),
            )
            .await
            .map_err(|error| { tracing::debug!(target:"connect_service::initial_telephone",?error,"Owned initial activation refused"); Error::forbidden() })?;
        let answer = observer
            .wait_answered(deadline, cancel.clone())
            .await
            .map_err(|_| Error::forbidden())?;
        if answer.connection() != &leg.connection()
            || answer.external_reference().expose_secret() != leg.signal.sip_call_id()
        {
            return Err(Error::forbidden());
        }
        let codec = observer
            .negotiated_audio_codec()
            .map_err(|_| Error::forbidden())?;
        let capture = leg
            .signal
            .capture_answer(answer.external_reference().expose_secret(), &codec.name)?;
        let store = runtime.media.store.clone();
        let recorded = crate::canonical_consultation::store_call(move || {
            store.record_initial_telephone_answer(&capture)
        })
        .await?;
        let origin = recorded.media.ok_or_else(|| {
            Error::new(
                &recorded
                    .problem
                    .unwrap_or_else(|| "authority-unavailable".into()),
                "Observed SIP answer could not be admitted",
            )
        })?;
        leg.media
            .set(origin.clone())
            .map_err(|_| Error::forbidden())?;
        leg.codec.set(codec).map_err(|_| Error::forbidden())?;
        let admission_read = loop {
            match runtime
                .media
                .store
                .try_initial_telephone_media_read(origin.clone())
            {
                Ok(read) => break Arc::new(read),
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "storage-unavailable" | "authority-busy"
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::select! {biased;
                        _=runtime.stop.cancelled()=>return Err(Error::forbidden()),
                        _=leg.cancel.cancelled()=>return Err(Error::forbidden()),
                        _=tokio::time::sleep_until(deadline)=>return Err(error),
                        _=tokio::time::sleep(Duration::from_millis(10))=>{},
                    }
                }
                Err(error) => return Err(error),
            }
        };
        let callback_read = admission_read.clone();
        let revision = runtime
            .core
            .participant_revision(&sid)
            .map_err(|_| Error::forbidden())?;
        let adapter = self.adapter.clone();
        let callback_leg = leg.clone();
        let media_policy = runtime.media.clone();
        let retained_context = Arc::new(Mutex::new(None));
        let context_keeper = retained_context.clone();
        let (_, joined) = active
            .commit_staged_participant_with_held(
                mutation(revision),
                deadline,
                cancel.clone(),
                Box::new(policy::DialInvocation {
                    leg: leg.clone(),
                    revision,
                    media: Some(admission_read),
                }),
                move |p| async move {
                    callback_leg
                        .prepared
                        .set(p.clone())
                        .map_err(|_| ParticipantControlError::Authority)?;
                    let context = adapter
                        .initial_participant_context(p)
                        .map_err(ParticipantControlError::Core)?;
                    callback_leg
                        .context
                        .set(Arc::downgrade(&context))
                        .map_err(|_| ParticipantControlError::Authority)?;
                    if !callback_read.current() {return Err(ParticipantControlError::Authority);}
                    let read=callback_read;
                    let invocation = policy::MediaInvocation {
                        leg: callback_leg.clone(),
                        read,
                        revision,
                    };
                    let controller =
                        context.prepare_media_controller(media_policy, 4, &invocation).map_err(|error| {tracing::debug!(target:"connect_service::initial_telephone",?error,"Initial guarded controller refused");error})?;
                    callback_leg
                        .controller
                        .set(Arc::downgrade(&controller))
                        .map_err(|_| ParticipantControlError::Authority)?;
                    controller
                        .change_with_held(
                            0,
                            &[policy::Leg::desired()],
                            &std::collections::BTreeSet::new(),
                            deadline,
                            cancel.clone(),
                            Arc::new(invocation.clone()),
                        )
                        .await.map_err(|error| {tracing::debug!(target:"connect_service::initial_telephone",?error,"Initial guarded streams refused");error})?;
                    let bindings = controller.active_bindings();
                    if bindings.len() != 1 {
                        return Err(ParticipantControlError::Authority);
                    }
                    callback_leg.binding.set(bindings[0].clone())
                        .map_err(|_| ParticipantControlError::Authority)?;
                    // The outer context must live as long as this managed leg. It is
                    // retained by the job, not by its own controller/policy cycle.
                    *context_keeper
                        .lock()
                        .map_err(|_| ParticipantControlError::Authority)? = Some(context);
                    Ok(Arc::new(invocation) as Arc<dyn std::any::Any + Send + Sync>)
                },
            )
            .await
            .map_err(|error| { tracing::debug!(target:"connect_service::initial_telephone",?error,"Guarded initial admission refused"); Error::forbidden() })?;
        leg.admitted.set(joined).map_err(|_| Error::forbidden())?;
        // Core admission is already observed. Retain this exact owned leg and
        // retry only its durable metadata, never originate/admit a second call.
        loop {
            let store = runtime.media.store.clone();
            let saved = origin.clone();
            let ready = policy::Ready { leg: leg.clone() };
            match crate::canonical_consultation::store_call(move || {
                store.record_initial_telephone_connected(saved, &ready)
            })
            .await
            {
                Ok(_) => break,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "storage-unavailable" | "capacity-exceeded" | "authority-busy"
                    ) && tokio::time::Instant::now() < deadline =>
                {
                    tokio::select! { biased;
                        _=runtime.stop.cancelled()=>return Err(Error::forbidden()),
                        _=leg.cancel.cancelled()=>return Err(Error::forbidden()),
                        _=tokio::time::sleep_until(deadline)=>return Err(error),
                        _=tokio::time::sleep(Duration::from_millis(50))=>{},
                    }
                }
                Err(error) => return Err(error),
            }
        }
        writer
            .media
            .set(origin.clone())
            .map_err(|_| Error::forbidden())?;
        leg.dtmf_registration
            .set(format!("dtmfsip_{}", uuid::Uuid::new_v4().simple()))
            .map_err(|_| Error::forbidden())?;
        let mut received = None;
        let mut projected = std::collections::BTreeSet::new();
        loop {
            if leg.cancel.is_cancelled() || runtime.stop.is_cancelled() {
                return Ok(());
            }
            if !self.adapter.is_connection_live(&leg.connection()) {
                return Ok(());
            }
            match runtime
                .media
                .store
                .try_initial_telephone_media_read(origin.clone())
            {
                Ok(read) if read.current() => {}
                Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => {
                    // Named signal authority is independent of media/codec
                    // permission. Keeping this existing physical call alive
                    // grants no RTP; every packet still uses its media guard.
                    let mut alive = false;
                    let mut busy = false;
                    for direction in [
                        connect_store::InitialTelephoneSignalDirection::FromTelephone,
                        connect_store::InitialTelephoneSignalDirection::ToTelephone,
                    ] {
                        match runtime
                            .media
                            .store
                            .try_initial_telephone_signal_read(origin.clone(), direction)
                        {
                            Ok(read) => alive |= read.current(),
                            Err(e) => {
                                busy |= !matches!(e.code.as_str(), "forbidden" | "unauthorized")
                            }
                        }
                    }
                    if !alive && !busy {
                        tracing::debug!(target:"connect_service::initial_telephone",code=%error.code,"Current initial telephone media and signal authority withdrawn");
                        return Ok(());
                    }
                }
                // Packet guards still refuse while Store/fences are busy. A
                // failed metadata probe is not evidence that a live leg ended.
                Err(_) => {}
                Ok(_) => return Ok(()),
            }
            if let Some(controller) = leg.controller.get().and_then(Weak::upgrade) {
                for source in &leg.signal.origin().source().audience {
                    if source.participant_id == leg.signal.origin().source().target.participant_id
                        || projected.contains(&source.participant_id)
                    {
                        continue;
                    }
                    let before = controller.revision();
                    let result=runtime.core.change_staged_audio_subscriptions(
                        &sid, &leg.connection(), before,
                        rvoip_core::staged_streams::audio_subscriptions::AudioSubscriptionChange::Subscribe(vec![rvoip_core::staged_streams::audio_subscriptions::AudioSubscription::Participant(ParticipantId::from_string(source.participant_id.clone()))]),
                        tokio::time::Instant::now()+Duration::from_secs(2),leg.cancel.clone(),None,
                    ).await;
                    match result {
                        Ok(after) if after == before + 1 => {
                            let store = runtime.media.store.clone();
                            let saved = origin.clone();
                            let participant = source.participant_id.clone();
                            let ready = policy::Ready { leg: leg.clone() };
                            // Reconcile the original fixed receive intent. A failed
                            // metadata settlement cannot trigger another Core write.
                            crate::canonical_consultation::store_call(move || {
                                store.record_initial_telephone_receive_projection(
                                    saved,
                                    &participant,
                                    before,
                                    &ready,
                                )
                            })
                            .await?;
                            projected.insert(source.participant_id.clone());
                        }
                        Err(_) if controller.revision() == before => (),
                        _ => {
                            return Err(Error::new(
                                "reconciliation-required",
                                "Initial receive projection is uncertain",
                            ))
                        }
                    }
                }
            }
            if received.is_none() {
                if let Some(controller) = leg.controller.get().and_then(Weak::upgrade) {
                    match self.adapter.subscribe_managed_dtmf_source(
                        &runtime.core,
                        leg.connection(),
                        &controller,
                        leg.target.principal.clone(),
                    ) {
                        Ok(subscription) => received = Some(subscription),
                        Err(_) => (), // Metadata-only retry; this never redials or grants media.
                    }
                }
            }
            if let Some(subscription) = received.as_mut() {
                tokio::select! {biased;
                    _ = runtime.stop.cancelled() => return Ok(()),
                    _ = leg.cancel.cancelled() => return Ok(()),
                    input = subscription.recv() => match input {
                        Some(observed) => if let Err(error) = runtime.enqueue_received_dtmf(crate::canonical_sip_signal::SipLeg::Initial(leg.clone()), observed) {
                            tracing::debug!(code=%error.code, "Received SIP signal refused before journal admission");
                        },
                        None => received = None,
                    },
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {},
                }
            } else {
                tokio::select! {biased;_ = runtime.stop.cancelled()=>return Ok(()),_ = leg.cancel.cancelled()=>return Ok(()),_ = tokio::time::sleep(Duration::from_millis(25))=>{}}
            }
        }
    }
}
struct Absence {
    observer: Arc<SipInitialCallObservation>,
}
impl connect_store::TelephoneConnectionRuntimeObservation for Absence {
    fn observe_absent(&self, id: &str, generation: u64) -> Result<()> {
        if id == self.observer.connection().as_str()
            && generation == 1
            && self.observer.confirmed_local_absence()
        {
            Ok(())
        } else {
            Err(Error::new(
                "reconciliation-required",
                "Exact SIP cleanup is not confirmed",
            ))
        }
    }
}
