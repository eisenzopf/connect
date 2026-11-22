//! One original F, one B warm owner, one decoded observation, one publication.
use super::super::{continuation::Phase, start::StartedReplacement};
use super::*;
use connect_store::NativeReplacementPublicVideoStage as Stage;
use rvoip_core::{
    orchestrator::participant_control::ManagedAudioDeviceReplacementPlan,
    staged_streams::{old_video_freeze::FrozenOldVideoFrontier, StreamControlError},
};

pub(super) struct Cut {
    pub(super) frontier: Arc<FrozenOldVideoFrontier>,
    pub(super) plan: ManagedAudioDeviceReplacementPlan,
    pub(super) id: String,
    pub(super) nonce: String,
}
pub(super) fn stream_error(error: StreamControlError) -> Error {
    match error {
        StreamControlError::NotReady => {
            Error::new("authority-busy", "Original public video metadata is busy")
        }
        _ => Error::forbidden(),
    }
}
// Fixed enum classifications only: never format nested errors or private text.
fn stream_kind(error: &StreamControlError) -> &'static str {
    match error {
        StreamControlError::Model(_) => "media-model",
        StreamControlError::Unsupported => "media-unsupported",
        StreamControlError::Authority => "media-authority",
        StreamControlError::NotRecipient => "media-not-recipient",
        StreamControlError::Cancelled => "media-cancelled",
        StreamControlError::Deadline => "media-deadline",
        StreamControlError::NotReady => "media-not-ready",
        StreamControlError::MetadataBusy => "media-metadata-busy",
        StreamControlError::Backpressure => "media-backpressure",
        StreamControlError::CleanupUnconfirmed => "media-cleanup-unconfirmed",
    }
}
fn participant_kind(
    error: &rvoip_core::orchestrator::participant_control::ParticipantControlError,
) -> &'static str {
    use rvoip_core::orchestrator::participant_control::ParticipantControlError as E;
    use rvoip_core::participant_transitions::ParticipantTransitionError as T;
    match error {
        E::Transition(error) => match error {
            T::Invalid => "participant-transition-invalid",
            T::Conflict => "participant-transition-conflict",
            T::Inactive => "participant-transition-inactive",
            T::OwnerMismatch => "participant-transition-owner-mismatch",
            T::NotReady => "participant-transition-not-ready",
            T::Capacity => "participant-transition-capacity",
            T::Ended => "participant-transition-ended",
        },
        E::Core(_) => "participant-core",
        E::Media(error) => stream_kind(error),
        E::Unsupported => "participant-unsupported",
        E::Authority => "participant-authority",
        E::Cancelled => "participant-cancelled",
        E::Deadline => "participant-deadline",
    }
}
fn terminal(root: &PreparedReplacement, stage: &'static str, kind: &str) {
    tracing::debug!(target:"connect_service::original_replacement_terminal",
        boundary="public-video-cut-terminal",stage,error_kind=kind,
        operation=%root.origin.request().operation_id,record=%root.origin.record_id(),
        connection=%root.handle.intent().replacement,"Original replacement terminal boundary");
}

// Only a linear Core-issued pre-entry metadata token authorizes this wait.
// The token has already released all policy/IO aliases and keeps its earliest
// original100ms+observed grant cutoff; it is never dropped/recreated for retry.
async fn finish_warm_invocation<'a>(
    runtime: &Runtime,
    root: &PreparedReplacement,
    until: tokio::time::Instant,
    mut attempt: rvoip_core::staged_streams::ControlResult<
        rvoip_core::staged_streams::direct_subscriptions::PublicVideoWarmInvocation<'a>,
    >,
) -> rvoip_core::staged_streams::ControlResult<()> {
    use rvoip_core::staged_streams::direct_subscriptions::PublicVideoWarmInvocation;
    let cancel = root.handle.cancellation();
    loop {
        match attempt? {
            PublicVideoWarmInvocation::Complete => return Ok(()),
            PublicVideoWarmInvocation::DefinitelyUnstarted(retry) => {
                let deadline = until.min(retry.deadline());
                if runtime.stop.is_cancelled()
                    || cancel.is_cancelled()
                    || retry.cancellation().is_cancelled()
                {
                    return Err(StreamControlError::Cancelled);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(StreamControlError::Deadline);
                }
                tokio::select! {biased;
                    _=runtime.stop.cancelled()=>return Err(StreamControlError::Cancelled),
                    _=cancel.cancelled()=>return Err(StreamControlError::Cancelled),
                    _=retry.cancellation().cancelled()=>return Err(StreamControlError::Cancelled),
                    _=tokio::time::sleep_until(deadline)=>return Err(StreamControlError::Deadline),
                    _=tokio::time::sleep_until(deadline.min(tokio::time::Instant::now()+std::time::Duration::from_millis(2)))=>{},
                }
                attempt = retry.retry().await;
            }
        }
    }
}

impl Preparation {
    pub(super) fn binding(&self) -> Result<wire::Binding> {
        let native = self.negotiated.get().ok_or_else(Error::forbidden)?;
        let intent = self.reservation()?.intent();
        let value = wire::Binding {
            stream_id: native.native.logical_stream_id.as_str().into(),
            preview_stream_id: intent.preview_stream_id.clone(),
            mid: native.native.mid.clone(),
            stream_generation: native.stream_generation,
            physical_writer_incarnation: format!("{:032x}", native.physical_writer_incarnation),
            connection_generation: native.connection_generation,
            primary_ssrc: native.native.primary_ssrc,
            payload_type: native.native.payload_type,
            clock_rate_hz: native.native.clock_rate_hz,
            codec: "vp8".into(),
        };
        value.validate(intent)?;
        Ok(value)
    }
    pub(super) fn context<'a>(
        &'a self,
        root: &'a PreparedReplacement,
        private_nonce: &'a str,
        completed: bool,
    ) -> Result<wire::Context<'a>> {
        let reservation = self.reservation()?;
        Ok(wire::Context {
            intent: reservation.intent(),
            operation_record_id: root.origin.record_id(),
            operation_id: &root.origin.request().operation_id,
            target_connection_id: reservation.candidate_connection_id(),
            target_generation: reservation.target_generation(),
            private_readiness_nonce: private_nonce,
            completed,
        })
    }
    pub(super) fn phase(
        &self,
        root: &PreparedReplacement,
        private_nonce: &str,
        completed: bool,
    ) -> Result<wire::Phase> {
        let r = self.reservation()?;
        let cut = self
            .cut
            .get()
            .ok_or_else(|| Error::new("not-ready", "Original public video cut is not bound"))?;
        let phase = wire::Phase::CutBoundPostF {
            profile: wire::PROFILE.into(),
            reservation_id: r.intent().reservation_id.clone(),
            operation_record_id: root.origin.record_id().into(),
            operation_id: root.origin.request().operation_id.clone(),
            target_connection_id: r.candidate_connection_id().into(),
            target_generation: r.target_generation(),
            binding: self.binding()?,
            source_cut_id: cut.id.clone(),
            public_readiness: wire::Challenge {
                nonce: cut.nonce.clone(),
                decoder_startup: wire::DECODER_STARTUP.into(),
            },
        };
        phase.validate(&self.context(root, private_nonce, completed)?)?;
        Ok(phase)
    }
    pub(in crate::canonical_replacement) fn commit_plan(
        &self,
    ) -> Result<ManagedAudioDeviceReplacementPlan> {
        if !self.required() || !self.ready_recorded.load(Ordering::Acquire) {
            return Err(Error::new(
                "not-ready",
                "Original B decoder readiness is not durably recorded",
            ));
        }
        let cut = self.cut.get().ok_or_else(Error::forbidden)?;
        if self
            .warm
            .get()
            .is_none_or(|warm| !warm.is_current() || !warm.matches_original_plan(&cut.plan))
        {
            return Err(Error::forbidden());
        }
        Ok(cut.plan.clone())
    }
}
impl Runtime {
    pub(in crate::canonical_replacement) fn prepare_public_video_cut<'a>(
        &'a self,
        root: &'a Arc<PreparedReplacement>,
        started: &'a Arc<StartedReplacement>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if !root.public_video.required() {
                return Ok(());
            }
            let p = &root.public_video;
            let until = root
                .deadline
                .min(tokio::time::Instant::now() + Duration::from_secs(5));
            let cancel = root.handle.cancellation();
            // A definitely-unstarted retry must retain the SAME plan/freeze slot.
            // Planning may itself retry only before it returns an owned plan.
            let (roles, plan) = super::super::projection::read(
                &self.role_projection,
                &self.stop,
                until,
                || async {
                    let plan = self
                        .core
                        .plan_staged_device_replacement(
                            &root.handle,
                            super::super::movement::replacement_video(&root.plan)?,
                        )
                        .map_err(|error| {
                            // MetadataBusy is retried by projection; it is not terminal.
                            if !super::super::projection::is_core_metadata_busy(&error) {
                                terminal(root, "plan-core", participant_kind(&error));
                            }
                            super::super::projection::core_plan_error(error)
                        })?;
                    if plan.media().private_predecessor().is_some() {
                        terminal(root, "plan-private-predecessor", "refused");
                        return Err(Error::forbidden());
                    }
                    Ok(plan)
                },
            )
            .await
            .map_err(|error| {
                terminal(root, "plan-projection", &error.code);
                error
            })?;
            drop(roles);
            let frontier = loop {
                if cancel.is_cancelled() {
                    terminal(root, "freeze-root-cancelled", "cancelled");
                    return Err(Error::forbidden());
                }
                let (roles, i) = super::super::projection::read(
                    &self.role_projection,
                    &self.stop,
                    until,
                    || async {
                        self.continuation_read_once(root, started, Phase::FreezeVideo(plan.clone()))
                            .await
                    },
                )
                .await
                .map_err(|error| {
                    terminal(root, "freeze-projection", &error.code);
                    error
                })?;
                // Only complete current source Readers cover bounded original drain;
                // the global role coordinator does not cover physical IO.
                drop(roles);
                let reserved = match p.reserved() {
                    Ok(reserved) => reserved,
                    Err(error) => {
                        drop(i);
                        terminal(root, "freeze-reservation", &error.code);
                        return Err(error);
                    }
                };
                let attempt = self
                    .core
                    .try_freeze_staged_public_video_with_reservation(&plan, reserved, &i)
                    .await;
                drop(i);
                match attempt {
                    Ok(Some(frontier)) => break frontier,
                    Ok(None) => {} // Core proved SAME Unstarted slot; no pause/IO.
                    Err(error) => {
                        terminal(root, "freeze-core", participant_kind(&error));
                        tracing::debug!(target:"connect_service::canonical_replacement",?error,
                        "Original public freeze refused or already attempted");
                        return Err(Error::forbidden()); // NEVER remap late MetadataBusy
                    }
                }
                tokio::select! {biased;
                    _=self.stop.cancelled()=>return Err(Error::forbidden()),
                    _=cancel.cancelled()=>return Err(Error::forbidden()),
                    _=tokio::time::sleep_until(until)=>return Err(Error::new("authority-busy","Original freeze admission cutoff elapsed")),
                    _=tokio::time::sleep(Duration::from_millis(20))=>{},
                }
            };
            // Own SAME F immediately, including through any later metadata refusal.
            // IDs name the owned Core F; no u64 ordinal is exposed to JavaScript.
            p.cut
                .set(Cut {
                    frontier,
                    plan: plan.clone(),
                    id: format!("source-cut-{}", uuid::Uuid::new_v4()),
                    nonce: format!("public-ready-{}", uuid::Uuid::new_v4()),
                })
                .map_err(|_| {
                    terminal(root, "cut-slot-already-set", "refused");
                    Error::forbidden()
                })?;
            let frontier = &p
                .cut
                .get()
                .ok_or_else(|| {
                    terminal(root, "cut-slot-missing", "refused");
                    Error::forbidden()
                })?
                .frontier;
            if frontier.operation_id() != root.handle.intent().operation_id
                || frontier.source_connection() != &root.handle.intent().source
                || frontier.target_connection() != &root.handle.intent().replacement
                || frontier.realm_incarnation() != p.reserved()?.realm_incarnation()
            {
                terminal(root, "frontier-original-scope", "refused");
                return Err(Error::forbidden());
            }
            let (roles, ()) = super::super::projection::read(
                &self.role_projection,
                &self.stop,
                until,
                || async {
                    let i = self
                        .continuation_read_once(root, started, Phase::WarmVideo(plan.clone()))
                        .await?;
                    match self
                        .core
                        .try_prepare_staged_public_video_warm_with_held(&plan, &i)
                    {
                        Ok(Some(warm)) => {
                            // Own actual original B BEFORE releasing even this setup
                            // interval, then before any native task/SQL observation.
                            if let Err(error) = p.own_warm(root, &plan, warm) {
                                drop(i);
                                terminal(root, "warm-owned-scope", &error.code);
                                return Err(error);
                            }
                            Ok(())
                        }
                        Ok(None) => Err(Error::new(
                            "authority-busy",
                            "Original B warm setup is definitely unattempted",
                        )),
                        Err(error) => {
                            drop(i);
                            terminal(root, "warm-core", participant_kind(&error));
                            tracing::debug!(target:"connect_service::canonical_replacement",?error,
                        "Original B warm setup refused or already attempted");
                            Err(Error::forbidden()) // not a generic MetadataBusy retry
                        }
                    }
                },
            )
            .await
            .map_err(|error| {
                terminal(root, "warm-projection", &error.code);
                error
            })?;
            drop(roles);
            let warm = p.warm.get().ok_or_else(|| {
                terminal(root, "warm-owner-missing", "refused");
                Error::forbidden()
            })?;
            finish_warm_invocation(self, root, until, warm.start().await)
                .await
                .map_err(|error| {
                    terminal(root, "warm-start", stream_kind(&error));
                    stream_error(error)
                })?;
            finish_warm_invocation(self, root, until, warm.request_initial_picture().await)
                .await
                .map_err(|error| {
                    terminal(root, "initial-picture", stream_kind(&error));
                    stream_error(error)
                })?;
            self.record_public_video_phase(root, started, Stage::CutBoundPostF)
                .await
                .map_err(|error| {
                    terminal(root, "cut-bound-phase-publication", &error.code);
                    error
                })?;
            p.cut_recorded.store(true, Ordering::Release);
            loop {
                // Register before checking, so a simultaneous original Ready cannot
                // be lost. Waiting owns no Store Reader, role mutex or packet grant.
                let notified = p.ready_signal.notified();
                tokio::pin!(notified);
                // Seeing the marker alone does not prove its capturing handler
                // released signed Reader aliases. Pair Acquire with that winner's
                // post-drop Release before any standalone metadata admission.
                if p.ready_readers_released.load(Ordering::Acquire) && p.ready.get().is_some() {
                    break;
                }
                let cancel = root.handle.cancellation();
                tokio::select! {biased;
                    _=self.stop.cancelled()=>return Err(Error::forbidden()),
                    _=cancel.cancelled()=>return Err(Error::forbidden()),
                    _=tokio::time::sleep_until(root.deadline)=>return Err(Error::new("expired","Original B decoder cutoff elapsed")),
                    _=&mut notified=>{},
                }
            }
            self.record_public_video_phase(root, started, Stage::DecodedReady)
                .await
                .map_err(|error| {
                    terminal(root, "decoded-ready-phase-publication", &error.code);
                    error
                })?;
            p.ready_recorded.store(true, Ordering::Release);
            Ok(())
        })
    }
}
