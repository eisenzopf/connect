//! Fresh operator and original physical grants for exact managed preparation.
use super::*;
use connect_store::NativeReplacementPreparationRead;
use rvoip_core::{
    orchestrator::participant_control::{
        ControlResult as ParticipantResult, ParticipantControlError,
        ParticipantReplacementAuthority, ParticipantReplacementControl,
        ParticipantReplacementLease, ParticipantReplacementScope, ReplacementPhase,
    },
    participant_transitions::ParticipantReplacementMutation,
};

#[derive(Clone)]
pub(crate) struct ReplacementPreparationInvocation {
    pub policy: Arc<Policy>,
    pub read: Arc<NativeReplacementPreparationRead>,
    pub principal: AuthenticatedPrincipal,
    pub intent: ParticipantReplacementMutation,
    pub deadline: tokio::time::Instant,
    pub cancel: tokio_util::sync::CancellationToken,
    pub telephone: Option<Arc<crate::telephone_replacement::authority::Target>>,
}
impl ParticipantReplacementLease for ReplacementPreparationInvocation {
    fn current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> ParticipantResult<ParticipantReplacementAuthority> {
        let deny = || ParticipantControlError::Authority;
        let ctx = self.read.context();
        if scope.phase != ReplacementPhase::Prepare
            || scope.plan.is_some()
            || !self.read.current()
            || self.cancel.is_cancelled()
            || scope.intent != &self.intent
            || scope.conversation.as_str() != ctx.conversation_id
            || scope.session.as_str() != ctx.session_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &self.principal,
                scope.principal,
            )
            || scope.intent.operation_id != ctx.operation_record_id
            || scope.intent.correlation_id != ctx.operation_record_id
            || scope.intent.source.as_str() != ctx.source_runtime_connection_id
            || scope.intent.retained.as_str() != ctx.retained_runtime_connection_id
            || scope.intent.participant.as_str() != ctx.participant_id
            || scope.intent.endpoint.as_str() != ctx.target_endpoint_id
            || scope.intent.device.as_str() != ctx.target_device_id
        {
            return Err(deny());
        }
        let old = self.policy.replacement_source_owner(&self.read)?;
        if let Some(target) = &self.telephone {
            if ctx.target_transport != connect_protocol::provisioning::EndpointTransport::Telephone
                || target.principal.subject != ctx.target_route_owner_id
                || target.member.identity().as_str() != ctx.identity_id
                || target.member.participant() != old.member.participant()
                || target.member.epoch() != old.member.epoch()
                || target.endpoint.transport() != Transport::Sip
                || target.endpoint.endpoint().as_str() != ctx.target_endpoint_id
                || target.endpoint.device().as_str() != ctx.target_device_id
            {
                return Err(deny());
            }
            return Ok(ParticipantReplacementAuthority {
                actor: current_member(&self.principal, self.read.caller()).map_err(|_| deny())?,
                target: target.member.clone(),
                endpoint: target.endpoint.clone(),
                may_send_audio: ctx.role != "observer"
                    && old.receive_media.contains(&Medium::Audio),
                may_receive_audio: old.send_media.contains(&Medium::Audio),
            });
        }
        if !matches!(
            ctx.target_transport,
            connect_protocol::provisioning::EndpointTransport::Quic
                | connect_protocol::provisioning::EndpointTransport::Webtransport
        ) {
            return Err(ParticipantControlError::Unsupported);
        }
        let principal = self.policy.replacement_source_principal(&self.read)?;
        let binding = IdentityBinding::from_authenticated(
            &principal,
            IdentityId::from_string(ctx.identity_id.clone()),
            chrono::Utc::now(),
        )
        .map_err(|_| deny())?;
        let mut endpoint = EndpointBinding::provision(
            binding.clone(),
            EndpointId::from_string(ctx.target_endpoint_id.clone()),
            DeviceId::from_string(ctx.target_device_id.clone()),
            crate::canonical_runtime::core_transport(&ctx.target_transport).map_err(|_| deny())?,
            ctx.target_address.clone(),
        )
        .map_err(|_| deny())?;
        endpoint
            .verify(&binding, "connect-store".into(), chrono::Utc::now())
            .map_err(|_| deny())?;
        Ok(ParticipantReplacementAuthority {
            actor: current_member(&self.principal, self.read.caller()).map_err(|_| deny())?,
            target: old.member,
            endpoint,
            may_send_audio: ctx.role != "observer" && old.receive_media.contains(&Medium::Audio),
            may_receive_audio: old.send_media.contains(&Medium::Audio),
        })
    }
}
impl Policy {
    pub(crate) fn replacement_telephone_source(
        &self,
        read: &NativeReplacementPreparationRead,
    ) -> ParticipantResult<Option<Arc<crate::telephone_replacement::media::Policy>>> {
        let deny = || ParticipantControlError::Authority;
        let Some(phone) = read.source().telephone() else {
            return Ok(None);
        };
        if !read.current() {
            return Err(deny());
        }
        let id = ConnectionId::from_string(read.context().source_runtime_connection_id.clone());
        let source = self
            .telephone_targets
            .lock()
            .map_err(|_| deny())?
            .get(&id)
            .and_then(Weak::upgrade)
            .ok_or_else(deny)?;
        let _ = source.established_owner(phone).map_err(|_| deny())?;
        Ok(Some(source))
    }
    pub(crate) fn replacement_source_principal(
        &self,
        read: &NativeReplacementPreparationRead,
    ) -> ParticipantResult<AuthenticatedPrincipal> {
        let deny = || ParticipantControlError::Authority;
        if !read.current() {
            return Err(deny());
        }
        let id = ConnectionId::from_string(read.context().source_runtime_connection_id.clone());
        match read.source() {
            connect_store::NativeReplacementSourceRead::Native(_) => {
                Ok(self.entry(&id).map_err(|_| deny())?.principal.clone())
            }
            connect_store::NativeReplacementSourceRead::Telephone(phone, _) => {
                let source = self
                    .telephone_targets
                    .lock()
                    .map_err(|_| deny())?
                    .get(&id)
                    .and_then(Weak::upgrade)
                    .ok_or_else(deny)?;
                let _ = source.established_owner(phone).map_err(|_| deny())?;
                source.principal().map_err(|_| deny())
            }
        }
    }
    pub(crate) fn replacement_source_owner(
        &self,
        read: &NativeReplacementPreparationRead,
    ) -> ParticipantResult<MediaOwner> {
        let deny = || ParticipantControlError::Authority;
        let ctx = read.context();
        if !read.current() {
            return Err(deny());
        }
        let retained = self
            .entry(&ConnectionId::from_string(
                ctx.retained_runtime_connection_id.clone(),
            ))
            .map_err(|_| deny())?;
        let old = match read.source() {
            connect_store::NativeReplacementSourceRead::Native(source_read) => {
                let source = self
                    .entry(&ConnectionId::from_string(
                        ctx.source_runtime_connection_id.clone(),
                    ))
                    .map_err(|_| deny())?;
                let owner = source
                    .owner(source_read.target(), &source.principal, false)
                    .map_err(|_| deny())?;
                source
                    .activation()
                    .map_err(|_| deny())?
                    .ok_or_else(deny)?
                    .current(owner)
                    .map_err(|_| deny())?
            }
            connect_store::NativeReplacementSourceRead::Telephone(phone, _) => {
                let source = self
                    .telephone_targets
                    .lock()
                    .map_err(|_| deny())?
                    .get(&ConnectionId::from_string(
                        ctx.source_runtime_connection_id.clone(),
                    ))
                    .and_then(Weak::upgrade)
                    .ok_or_else(deny)?;
                source.established_owner(phone).map_err(|_| deny())?
            }
        };
        let remote = retained
            .owner(read.retained().target(), &retained.principal, false)
            .map_err(|_| deny())?;
        retained
            .activation()
            .map_err(|_| deny())?
            .ok_or_else(deny)?
            .current(remote)
            .map_err(|_| deny())?;
        if old.member.participant().as_str() != ctx.participant_id
            || old.member.epoch() != ctx.membership_epoch
        {
            return Err(deny());
        }
        Ok(old)
    }
}
impl Policy {
    pub(super) fn replacement_preparation_control(
        &self,
        scope: &ParticipantReplacementScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ParticipantResult<ParticipantReplacementControl> {
        let invocation = held
            .downcast_ref::<ReplacementPreparationInvocation>()
            .ok_or(ParticipantControlError::Authority)?;
        if !std::ptr::eq(self, Arc::as_ptr(&invocation.policy)) {
            return Err(ParticipantControlError::Authority);
        }
        let _ = invocation.current(scope)?;
        // Current() still enforces the five-second reader. Its upper deadline
        // bounds the owned resource ticket without retaining that reader.
        ParticipantReplacementControl::new(
            Box::new(invocation.clone()),
            invocation.cancel.clone(),
            invocation.deadline,
        )
    }
}

/// Allocated before physical movement. Its origin cannot admit packets until
/// the exact durable completion, and it retains no current Store reader.
pub(crate) struct ReplacementAdoption {
    target: Arc<crate::canonical_replacement::TargetPolicy>,
    metadata: Arc<connect_store::NativeReplacementActivation>,
    entry: Arc<super::Connection>,
    writer: std::sync::Mutex<Option<super::data_writes::Observer>>,
}
impl super::Policy {
    pub(crate) fn prepare_replacement_adoption(
        self: &Arc<Self>,
        target: &Arc<crate::canonical_replacement::TargetPolicy>,
        metadata: Arc<connect_store::NativeReplacementActivation>,
        binding: rvoip_core::relationships::ConnectionBinding,
    ) -> ControlResult<Arc<ReplacementAdoption>> {
        let origin = metadata.origin().clone();
        #[cfg(feature = "test-support")]
        if let Some(armed) = self
            .replacement_adoption_refusals
            .try_lock()
            .map_err(super::metadata_contention)?
            .remove(origin.offer_record_id())
        {
            armed.store(false, std::sync::atomic::Ordering::Release);
            return Err(StreamControlError::Authority);
        }

        if !std::ptr::eq(self.as_ref(), Arc::as_ptr(&target.base))
            || !binding.same_incarnation(target.pending.binding())
            || binding.state() != rvoip_core::relationships::BindingState::Active
            || origin.generation() != binding.generation()
            || origin.runtime_connection_id() != binding.connection().as_str()
            || origin.offer_record_id() != target.pending.intent().operation_id
            || metadata.target().runtime_connection_id != origin.runtime_connection_id()
        {
            return Err(StreamControlError::Authority);
        }
        let activation = target
            .activation
            .get()
            .cloned()
            .ok_or(StreamControlError::NotReady)?;
        let writer = target
            .data_observer
            .as_ref()
            .map(|observer| {
                super::data_writes::Observer::pending_replacement(
                    self.store.clone(),
                    &metadata,
                    observer.scope().clone(),
                )
                .map_err(super::packet_authority_error)
            })
            .transpose()?;
        let entry = Arc::new(super::Connection {
            domain: Arc::downgrade(self),
            origin,
            principal: target.principal.clone(),
            profile: target.profile.clone(),
            plan: target.plan.clone(),
            negotiated_remote: std::sync::Mutex::new(target.plan.remote_capabilities().clone()),
            activation: std::sync::Mutex::new(Some(activation)),
            binding: std::sync::Mutex::new(Some(binding)),
            preparation: Default::default(),
        });
        Ok(Arc::new(ReplacementAdoption {
            target: target.clone(),
            metadata,
            entry,
            writer: std::sync::Mutex::new(writer),
        }))
    }
}
impl ReplacementAdoption {
    pub(crate) fn install(&self) -> ControlResult<()> {
        let target = &self.target;
        if target
            .root
            .upgrade()
            .is_none_or(|root| !root.handle.is_committed())
        {
            return Err(StreamControlError::Authority);
        }
        target
            .activation
            .get()
            .ok_or(StreamControlError::NotReady)?
            .current(
                self.entry
                    .owner(self.metadata.target(), &self.entry.principal, false)?,
            )?;
        let id = target.pending.binding().connection();
        let mut entries = target
            .base
            .entries
            .try_lock()
            .map_err(super::metadata_contention)?;
        if let Some(existing) = entries.get(id) {
            return if Arc::ptr_eq(existing, &self.entry) {
                Ok(())
            } else {
                Err(StreamControlError::Authority)
            };
        }
        let mut provisional = target
            .base
            .replacement_targets
            .try_lock()
            .map_err(super::metadata_contention)?;
        if provisional
            .get(id)
            .and_then(std::sync::Weak::upgrade)
            .is_none_or(|p| !Arc::ptr_eq(&p, target))
        {
            return Err(StreamControlError::Authority);
        }
        let replacing = usize::from(entries.contains_key(&target.pending.intent().source));
        if entries.len().saturating_sub(replacing) >= target.base.capacity {
            return Err(StreamControlError::Authority);
        }
        let mut writer = self.writer.try_lock().map_err(super::metadata_contention)?;
        match (&target.data_observer, writer.take()) {
            (Some(observer), Some(writer)) => {
                #[cfg(feature = "test-support")]
                let writer = if let Some(armed) = target
                    .base
                    .replacement_data_observer_refusals
                    .try_lock()
                    .map_err(super::metadata_contention)?
                    .remove(self.metadata.origin().offer_record_id())
                {
                    armed.store(false, std::sync::atomic::Ordering::Release);
                    writer.with_mismatched_scope_for_test()
                } else {
                    writer
                };
                observer.activate(writer)?;
            }

            (None, None) => {}
            _ => return Err(StreamControlError::Authority),
        }
        // No fallible operation follows the registry changes. The original
        // physical source has already gone; its slot transfers exactly once.
        entries.remove(&target.pending.intent().source);
        entries.insert(id.clone(), self.entry.clone());
        provisional.remove(id);
        Ok(())
    }

    /// Called while Store may own a transaction. Inspect only local leaf
    /// metadata with try_lock; never acquire Store authority or role coordination.
    pub(crate) fn hold_installed(
        &self,
        expected: &connect_store::NativeReplacementActivation,
    ) -> connect_protocol::Result<
        Option<Box<dyn connect_store::NativeReplacementInstallationGuard + '_>>,
    > {
        if !std::ptr::eq(Arc::as_ptr(&self.metadata), expected)
            || !Arc::ptr_eq(&self.entry.origin, expected.origin())
        {
            return Err(connect_protocol::Error::forbidden());
        }
        fn busy<T>(error: std::sync::TryLockError<T>) -> connect_protocol::Error {
            match error {
                std::sync::TryLockError::WouldBlock => connect_protocol::Error::new(
                    "authority-busy",
                    "Replacement installation metadata is busy",
                ),
                std::sync::TryLockError::Poisoned(_) => connect_protocol::Error::forbidden(),
            }
        }
        let entries = self.target.base.entries.try_lock().map_err(busy)?;
        let provisional = self
            .target
            .base
            .replacement_targets
            .try_lock()
            .map_err(busy)?;
        let id = self.target.pending.binding().connection();
        let activation = self.entry.activation.try_lock().map_err(busy)?;
        let binding = self.entry.binding.try_lock().map_err(busy)?;
        let installed = entries
            .get(id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
            && !provisional.contains_key(id)
            && self
                .target
                .data_observer
                .as_ref()
                .is_none_or(|observer| observer.installed())
            && activation
                .as_ref()
                .zip(self.target.activation.get())
                .is_some_and(|(installed, original)| {
                    let original: Arc<dyn MediaActivation> = original.clone();
                    Arc::ptr_eq(installed, &original)
                })
            && binding.as_ref().is_some_and(|binding| {
                binding.connection() == id
                    && binding.generation() == expected.origin().generation()
                    && binding.state() == rvoip_core::relationships::BindingState::Active
            });
        if !installed {
            return Ok(None);
        }
        // Adapter registry presence can survive controller cancellation while
        // cleanup drains. Check the original sealed resources, not only its ID.
        // This try-read cannot wait under the Store publication transaction.
        let generation = self
            .target
            .activation
            .get()
            .ok_or_else(connect_protocol::Error::forbidden)?
            .try_confirmed_generation()
            .map_err(|error| match error {
                StreamControlError::NotReady => connect_protocol::Error::new(
                    "authority-busy",
                    "Native activation metadata is busy",
                ),
                _ => connect_protocol::Error::forbidden(),
            })?;
        if generation != Some(expected.origin().generation()) {
            return Err(connect_protocol::Error::forbidden());
        }
        Ok(Some(Box::new(InstalledEntry {
            #[cfg(feature = "test-support")]
            policy: &self.target.base,
            #[cfg(feature = "test-support")]
            record: self.metadata.origin().offer_record_id(),
            native: self
                .target
                .activation
                .get()
                .ok_or_else(connect_protocol::Error::forbidden)?,
            generation: expected.origin().generation(),
            _entries: entries,
            _provisional: provisional,
            _activation: activation,
            _binding: binding,
        })))
    }
}

/// Service removal takes the same registry mutex. Keep both registries stable
/// only for the short final SQL commit, after all fallible Store work is done.
struct InstalledEntry<'a> {
    #[cfg(feature = "test-support")]
    policy: &'a super::Policy,
    #[cfg(feature = "test-support")]
    record: &'a str,
    native: &'a rvoip_uctp::v2::native_negotiation::NativeMediaActivation,
    generation: u64,
    _activation: std::sync::MutexGuard<'a, Option<Arc<dyn MediaActivation>>>,
    _binding: std::sync::MutexGuard<'a, Option<rvoip_core::relationships::ConnectionBinding>>,

    _entries:
        std::sync::MutexGuard<'a, std::collections::BTreeMap<ConnectionId, Arc<super::Connection>>>,
    _provisional: std::sync::MutexGuard<
        'a,
        std::collections::BTreeMap<
            ConnectionId,
            std::sync::Weak<crate::canonical_replacement::TargetPolicy>,
        >,
    >,
}
impl connect_store::NativeReplacementInstallationGuard for InstalledEntry<'_> {
    fn publish(
        &self,
        commit: Box<dyn FnOnce() -> connect_protocol::Result<()> + '_>,
    ) -> connect_protocol::Result<()> {
        #[cfg(feature = "test-support")]
        self.policy.retire_at_publication_for_test(self.record)?;
        #[cfg(feature = "test-support")]
        let mut retirement = None;
        let published = self.native.try_publish_installed(self.generation, || {
            #[cfg(feature = "test-support")]
            {
                retirement = self
                    .policy
                    .begin_publication_retirement_for_test(self.record)?;
                if let Some(retirement) = &retirement {
                    retirement.wait_for_cancellation()?;
                }
            }
            commit()
        });
        // Core releases installation metadata before we join actual retirement.
        // This hook is absent from normal production builds.
        #[cfg(feature = "test-support")]
        if let Some(retirement) = retirement {
            retirement.join()?;
        }
        published.map_err(|error| match error {
            StreamControlError::NotReady => connect_protocol::Error::new(
                "authority-busy",
                "Native publication metadata is busy",
            ),
            _ => connect_protocol::Error::forbidden(),
        })?
    }
}
