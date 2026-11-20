//! Grant-free cold preparation for the existing native-to-telephone mode.
use super::*;
use rvoip_core::orchestrator::participant_control::{
    ManagedAudioDeviceReplacementPlan, ParticipantControlError, PreparedManagedCut,
};
use rvoip_core::staged_streams::StreamControlError;
pub(super) struct ColdCommit {
    pub plan: ManagedAudioDeviceReplacementPlan,
    pub metadata: Arc<connect_store::TelephoneReplacementCommitPreparation>,
    pub prepared: PreparedManagedCut,
    pub route_resource: rvoip_sip::adapter::SipManagedRouteResource,
    #[cfg(feature = "test-support")]
    pub control_probe: Option<Arc<crate::canonical_media::preparation_custody_fixture::Probe>>,
}
impl Runtime {
    pub(super) async fn prepare_telephone_managed_cut(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        started: &Arc<media::Started>,
        until: tokio::time::Instant,
    ) -> Result<ColdCommit> {
        let plan = self
            .core
            .plan_staged_device_replacement(&root.handle, None)
            .map_err(|_| Error::forbidden())?;
        let context = started
            .policy
            .final_route_context()
            .map_err(|_| Error::forbidden())?;
        let route_resource = context.capture_managed_route_resource();
        let mut metadata: Option<Arc<connect_store::TelephoneReplacementCommitPreparation>> = None;
        loop {
            let cancel = root.handle.cancellation();
            if self.stop.is_cancelled()
                || cancel.is_cancelled()
                || tokio::time::Instant::now() >= until
            {
                return Err(Error::forbidden());
            }
            let roles = self.role_projection.lock().await;
            let mut i = self
                .telephone_scene_until(root, started, Phase::Commit(plan.clone()), until)
                .await?;
            let read = i.read.clone();
            let store = self.media.store.clone();
            let prepared = if let Some(prepared) = metadata.as_ref() {
                let same = prepared.clone();
                store_call(move || {
                    store.revalidate_prepared_telephone_replacement_commit(&same, &read)
                })
                .await?;
                prepared.clone()
            } else {
                let prepared = Arc::new(
                    store_call(move || store.prepare_telephone_replacement_commit(&read)).await?,
                );
                metadata = Some(prepared.clone());
                prepared
            };
            // Current Store scene eligibility requires the existing old/native
            // sources. This does not introduce a new telephone-to-telephone mode.
            let scope = rvoip_sip::adapter::SipManagedRouteScope::new(
                prepared.attempt_binding(),
                started.policy.pending.intent().replacement.clone(),
                started.policy.pending.generation(),
                rvoip_sip::adapter::SipManagedRouteRole::ReplacementTarget,
            )
            .map_err(|_| Error::forbidden())?;
            let facts = context.try_with_managed_final_route(&route_resource, &scope, |proof| {
                i.final_turn = true;
                i.final_sip_route = Some(proof.view());
                self.core.capture_staged_replacement_cut_facts(
                    &plan,
                    i.deadline,
                    cancel.clone(),
                    &i,
                )
            });
            // Even a fatal notification is consumed only after every actual
            // planning alias and role fence drops, never under route guards.
            drop(i);
            drop(roles);
            let facts = match facts {
                Err(failure) => {
                    let retry = failure.is_busy();
                    failure.finish();
                    if retry {
                        self.managed_cut_metadata_pause(until, &cancel).await?;
                        continue;
                    }
                    return Err(Error::forbidden());
                }
                Ok(Err(ParticipantControlError::Media(
                    StreamControlError::MetadataBusy | StreamControlError::Backpressure,
                ))) => {
                    self.managed_cut_metadata_pause(until, &cancel).await?;
                    continue;
                }
                Ok(Err(_)) => return Err(Error::forbidden()),
                Ok(Ok(facts)) => facts,
            };
            let cut = self
                .core
                .prepare_staged_managed_cut(facts)
                .await
                .map_err(|_| Error::forbidden())?;
            #[cfg(feature="test-support")]
            let control_probe=self.media.take_preparation_custody_probe_for_test(
                root.operation_id(),
                crate::canonical_media::preparation_custody_fixture::PreparationCustodyPhase::ManagedFinalPrepared)
                .or_else(||self.media.take_preparation_custody_probe_for_test(root.operation_id(),crate::canonical_media::preparation_custody_fixture::PreparationCustodyPhase::ManagedPostPanic));
            #[cfg(feature = "test-support")]
            if let Some(probe) = &control_probe {
                probe.pause(&self.stop, &cancel, until, None).await;
            }
            return Ok(ColdCommit {
                plan,
                metadata: prepared,
                prepared: cut,
                route_resource,
                #[cfg(feature = "test-support")]
                control_probe,
            });
        }
    }
}
