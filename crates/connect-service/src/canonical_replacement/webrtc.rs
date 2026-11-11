//! One RTC replacement owner, from the original signed SDP through adoption.
//! No originating reader survives ICE/DTLS/RTP waits or the active lifetime.
pub(super) mod admission;

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
    pub(super) fn start_private(&self, policy: &Arc<TargetPolicy>) -> Result<()> {
        match self {
            Self::Native(n) => n.start_private(policy),
            Self::WebRtc(_) => Ok(()),
        }
    }
    pub(super) fn begin_movement(&self) {
        if let Self::Native(n) = self {
            n.begin_movement();
        }
    }
    pub(super) fn activate(
        &self,
        adoption: &crate::canonical_media::replacement::ReplacementAdoption,
        captured: &Arc<connect_store::NativeReplacementActivation>,
    ) -> Result<()> {
        let entry = adoption.lifecycle_entry(captured)?;
        match self {
            Self::Native(n) => n.activate(entry),
            Self::WebRtc(source) => {
                if entry.origin.runtime_connection_id()
                    != source.policy.pending.binding().connection().as_str()
                    || entry.origin.generation() != source.policy.pending.generation()
                {
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
    offer: ReplacementOffer,
    adopted: OnceLock<Arc<connect_store::NativeMediaOrigin>>,
}
impl ReplacementSource for Source {
    fn offer(&self) -> Result<ReplacementOffer> {
        self.current()?;
        Ok(ReplacementOffer {
            participant_id: self.offer.participant_id.clone(),
            offer: self.offer.offer.clone(),
        })
    }
    fn current(&self) -> Result<()> {
        let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
        if runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if let Some(origin) = self.adopted.get() {
            return if runtime
                .media
                .connection_lifetime_current(origin, Some(self.policy.pending.binding()))?
            {
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
        } else if chrono::Utc::now().timestamp() >= self.origin.expires_at() {
            return Err(Error::forbidden());
        }
        // Before private admission only the original resource lifetime applies;
        // no conversation payload or participant publication is available.
        Ok(())
    }
    fn release_invocation(&self) {}
}
#[derive(Default)]
struct Owned {
    root: Option<Arc<PreparedReplacement>>,
    quarantined: Option<rvoip_core::staged_streams::QuarantinedInitialMedia>,
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
// One diagnostic per failed owned stage, with original correlation only.
// No media, credentials, new attempt or cleanup deadline is introduced.
fn cleanup_diagnostic(owned: &Owned, stage: &'static str, error: &impl std::fmt::Debug) {
    tracing::debug!(target: "connect_service::rtc_replacement_cleanup",
        stage,
        connection = ?owned.root.as_ref().map(|root| &root.handle.intent().replacement),
        operation_id = ?owned.root.as_ref().map(|root| &root.handle.intent().operation_id),
        operation_record = ?owned.source.as_ref().map(|s| s.origin.operation_record_id()),
        ?error,
        "RTC replacement original retirement stage refused");
}
type OriginalWorker<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>>;

impl Runtime {
    pub(crate) async fn webrtc_replacement_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<Reply>>,
    ) -> Result<()> {
        let mut owned = Owned::default();
        // The original child already owns one Box. Erase its concrete coroutine
        // shape at this Send-checked boundary without changing poll/drop custody.
        let handler_finished = a.invocation_cancellation();
        let start: OriginalWorker<'_, Reply> =
            Box::pin(self.webrtc_replacement_start(a, &physical, &credential, &mut owned));
        let result = start.await;
        if let Err(error) = &result {
            tracing::debug!(target: "connect_service::canonical_replacement",
                operation_id = ?owned.root.as_ref().map(|root| &root.handle.intent().operation_id),
                stage = "initial-signaling", code = %error.code,
                original_start_attempt_owned = owned.attempt.is_some(),
                original_driver_owned = owned.driver.is_some(),
                "RTC replacement original signaling refused");
        }
        let succeeded = result.is_ok();
        let _ = tx.send(result);
        if let Some(source) = &owned.source {
            source.release_invocation();
        }
        let admitted = if succeeded {
            let deadline = owned.deadline.ok_or_else(Error::forbidden)?;
            let finished = tokio::select! {biased;
                _=self.stop.cancelled()=>false,_=physical.lifetime.cancelled()=>false,
                _=tokio::time::sleep_until(deadline)=>false,_=handler_finished.cancelled()=>true,
            };
            if finished {
                let admission: OriginalWorker<'_, ()> =
                    Box::pin(self.webrtc_replacement_admit(&mut owned, &physical));
                match admission.await {
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
            let mut reconciled_role = None;
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
                    self.reconcile_connection_role_generation(
                        source.policy.pending.binding().connection(),
                        &mut reconciled_role,
                    )
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
                        let error = Error::storage();
                        cleanup_diagnostic(&owned, "uncertain-start-custody", &error);
                        failure.get_or_insert(error);
                    }
                }
            }
            let store = self.media.store.clone();
            if let Err(error) =
                store_call(move || store.native_replacement_start_uncertain(&uncertain)).await
            {
                cleanup_diagnostic(&owned, "uncertain-start-record", &error);
                failure.get_or_insert(error);
            }
        }
        if let Some(driver) = &owned.driver {
            if let Err(error) = driver.shutdown().await {
                cleanup_diagnostic(&owned, "original-driver", &error);
                failure.get_or_insert(error);
            }
        }
        if let Some(allocated) = &owned.allocated {
            if let Some(binding) = &allocated.webrtc {
                if let Err(error) = binding.shutdown().await {
                    cleanup_diagnostic(&owned, "original-physical", &error);
                    failure.get_or_insert_with(|| denied(error));
                }
            }
        }
        if owned.allocated.is_some() {
            // The successful original allocation and root were captured before
            // Source was installed. Retire that exact route even if constructing
            // Source failed. This also avoids two racing Core map lookups after
            // physical closure. Adapter retirement still joins backend/controller
            // resources and publishes the original Core terminal event.
            let binding = owned.allocated.as_ref().and_then(|a| a.webrtc.as_ref());
            match (self.webrtc.get(), owned.root.as_ref(), binding) {
                (Some(host), Some(root), Some(binding)) => {
                    if let Err(error) = host
                        .adapter()
                        .end_original_binding(
                            root.handle.intent().replacement.clone(),
                            binding,
                            rvoip_core::adapter::EndReason::Normal,
                        )
                        .await
                    {
                        cleanup_diagnostic(&owned, "original-route", &error);
                        failure.get_or_insert_with(|| denied(error));
                    }
                }
                _ => {
                    let error = Error::new(
                        "cleanup-unconfirmed",
                        "RTC replacement original binding is unavailable",
                    );
                    cleanup_diagnostic(&owned, "original-binding", &error);
                    failure.get_or_insert(error);
                }
            }
        }
        if let Some(source) = &owned.source {
            let id = source.policy.pending.binding().connection();
            if self.native_connection_live(id) {
                let error = Error::new(
                    "cleanup-unconfirmed",
                    "RTC replacement original route remains live",
                );
                cleanup_diagnostic(&owned, "route-still-live", &error);
                failure.get_or_insert(error);
            } else if let Some(origin) = source.adopted.get() {
                // The movement watcher also settles this same receipt. Both
                // paths are idempotent observations of the original lifetime;
                // neither recreates a mutation or borrows the old credential.
                if let Err(error) = self.settle_adopted_rtc_retirement(origin.clone()).await {
                    cleanup_diagnostic(&owned, "durable-retirement", &error);
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
        a: Arc<AuthorizedEnvelope>,
        physical: &Arc<Physical>,
        credential: &str,
        owned: &mut Owned,
    ) -> Result<Reply> {
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(&a, authority)?;
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
            // This signed resource use needs its exact live original preparation.
            // Missing custody denies authority, including after natural retirement.
            .ok_or_else(Error::forbidden)?;
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
        // Before begin, a temporary metadata refusal has no phase/Core effect.
        // Do not retain the signed mutation writer through five-second retries.
        let (read, entries) = {
            let store = self.media.store.clone();
            let receipt = allocated.receipt.clone();
            let key = credential.to_owned();
            let a = a.clone();
            let read = store_call(move || {
                store.native_replacement_start_read_with_peer(&receipt, &key, proof(&a)?)
            })
            .await?;
            let entries = self.media.replacement_preparation_entries(read.source())?;
            (read, entries)
        };
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
        // The original start reader still bounds SDP and resource admission.
        // Later private readiness belongs to the unchanged reservation lifetime;
        // every packet continues to require fresh current authority.
        let readiness = super::readiness::ReadyGate::new(&read, &plan, root.deadline)?;
        let policy = Arc::new(TargetPolicy {
            base: self.media.clone(),
            pending: root.handle.prepared_binding(),
            principal: a.principal().clone(),
            profile,
            plan,
            root: Arc::downgrade(&root),
            activation: OnceLock::new(),
            public_continuation: OnceLock::new(),
            origin: OnceLock::new(),
            preparation_operation: OnceLock::new(),
            receive_diagnostics: Default::default(),
            scene_snapshots: Default::default(),
            data_observer: None,
            readiness: Some(readiness.clone()),
        });
        let (attempt, read, origin) = {
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
        // Immutable attribution was validated and committed with the original
        // begin. No primary-SQL retry extends this signed mutation interval.
        let source = Arc::new(Source {
            runtime: Arc::downgrade(self),
            policy: policy.clone(),
            origin,
            offer: ReplacementOffer {
                participant_id: read.target().connection.participant_id.clone(),
                offer: read.target().offer.clone(),
            },
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
        owned.controller = Some(controller.clone());
        let source_generation = read.source().context().source_generation;
        let target_generation = read.target().connection.generation;
        let participant = read.target().connection.participant_id.clone();
        owned.quarantined = Some(
            controller
                .stage_initial_selected_with_held(
                    policy.plan.selected(),
                    deadline,
                    physical.lifetime.child_token(),
                    Arc::new(invocation),
                )
                .await
                .map_err(denied)?,
        );
        drop(read);
        let a = Arc::try_unwrap(a)
            .map_err(|_| Error::new("conflict", "Original RTC invocation is still shared"))?
            .into_resource_preparation(
                participant.clone(),
                policy
                    .plan
                    .selected()
                    .iter()
                    .map(|s| s.requested.logical_id.as_str().to_owned())
                    .collect(),
                deadline,
            )
            .map_err(denied)?;
        // The driver receives immutable original parameters and resource-only
        // custody. No Store grant survives its SDP/ICE/DTLS work.
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
        crate::peer_authority::validate_replacement_resource_answer(
            &intent,
            source_generation,
            target_generation,
            &participant,
            &policy.plan,
            &payload,
        )?;
        let reply =
            crate::canonical::reply(a.envelope(), "connection.answer", payload).map_err(denied)?;
        a.answer_offer(reply, deadline).await.map_err(denied)?;
        // The answer has already been delivered. Reobserve only its scoped
        // original operation metadata; never repeat answer delivery or IO.
        let until = deadline.min(Instant::now() + Duration::from_secs(5));
        let wait =
            connect_store::AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let _cancel_record_wait = wait.cancel_on_drop();
        let root_cancelled = root.handle.cancellation();
        let record = admission::read_busy(
            until,
            &wait,
            async {
                tokio::select! { biased;
                    _ = self.stop.cancelled() => {},
                    _ = physical.lifetime.cancelled() => {},
                    _ = root_cancelled.cancelled() => {},
                    _ = driver.closed() => {},
                }
            },
            || {
                let store = self.media.store.clone();
                let origin = source.origin.clone();
                let wait = wait.clone();
                async move {
                    store_call(move || {
                        let proof =
                            store.resume_native_replacement_start_authority(&origin, &wait)?;
                        store.native_replacement_start_record_with_peer(&origin, &proof)
                    })
                    .await
                }
            },
        )
        .await
        .map_err(|error| {
            tracing::debug!(target: "connect_service::canonical_replacement",
                operation_record = record, stage = "post-answer-record", code = %error.code,
                "RTC replacement original metadata refused");
            error
        })?;
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
        let prepared = owned
            .quarantined
            .take()
            .ok_or_else(Error::forbidden)?
            .prepare()
            .await
            .map_err(denied)?;
        let until = deadline.min(Instant::now() + Duration::from_secs(2));
        let wait =
            connect_store::AuthorityWait::new(until.saturating_duration_since(Instant::now()));
        let _cancel_wait = wait.cancel_on_drop();
        let root_cancelled = root.handle.cancellation();
        let (proof, read, entries) = admission::read(
            until,
            &wait,
            async {
                tokio::select! { biased;
                    _ = self.stop.cancelled() => {},
                    _ = physical.lifetime.cancelled() => {},
                    _ = root_cancelled.cancelled() => {},
                    _ = driver.closed() => {},
                }
            },
            || async {
                // Retry only fresh metadata after the original SDP/physical
                // start. Each failed observation drops its grant and registry
                // entries before another attempt; no Core effect runs here.
                let store = self.media.store.clone();
                let origin = source.origin.clone();
                let wait = wait.clone();
                let (proof, read) = store_call(move || {
                    // Same current Read as native dormant muted activation.
                    let proof = store
                        .resume_native_replacement_start_activation_authority(&origin, &wait)?;
                    let read = store.native_replacement_start_read_under(&origin, &proof)?;
                    Ok((Arc::new(proof), Arc::new(read)))
                })
                .await?;
                if !adapter.is_connection_live(id)
                    || allocated
                        .physical
                        .upgrade()
                        .is_none_or(|current| !Arc::ptr_eq(&current, physical))
                {
                    return Err(Error::forbidden());
                }
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
                Ok((proof, read, entries))
            },
        )
        .await
        .map_err(|error| {
            tracing::debug!(operation_record=%source.origin.operation_record_id(),
                stage="fresh-resource-admission",code=%error.code,
                "RTC replacement resource metadata refused");
            error
        })?;
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
        prepared
            .activate_muted_with_held(invocation.clone())
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
        // The physical cut and exact receipt are complete. Historical recording
        // may need a Writer if cancellation/uncertainty settles concurrently.
        // Drop all activation readers before recording this same receipt.
        drop(invocation);
        drop(read);
        drop(proof);
        let store = self.media.store.clone();
        let receipt = started.receipt.clone();
        self.owned_store_call(move || store.record_native_replacement_started(&receipt))
            .await?;
        // The original resources and their immutable capture already exist.
        // Temporary command-SQL exclusion must reobserve only this origin read,
        // never configure/mute, start recording, or the once-only physical IO.
        // Reuse the SAME admission cutoff and cancellation owner.
        let origin = admission::read_busy(
            until,
            &wait,
            async {
                tokio::select! { biased;
                    _ = self.stop.cancelled() => {},
                    _ = physical.lifetime.cancelled() => {},
                    _ = root_cancelled.cancelled() => {},
                    _ = driver.closed() => {},
                }
            },
            || {
                std::future::ready(
                    self.media
                        .store
                        .native_replacement_media_origin(started.receipt.clone()),
                )
            },
        )
        .await
        .map_err(|error| {
            tracing::debug!(target: "connect_service::canonical_replacement",
                operation_record = source.origin.operation_record_id(),
                stage = "started-media-origin", code = %error.code,
                "RTC replacement original metadata refused");
            error
        })?;
        source
            .policy
            .origin
            .set(origin)
            .map_err(|_| Error::forbidden())?;
        self.queue_replacement_media(root, started, peer);
        Ok(())
    }
}
