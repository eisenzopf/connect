//! Admission and custody of one original pre-cut public-programme route.
use super::super::{continuation::Phase, movement, projection, start::StartedReplacement};
use super::*;
use rvoip_core::{
    orchestrator::participant_control::ManagedAudioDeviceReplacementPlan,
    staged_streams::{
        direct_subscriptions::PreCutPublicVideoBridgeScope,
        public_video_reservation::PreCutPublicVideoBridgeOwnership,
    },
};
use tokio::time::Instant;

#[derive(Default)]
pub(super) struct Bridge {
    deadline: OnceLock<Instant>,
    pub(super) output: OnceLock<Arc<rvoip_webrtc::media::core_video::ReceiverReleaseObservation>>,
    plan: OnceLock<ManagedAudioDeviceReplacementPlan>,
    ownership: OnceLock<Arc<PreCutPublicVideoBridgeOwnership>>,
}

impl Preparation {
    pub(super) fn pre_cut_bridge_plan(&self) -> Result<&ManagedAudioDeviceReplacementPlan> {
        self.bridge.plan.get().ok_or_else(Error::forbidden)
    }

    pub(in crate::canonical_replacement) fn bridge_resource_matches(
        &self,
        scope: &PreCutPublicVideoBridgeScope,
    ) -> bool {
        self.bridge
            .ownership
            .get()
            .is_some_and(|original| scope.matches_ownership(original))
    }

    pub(in crate::canonical_replacement) fn close_bridge(&self) {
        if let Some(scope) = self.bridge_scope.get() {
            scope.close();
        }
    }
}

impl Runtime {
    /// The negotiated bridge lifecycle calls this before starting the original
    /// writer. It owns the same plan and reserved resource across setup contention.
    pub(in crate::canonical_replacement) async fn prepare_pre_cut_public_video_bridge(
        &self,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
    ) -> Result<Arc<PreCutPublicVideoBridgeScope>> {
        let preparation = &root.public_video;
        if !preparation.required()
            || root.handle.cancellation().is_cancelled()
            || connect_protocol::webrtc_public_video_bridge::OptIn::from_request(
                root.origin.request(),
            )?
            .is_none()
        {
            return Err(Error::forbidden());
        }
        let bridge = &preparation.bridge;
        let until = *bridge
            .deadline
            .get_or_init(|| root.deadline.min(Instant::now() + Duration::from_secs(5)));
        let (roles, scope) = projection::read(&self.role_projection, &self.stop, until, || async {
            if root.handle.cancellation().is_cancelled() {
                return Err(Error::forbidden());
            }
            if let Some(scope) = preparation.bridge_scope.get() {
                return scope
                    .is_current()
                    .then(|| scope.clone())
                    .ok_or_else(Error::forbidden);
            }
            if bridge.plan.get().is_none() {
                let plan = self
                    .core
                    .plan_staged_device_replacement(
                        &root.handle,
                        movement::replacement_video(&root.plan)?,
                    )
                    .map_err(projection::core_plan_error)?;
                if plan.media().private_predecessor().is_some() {
                    return Err(Error::forbidden());
                }
                bridge.plan.set(plan).map_err(|_| Error::forbidden())?;
            }
            let plan = bridge.plan.get().ok_or_else(Error::forbidden)?;
            if bridge.ownership.get().is_none() {
                let control = self
                    .continuation_read_once(root, started, Phase::ReserveVideo)
                    .await?;
                let resource = self
                    .core
                    .prepare_staged_public_video_bridge_ownership_with_held(plan, &control)
                    .map_err(projection::core_plan_error)?;
                // Store before any fallible read/route attempt. It is never
                // re-minted on retry and its Drop cannot reopen raw access.
                bridge
                    .ownership
                    .set(Arc::new(resource))
                    .map_err(|_| Error::forbidden())?;
                drop(control);
            }
            let resource = bridge.ownership.get().ok_or_else(Error::forbidden)?;
            let control = self
                .continuation_read_once(root, started, Phase::BridgeVideo(plan.clone()))
                .await?;
            let scope = self
                .core
                .prepare_staged_pre_cut_public_video_bridge_with_held(plan, resource, &control)
                .map_err(projection::core_plan_error)?;
            drop(control);
            preparation.own_bridge_scope(root, scope.clone())?;
            // Pin the actual route on Root before this fallible currentness
            // observation. A retired route cannot be reminted on retry.
            scope
                .is_current()
                .then_some(scope)
                .ok_or_else(Error::forbidden)
        })
        .await?;
        drop(roles);
        Ok(scope)
    }
}
