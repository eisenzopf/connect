//! Preparation owns immutable facts and the original linear model, never grants.
use super::*;
use rvoip_core::orchestrator::participant_control::{
    ManagedAudioDeviceReplacementPlan, ParticipantControlError, PreparedManagedCut,
};
use rvoip_core::staged_streams::StreamControlError;

pub(super) struct ColdCommit {
    pub plan: ManagedAudioDeviceReplacementPlan,
    pub metadata: Arc<connect_store::NativeReplacementCommitPreparation>,
    pub prepared: PreparedManagedCut,
    pub adoption: Arc<crate::canonical_media::replacement::ReplacementAdoption>,
    pub source_phone: Option<Arc<crate::telephone_replacement::media::Policy>>,
    pub route_resource: Option<rvoip_sip::adapter::SipManagedRouteResource>,
    #[cfg(feature = "test-support")]
    pub control_probe: Option<Arc<crate::canonical_media::preparation_custody_fixture::Probe>>,
}
fn busy(error: &ParticipantControlError) -> bool {
    matches!(
        error,
        ParticipantControlError::Media(
            StreamControlError::MetadataBusy | StreamControlError::Backpressure
        )
    )
}
impl Runtime {
    pub(super) async fn prepare_native_managed_cut(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        until: tokio::time::Instant,
    ) -> Result<ColdCommit> {
        // Select exactly once. In particular, this preserves the previously
        // recorded F and its nonce rather than recreating it after a Busy turn.
        let plan = if root.public_video.required() {
            root.public_video.commit_plan()?
        } else {
            self.core
                .plan_staged_device_replacement(&root.handle, replacement_video(&root.plan)?)
                .map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-select-plan",?error,"Managed replacement cold stage refused");
                    super::super::projection::core_plan_error(error)
                })?
        };
        let plan = if let Some(parent) = plan.media().private_predecessor() {
            let original = {
                let audiences = self
                    .consultation_audiences
                    .lock()
                    .map_err(|_| Error::storage())?;
                let original = audiences.get(parent).ok_or_else(Error::forbidden)?;
                if original.session != root.origin.request().session_id
                    || original.handle.primary_connection() != &plan.media().request().source
                {
                    return Err(Error::forbidden());
                }
                original.handle.clone()
            };
            plan.with_private_consultation(&original)
                .map_err(super::super::projection::core_plan_error)?
        } else {
            plan
        };
        let until_epoch = (chrono::Utc::now()
            + chrono::Duration::from_std(
                until.saturating_duration_since(tokio::time::Instant::now()),
            )
            .map_err(|_| Error::forbidden())?)
        .timestamp();
        let mut metadata: Option<Arc<connect_store::NativeReplacementCommitPreparation>> = None;
        let mut route_resource = None;
        loop {
            let cancel = root.handle.cancellation();
            if self.stop.is_cancelled()
                || cancel.is_cancelled()
                || tokio::time::Instant::now() >= until
            {
                tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-original-cutoff",runtime_stopped=self.stop.is_cancelled(),root_cancelled=cancel.is_cancelled(),cutoff_elapsed=tokio::time::Instant::now()>=until,"Managed replacement cold stage refused");
                return Err(Error::forbidden());
            }
            let roles = self.role_projection.lock().await;
            let mut i = match self
                .continuation_read_for_commit_preparation(root, started, plan.clone())
                .await
            {
                Ok(i) => i,
                Err(error) if error.code == "authority-busy" => {
                    drop(roles);
                    self.managed_cut_metadata_pause(until, &cancel).await?;
                    continue;
                }
                Err(error) => {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-planning-read",code=%error.code,"Managed replacement cold stage refused");
                    return Err(error);
                }
            };
            i.deadline = i.deadline.min(until);
            // Planning owner validation is real, but the original held-commit
            // registry oracle belongs to final admission, not this new stage.
            i.managed_capture = true;
            let store = self.media.store.clone();
            let read = i.read.clone();
            let prepared = if let Some(prepared) = metadata.as_ref() {
                let original = prepared.clone();
                store_call(move || {
                    store.revalidate_prepared_native_replacement_commit(&original, &read)
                })
                .await.map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-revalidate-store-metadata",code=%error.code,"Managed replacement cold stage refused");
                    error
                })?;
                prepared.clone()
            } else {
                let origin = started
                    .policy
                    .origin
                    .get()
                    .cloned()
                    .ok_or_else(Error::forbidden)?;
                let prepared = Arc::new(
                    store_call(move || {
                        store.prepare_native_replacement_commit_until(origin, &read, until_epoch)
                    })
                    .await.map_err(|error| {
                        tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-prepare-store-metadata",code=%error.code,"Managed replacement cold stage refused");
                        error
                    })?,
                );
                metadata = Some(prepared.clone());
                prepared
            };
            // The route proof is lexical and all callbacks below are pure or
            // try-only. It cannot escape in facts or survive the later await.
            let source_phone = i.source_phone.clone();
            // The pending model ticket is deliberately not Active. Retain the
            // SAME incarnation's actual activated resource view, validated by
            // this planning read, before any read/role/SIP aliases are dropped.
            // This does not commit the Participant replacement model.
            let capture = |invocation: &Invocation| {
                // Current-owner reads may refuse Busy. Complete them BEFORE
                // Core capture stages the once-only managed model token.
                let binding = invocation
                    .owner(started.policy.pending.binding().connection())
                    .map_err(ParticipantControlError::Media)?
                    .connection;
                self.core
                    .capture_staged_replacement_cut_facts(
                        &plan,
                        invocation.deadline,
                        cancel.clone(),
                        invocation,
                    )
                    .map(|facts| (facts, binding))
            };
            let facts = if let Some(phone) = source_phone.as_ref() {
                let context = phone
                    .final_route_context()
                    .map_err(|_| Error::forbidden())?;
                let scope = rvoip_sip::adapter::SipManagedRouteScope::new(
                    prepared.attempt_binding(),
                    phone.pending.intent().replacement.clone(),
                    phone.pending.generation(),
                    rvoip_sip::adapter::SipManagedRouteRole::ReplacementSource,
                )
                .map_err(|_| Error::forbidden())?;
                if route_resource.is_none() {
                    route_resource = Some(context.capture_managed_route_resource());
                }
                context.try_with_managed_final_route(
                    route_resource.as_ref().expect("outer route resource"),
                    &scope,
                    |proof| {
                        i.final_turn = true;
                        i.final_sip_route = Some(proof.view());
                        capture(&i)
                    },
                )
            } else {
                Ok(capture(&i))
            };
            // Includes every clone installed in owner/control callback leases.
            // No physical/codec preparation or retry sleep may retain these.
            drop(i);
            drop(roles);
            let (facts, adoption_binding) = match facts {
                Err(failure) => {
                    let retry = failure.is_busy();
                    failure.finish();
                    if retry {
                        self.managed_cut_metadata_pause(until, &cancel).await?;
                        continue;
                    }
                    return Err(Error::forbidden());
                }
                Ok(Err(error)) if busy(&error) => {
                    self.managed_cut_metadata_pause(until, &cancel).await?;
                    continue;
                }
                Ok(Err(error)) => {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-capture-core-facts",?error,"Managed replacement cold stage refused");
                    return Err(super::super::projection::core_plan_error(error));
                }
                Ok(Ok(facts)) => facts,
            };
            let adoption = self
                .media
                .prepare_replacement_adoption_metadata(
                    &started.policy,
                    prepared.activation().clone(),
                    adoption_binding,
                )
                .map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-adoption-metadata",?error,"Managed replacement cold stage refused");
                    Error::forbidden()
                })?;
            let cut = self
                .core
                .prepare_staged_managed_cut(facts)
                .await
                .map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",stage="managed-cold-prepare-core-cut",?error,"Managed replacement cold stage refused");
                    super::super::projection::core_plan_error(error)
                })?;
            #[cfg(feature="test-support")]
            let control_probe=self.media.take_preparation_custody_probe_for_test(
                root.operation_id(),
                crate::canonical_media::preparation_custody_fixture::PreparationCustodyPhase::ManagedFinalPrepared)
                .or_else(||self.media.take_preparation_custody_probe_for_test(root.operation_id(),crate::canonical_media::preparation_custody_fixture::PreparationCustodyPhase::ManagedPostPanic));
            #[cfg(feature = "test-support")]
            if let Some(probe) = &control_probe {
                probe.install_native_begin_observation(prepared.attempt_binding());
                probe.pause(&self.stop, &cancel, until, None).await;
            }
            return Ok(ColdCommit {
                plan,
                metadata: prepared,
                prepared: cut,
                adoption,
                source_phone,
                route_resource,
                #[cfg(feature = "test-support")]
                control_probe,
            });
        }
    }
}
