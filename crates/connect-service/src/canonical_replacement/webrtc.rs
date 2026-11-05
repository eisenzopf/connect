//! One RTC replacement owner, from the original signed SDP through adoption.
//! No originating reader survives ICE/DTLS/RTP waits or the active lifetime.
use super::*;
use super::{
    allocation::AllocatedReplacement,
    media::{Activation, StartAuthority, StartInvocation, TargetPolicy},
    start::StartedReplacement,
};
use crate::{
    canonical_native::Reply,
    canonical_runtime::Physical,
    canonical_webrtc::{
        ReplacementAttempt, ReplacementOffer, ReplacementPreparation, ReplacementSource,
    },
    StoreInvocationAuthority,
};
use connect_store::{
    NativeReplacementStartAttempt, NativeReplacementStartOrigin, NativeReplacementStartRead,
};
use rvoip_core::{
    ids::ConversationId,
    staged_streams::StreamController,
    stream_lifecycle::{Medium, StreamDirection, StreamGateAction},
};
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;
use std::{
    collections::BTreeSet,
    sync::{Mutex, OnceLock, Weak},
};
use tokio::time::Instant;

pub(super) enum ReplacementLifecycle {
    Native(crate::canonical_native_lifecycle::PreparedReplacementLifecycle),
    WebRtc(Arc<Source>),
}
impl ReplacementLifecycle {
    pub(super) fn activate(&self, runtime: &Runtime) -> Result<()> {
        match self {
            Self::Native(n) => n.activate(runtime),
            Self::WebRtc(source) => {
                let entry = runtime
                    .media
                    .entry(source.policy.pending.binding().connection())
                    .map_err(|_| Error::forbidden())?;
                if entry.origin.generation() != source.policy.pending.generation() {
                    return Err(Error::forbidden());
                }
                if let Some(previous) = source.adopted.get() {
                    if !Arc::ptr_eq(previous, &entry.origin) {
                        return Err(Error::forbidden());
                    }
                } else {
                    source
                        .adopted
                        .set(entry.origin.clone())
                        .map_err(|_| Error::forbidden())?;
                }
                if let Some(gate) = &source.policy.readiness {
                    if let Some(preroll) = gate.preroll.get() {
                        preroll.close();
                    }
                }
                Ok(())
            }
        }
    }
}
pub(super) struct Source {
    runtime: Weak<Runtime>,
    pub(super) policy: Arc<TargetPolicy>,
    origin: Arc<NativeReplacementStartOrigin>,
    initial: Mutex<Option<Arc<NativeReplacementStartRead>>>,
    adopted: OnceLock<Arc<connect_store::NativeMediaOrigin>>,
}
impl Source {
    fn start_read(&self, runtime: &Runtime) -> Result<Arc<NativeReplacementStartRead>> {
        let held = self.initial.lock().map_err(|_| Error::storage())?.clone();
        match held {
            Some(read) if read.current() => Ok(read),
            Some(_) => Err(Error::forbidden()),
            None => runtime
                .media
                .store
                .try_native_replacement_start_read(&self.origin)
                .map(Arc::new),
        }
    }
}
impl ReplacementSource for Source {
    fn offer(&self) -> Result<ReplacementOffer> {
        let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
        let read = self.start_read(&runtime)?;
        if read.target().endpoint_transport
            != connect_protocol::provisioning::EndpointTransport::Webrtc
        {
            return Err(Error::forbidden());
        }
        Ok(ReplacementOffer {
            participant_id: read.target().connection.participant_id.clone(),
            offer: read.target().offer.clone(),
        })
    }
    fn current(&self) -> Result<()> {
        let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
        if runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if let Some(origin) = self.adopted.get() {
            return if runtime.media.store.try_native_connection_current(origin)? {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        let root = self.policy.root.upgrade().ok_or_else(Error::forbidden)?;
        if root.handle.is_committed() {
            return Err(Error::new(
                "handoff-pending",
                "Original replacement adoption is being recorded",
            ));
        }
        if root.handle.cancellation().is_cancelled() || Instant::now() >= root.deadline {
            return Err(Error::forbidden());
        }
        if let Some(origin) = self.policy.origin.get() {
            let read = runtime
                .media
                .store
                .try_native_replacement_media_read(origin)?;
            if !read.current() {
                return Err(Error::forbidden());
            }
        } else {
            self.start_read(&runtime)?;
        }
        Ok(())
    }
    fn release_invocation(&self) {
        if let Ok(mut held) = self.initial.lock() {
            held.take();
        }
    }
}
#[derive(Default)]
struct Owned {
    root: Option<Arc<PreparedReplacement>>,
    allocated: Option<Arc<AllocatedReplacement>>,
    attempt: Option<NativeReplacementStartAttempt>,
    source: Option<Arc<Source>>,
    driver: Option<Arc<dyn ReplacementAttempt>>,
    controller: Option<Arc<StreamController>>,
    peer: Option<Arc<rvoip_core::peer_admission::PeerAdmission>>,
    intent: Option<connect_protocol::connection_control::OfferIntent>,
    deadline: Option<Instant>,
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|a| a.peer_guard())
        .ok_or_else(Error::forbidden)
}
fn denied<T: std::fmt::Debug>(e: T) -> Error {
    tracing::debug!(?e, "RTC replacement boundary refused");
    Error::forbidden()
}
impl Runtime {
    pub(crate) async fn webrtc_replacement_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<Reply>>,
    ) -> Result<()> {
        let mut owned = Owned::default();
        let result =
            Box::pin(self.webrtc_replacement_start(&a, &physical, &credential, &mut owned)).await;
        let succeeded = result.is_ok();
        let handler_finished = a.invocation_cancellation();
        let _ = tx.send(result);
        if let Some(source) = &owned.source {
            source.release_invocation();
        }
        drop(a);
        let admitted = if succeeded {
            let deadline = owned.deadline.ok_or_else(Error::forbidden)?;
            let finished = tokio::select! {biased;
                _=self.stop.cancelled()=>false,_=physical.lifetime.cancelled()=>false,
                _=tokio::time::sleep_until(deadline)=>false,_=handler_finished.cancelled()=>true,
            };
            if finished {
                match Box::pin(self.webrtc_replacement_admit(&mut owned, &physical)).await {
                    Ok(()) => true,
                    Err(error) => {
                        tracing::debug!(code=%error.code, "RTC replacement resource admission refused");
                        false
                    }
                }
            } else {
                false
            }
        } else {
            false
        };
        if admitted {
            let source = owned.source.as_ref().ok_or_else(Error::forbidden)?;
            let driver = owned.driver.as_ref().ok_or_else(Error::forbidden)?;
            loop {
                tokio::select! {biased;
                    _=self.stop.cancelled()=>break,_=physical.lifetime.cancelled()=>break,
                    _=driver.closed()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{},
                }
                match source.current() {
                    Ok(()) => {}
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "authority-busy" | "storage-unavailable" | "handoff-pending"
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                }
                if source.adopted.get().is_some() {
                    self.reconcile_connection_role(source.policy.pending.binding().connection())
                        .await;
                }
            }
        }
        // Metadata failures cannot skip joining the original physical owner.
        // Retain the first failure, attempt every owned cleanup, then report it.
        let mut failure = None;
        if let Some(root) = &owned.root {
            if !root.handle.is_committed() {
                root.handle.cancel();
            }
        }
        if let Some(attempt) = owned.attempt.take() {
            let uncertain = Arc::new(attempt.uncertain());
            if let Some(allocated) = &owned.allocated {
                match allocated.failed_start.lock() {
                    Ok(mut slot) => *slot = Some(uncertain.clone()),
                    Err(_) => {
                        failure.get_or_insert_with(Error::storage);
                    }
                }
            }
            let store = self.media.store.clone();
            if let Err(error) =
                store_call(move || store.native_replacement_start_uncertain(&uncertain)).await
            {
                failure.get_or_insert(error);
            }
        }
        if let Some(driver) = &owned.driver {
            if let Err(error) = driver.shutdown().await {
                failure.get_or_insert(error);
            }
        }
        if let Some(allocated) = &owned.allocated {
            if let Some(binding) = &allocated.webrtc {
                if let Err(error) = binding.shutdown().await {
                    failure.get_or_insert_with(|| denied(error));
                }
            }
        }
        if let Some(source) = &owned.source {
            let id = source.policy.pending.binding().connection();
            // Core owns the route relationship as well as adapter resources.
            // Ask Core to retire the exact original target even if its physical
            // driver has already closed and the adapter liveness test is false.
            if self.core.connection_transport(id).is_ok() {
                if let Err(error) = self
                    .core
                    .end_connection(id.clone(), rvoip_core::adapter::EndReason::Normal)
                    .await
                {
                    failure.get_or_insert_with(|| denied(error));
                }
            }
            if self.native_connection_live(id) {
                failure.get_or_insert_with(|| {
                    Error::new(
                        "cleanup-unconfirmed",
                        "RTC replacement original route remains live",
                    )
                });
            } else if let Some(origin) = source.adopted.get() {
                // The movement watcher also settles this same receipt. Both
                // paths are idempotent observations of the original lifetime;
                // neither recreates a mutation or borrows the old credential.
                if let Err(error) = self.settle_adopted_rtc_retirement(origin.clone()).await {
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(root) = &owned.root {
            // The preparation watcher transfers this entry's lifetime to us
            // at commit. Remove only our exact original entry after all owned
            // route joins and durable retirement attempts have completed.
            self.replacements
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain(|_, current| !Arc::ptr_eq(current, root));
        }
        failure.map_or(Ok(()), Err)
    }
    async fn settle_adopted_rtc_retirement(
        &self,
        origin: Arc<connect_store::NativeMediaOrigin>,
    ) -> Result<()> {
        let id = ConnectionId::from_string(origin.runtime_connection_id());
        let mut cutoff = None;
        loop {
            self.check_custodian_stop_deadline(&mut cutoff)?;
            let store = self.media.store.clone();
            let origin = origin.clone();
            let absent = crate::canonical_native::Absent {
                natives: self.media_adapters(),
                id: id.clone(),
                generation: origin.generation(),
            };
            match store_call(move || store.settle_native_replacement_cleanup(&origin, &absent))
                .await
            {
                Ok(()) => {
                    self.media.remove(&id);
                    return Ok(());
                }
                Err(error) => {
                    tracing::debug!(code=%error.code,"RTC replacement durable retirement remains pending")
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    async fn webrtc_replacement_start(
        self: &Arc<Self>,
        a: &Arc<AuthorizedEnvelope>,
        physical: &Arc<Physical>,
        credential: &str,
        owned: &mut Owned,
    ) -> Result<Reply> {
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(a, authority)?;
        let intent = crate::canonical_native::intent(a.envelope())?;
        if physical.binding.transport() != rvoip_core::Transport::WebSocket
            || intent.endpoint_transport()?
                != connect_protocol::provisioning::EndpointTransport::Webrtc
        {
            return Err(Error::forbidden());
        }
        let record = intent.offer["replacement_preparation"]["operation_record_id"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        let root = self
            .replacements
            .lock()
            .map_err(|_| Error::storage())?
            .get(record)
            .cloned()
            .ok_or_else(|| {
                Error::new("not-ready", "Original replacement preparation is not live")
            })?;
        let allocated = root
            .allocated
            .lock()
            .map_err(|_| Error::storage())?
            .clone()
            .ok_or_else(|| Error::new("not-ready", "Original target allocation is not recorded"))?;
        if allocated
            .physical
            .upgrade()
            .is_none_or(|p| !Arc::ptr_eq(&p, physical))
            || !allocated.recorded.load(Ordering::Acquire)
        {
            return Err(Error::forbidden());
        }
        let binding = allocated.webrtc.clone().ok_or_else(Error::forbidden)?;
        if allocated
            .started
            .lock()
            .map_err(|_| Error::storage())?
            .is_some()
        {
            return Err(Error::new(
                "reconciliation-required",
                "RTC replacement SDP was already attempted; observe the original operation",
            ));
        }
        let (read, entries) = retry_metadata(&self.stop, || async {
            let store = self.media.store.clone();
            let receipt = allocated.receipt.clone();
            let key = credential.to_owned();
            let a = a.clone();
            let read = store_call(move || {
                store.native_replacement_start_read_with_peer(&receipt, &key, proof(&a)?)
            })
            .await?;
            let entries = self.media.replacement_preparation_entries(read.source())?;
            Ok((read, entries))
        })
        .await?;
        let (profile, plan) = crate::canonical_media::offer_plan(
            read.target(),
            &crate::canonical_native::base_profile()?,
            a.principal(),
        )
        .map_err(denied)?;
        if !plan.rejected().is_empty()
            || !(1..=3).contains(&plan.selected().len())
            || plan
                .selected()
                .iter()
                .any(|s| !matches!(s.requested.medium, Medium::Audio | Medium::Video))
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "RTC replacement requires its exact selected audio and optional directional video ports",
            ));
        }
        let deadline = root.deadline.min(
            Instant::now()
                + Duration::from_secs(
                    (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64
                ),
        );
        let readiness = super::readiness::ReadyGate::new(&read, &plan, deadline)?;
        let policy = Arc::new(TargetPolicy {
            base: self.media.clone(),
            pending: root.handle.prepared_binding(),
            principal: a.principal().clone(),
            profile,
            plan,
            root: Arc::downgrade(&root),
            activation: OnceLock::new(),
            origin: OnceLock::new(),
            preparation_operation: OnceLock::new(),
            data_observer: None,
            readiness: Some(readiness.clone()),
        });
        let (attempt, read) = {
            let store = self.media.store.clone();
            let receipt = allocated.receipt.clone();
            let key = credential.to_owned();
            let a = a.clone();
            store_call(move || {
                store.begin_native_replacement_start_with_read(receipt, &key, proof(&a)?, read)
            })
            .await?
        };
        owned.attempt = Some(attempt);
        owned.root = Some(root.clone());
        owned.allocated = Some(allocated.clone());
        owned.deadline = Some(deadline);
        owned.intent = Some(intent.clone());
        let read = Arc::new(read);
        let origin = self.media.store.native_replacement_start_origin(
            owned.attempt.as_ref().ok_or_else(Error::storage)?,
            &read,
        )?;
        let source = Arc::new(Source {
            runtime: Arc::downgrade(self),
            policy: policy.clone(),
            origin,
            initial: Mutex::new(Some(read.clone())),
            adopted: OnceLock::new(),
        });
        owned.source = Some(source.clone());
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?.clone();
        owned.peer = Some(peer);
        let invocation = StartInvocation {
            policy: policy.clone(),
            authority: StartAuthority::Signed(a.clone()),
            read: read.clone(),
            entries,
            deadline,
        };
        self.media
            .register_replacement_target(&policy)
            .map_err(denied)?;
        let host = self.webrtc.get().ok_or_else(Error::forbidden)?;
        let (controller, activation) = host
            .adapter()
            .configure_media(
                policy.pending.binding().connection(),
                a.principal().clone(),
                ConversationId::from_string(intent.conversation_id.clone()),
                self.media.clone(),
                policy.plan.selected().len(),
                &invocation,
            )
            .map_err(denied)?;
        policy
            .activation
            .set(Arc::new(Activation::WebRtc(activation)))
            .map_err(|_| Error::forbidden())?;
        owned.controller = Some(controller);
        // Only short local metadata preparation retains this invocation; the
        // physical owner carries immutable provenance and releases it at answer.
        let driver = host.start_replacement(ReplacementPreparation {
            source: source.clone(),
            physical: physical.lifetime.child_token(),
            binding,
            deadline,
        })?;
        owned.driver = Some(driver.clone());
        let answer = tokio::select! {biased;
            _=self.stop.cancelled()=>return Err(Error::forbidden()),_=physical.lifetime.cancelled()=>return Err(Error::forbidden()),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","RTC replacement SDP deadline")),answer=driver.answer(deadline)=>answer?,
        };
        let mut payload = serde_json::to_value(answer).map_err(|_| Error::storage())?;
        payload["replacement_preparation"] = serde_json::json!({"profile":"connect-replacement-media-v1","operation_record_id":record,"readiness":readiness.challenge()});
        crate::peer_authority::bind_webrtc_replacement_answer(a, &read, &policy.plan, &payload)?;
        let reply =
            crate::canonical::reply(a.envelope(), "connection.answer", payload).map_err(denied)?;
        a.answer_offer(reply, deadline).await.map_err(denied)?;
        let record = self
            .media
            .store
            .native_replacement_start_record_with_peer(&source.origin, proof(a)?)?;
        drop(invocation);
        drop(read);
        Ok(Reply {
            record,
            answered: true,
        })
    }
    async fn webrtc_replacement_admit(
        self: &Arc<Self>,
        owned: &mut Owned,
        physical: &Arc<Physical>,
    ) -> Result<()> {
        let source = owned.source.as_ref().ok_or_else(Error::forbidden)?.clone();
        let root = owned.root.as_ref().ok_or_else(Error::forbidden)?.clone();
        let allocated = owned
            .allocated
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .clone();
        let driver = owned.driver.as_ref().ok_or_else(Error::forbidden)?.clone();
        let controller = owned
            .controller
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .clone();
        let deadline = owned.deadline.ok_or_else(Error::forbidden)?;
        let id = source.policy.pending.binding().connection();
        let adapter = self.webrtc.get().ok_or_else(Error::forbidden)?.adapter();
        tokio::select! {biased;
            _=self.stop.cancelled()=>return Err(Error::forbidden()),_=physical.lifetime.cancelled()=>return Err(Error::forbidden()),
            _=driver.closed()=>return Err(Error::new("closed","RTC replacement original owner retired")),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","RTC replacement resource deadline")),
            result=adapter.wait_initial_media(id,deadline,physical.lifetime.child_token())=>result.map_err(denied)?,
        }
        let store = self.media.store.clone();
        let origin = source.origin.clone();
        let until = deadline.min(Instant::now() + Duration::from_secs(2));
        let (proof, read) = store_call(move || {
            let wait =
                connect_store::AuthorityWait::new(until.saturating_duration_since(Instant::now()));
            let proof = store.resume_webrtc_replacement_start_authority(&origin, &wait)?;
            let read = store.native_replacement_start_read_under(&origin, &proof)?;
            Ok((Arc::new(proof), Arc::new(read)))
        })
        .await?;
        let entries = self.media.replacement_preparation_entries(read.source())?;
        let (_, plan) = crate::canonical_media::offer_plan(
            read.target(),
            &crate::canonical_native::base_profile()?,
            &source.policy.principal,
        )
        .map_err(denied)?;
        if plan.selected() != source.policy.plan.selected()
            || plan.rejected() != source.policy.plan.rejected()
        {
            return Err(Error::forbidden());
        }
        let peer = owned.peer.as_ref().ok_or_else(Error::forbidden)?.clone();
        let invocation = Arc::new(StartInvocation {
            policy: source.policy.clone(),
            authority: StartAuthority::Resumed {
                proof: proof.clone(),
                peer: peer.clone(),
                cancel: physical.lifetime.child_token(),
            },
            read: read.clone(),
            entries,
            deadline,
        });
        controller
            .change_selected_with_held(
                0,
                source.policy.plan.selected(),
                &BTreeSet::new(),
                deadline,
                physical.lifetime.child_token(),
                invocation.clone(),
            )
            .await
            .map_err(denied)?;
        let muted = source
            .policy
            .plan
            .selected()
            .iter()
            .filter(|s| s.requested.direction != StreamDirection::Send)
            .map(|s| s.requested.logical_id.clone())
            .collect();
        controller
            .set_media_gate_with_held(
                1,
                &muted,
                StreamGateAction::Mute,
                deadline,
                invocation.clone(),
            )
            .await
            .map_err(denied)?;
        if !read.current()
            || controller.revision() != 2
            || source
                .policy
                .activation
                .get()
                .and_then(|a| a.confirmed_generation())
                != Some(source.policy.pending.generation())
            || !adapter.is_connection_live(id)
        {
            return Err(Error::forbidden());
        }
        let streams = source
            .policy
            .plan
            .selected()
            .iter()
            .map(|s| s.requested.logical_id.as_str().to_owned())
            .collect::<Vec<_>>();
        let observation = serde_json::json!({"candidate":id.as_str(),"generation":source.policy.pending.generation(),"stream_revision":2,"streams":streams,"resources_prepared":true,"prepared_muted":true,"media_ready":false});
        let receipt = Arc::new(
            owned
                .attempt
                .take()
                .ok_or_else(Error::storage)?
                .started(observation)?,
        );
        let started = Arc::new(StartedReplacement {
            receipt,
            policy: source.policy.clone(),
            controller,
            resource_streams: streams,
            lifecycle: ReplacementLifecycle::WebRtc(source.clone()),
            intent: owned.intent.clone().ok_or_else(Error::storage)?,
            commit: Default::default(),
            phases: Default::default(),
            preparation: OnceLock::new(),
        });
        *allocated.started.lock().map_err(|_| Error::storage())? = Some(started.clone());
        self.media
            .store
            .record_native_replacement_started_with_peer(&started.receipt, &proof)?;
        let origin = self
            .media
            .store
            .native_replacement_media_origin(started.receipt.clone())?;
        source
            .policy
            .origin
            .set(origin)
            .map_err(|_| Error::forbidden())?;
        drop(invocation);
        drop(read);
        drop(proof);
        self.queue_replacement_media(root, started, peer);
        Ok(())
    }
}
