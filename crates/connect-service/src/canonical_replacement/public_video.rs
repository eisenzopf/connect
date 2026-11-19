//! Original public-video setup metadata; never an audience or packet grant.
use super::*;
use connect_store::NativeReplacementPublicVideoReservation;
use std::sync::OnceLock;
pub(super) mod installation;
mod negotiation;
mod observation;
mod pipeline;
mod ready;
mod reservation;
#[cfg(feature = "test-support")]
mod test_support;
use connect_protocol::webrtc_public_video_phase as wire;
use pipeline::Cut;

/// This state belongs to one original Root; its test-only shadow shares the
/// SAME Arc. It cannot opt in later or mint a new identity after expiry.
pub(super) struct Preparation {
    #[cfg(feature = "test-support")]
    decoded_metadata_probe: std::sync::Mutex<Option<std::sync::Weak<test_support::Probe>>>,
    required: bool,
    private_nonce: OnceLock<String>,
    cut: OnceLock<Cut>,
    ready: OnceLock<wire::ReadyMarker>,
    // Release publication belongs ONLY to the original capture winner, after
    // that handler drops its actual admitted signed Readers. No media grant.
    ready_readers_released: AtomicBool,
    ready_signal: tokio::sync::Notify,
    ready_recorded: AtomicBool,
    discard_reason: OnceLock<wire::DiscardReason>,
    old_restored: AtomicBool,
    discarded_recorded: AtomicBool,
    restore_deadline: OnceLock<tokio::time::Instant>,
    cut_recorded: AtomicBool,
    reservation: OnceLock<Arc<NativeReplacementPublicVideoReservation>>,
    reserved: OnceLock<
        Arc<
            rvoip_core::staged_streams::public_video_reservation::PublicVideoReservationPreparation,
        >,
    >,
    negotiated: OnceLock<rvoip_webrtc::canonical::ReservedVideoPortBinding>,
    warm:
        OnceLock<Arc<rvoip_core::staged_streams::direct_subscriptions::PublicVideoWarmPreparation>>,
}
impl Preparation {
    // Only the cold Store classifier supplies this immutable original choice.
    // Actual atomic recording rechecks current facts before issuing a token.
    pub(super) fn new(required: bool) -> Self {
        Self {
            #[cfg(feature = "test-support")]
            decoded_metadata_probe: Default::default(),
            required,
            private_nonce: OnceLock::new(),
            cut: OnceLock::new(),
            ready: OnceLock::new(),
            ready_readers_released: AtomicBool::new(false),
            ready_signal: tokio::sync::Notify::new(),
            ready_recorded: AtomicBool::new(false),
            discard_reason: OnceLock::new(),
            old_restored: AtomicBool::new(false),
            discarded_recorded: AtomicBool::new(false),
            restore_deadline: OnceLock::new(),
            cut_recorded: AtomicBool::new(false),
            reservation: OnceLock::new(),
            reserved: OnceLock::new(),
            negotiated: OnceLock::new(),
            warm: OnceLock::new(),
        }
    }
    pub(super) fn required(&self) -> bool {
        self.required
    }
    pub(super) fn discarded_recorded(&self) -> bool {
        self.discarded_recorded.load(Ordering::Acquire)
    }
    pub(super) fn record_discarded_observation(&self) {
        self.discarded_recorded.store(true, Ordering::Release);
    }

    // Reuse the existing two-second OLD restoration admission budget across
    // SAME-owner retries. No retry renews this control cutoff or media age.
    pub(super) fn original_restore_deadline(&self) -> tokio::time::Instant {
        *self
            .restore_deadline
            .get_or_init(|| tokio::time::Instant::now() + Duration::from_secs(2))
    }

    pub(super) fn install(
        &self,
        original_record: &str,
        reservation: NativeReplacementPublicVideoReservation,
    ) -> Result<()> {
        if !self.required || reservation.operation_record_id() != original_record {
            return Err(Error::forbidden());
        }
        if let Some(original) = self.reservation.get() {
            return if original.same_original(&reservation) {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        match self.reservation.set(Arc::new(reservation)) {
            Ok(()) => Ok(()),
            Err(same) => {
                let original = self.reservation.get().ok_or_else(Error::storage)?;
                if original.same_original(&same) {
                    Ok(())
                } else {
                    Err(Error::forbidden())
                }
            }
        }
    }
    pub(super) fn reservation(&self) -> Result<&Arc<NativeReplacementPublicVideoReservation>> {
        self.reservation
            .get()
            .ok_or_else(|| Error::new("not-ready", "Original public-video intent is not recorded"))
    }
}

impl Preparation {
    // Own the actual nonclone Core reservation BEFORE awaiting native binding
    // observation or SQL phase recording. A temporary scalar is not custody.
    pub(super) fn own_reserved(
        &self,
        root: &Arc<PreparedReplacement>,
        reserved: rvoip_core::staged_streams::public_video_reservation::PublicVideoReservationPreparation,
    ) -> Result<()> {
        let intent = self.reservation()?.intent();
        if !reserved.is_current()
            || !reserved.matches_original_preparation(&root.handle)
            || reserved.original_intent() != root.handle.intent()
            || reserved.target().logical_id.as_str() != intent.public_stream_id
            || reserved.conversation().as_str() != root.origin.request().conversation_id
            || reserved.session().as_str() != root.origin.request().session_id
        {
            return Err(Error::forbidden());
        }
        self.reserved
            .set(Arc::new(reserved))
            .map_err(|_| Error::forbidden())
    }
    pub(super) fn reserved(
        &self,
    ) -> Result<
        &Arc<
            rvoip_core::staged_streams::public_video_reservation::PublicVideoReservationPreparation,
        >,
    > {
        self.reserved
            .get()
            .ok_or_else(|| Error::new("not-ready", "Original B resource reservation is not owned"))
    }
    pub(super) fn bind_negotiated(
        &self,
        root: &Arc<PreparedReplacement>,
        native: rvoip_webrtc::canonical::ReservedVideoPortBinding,
    ) -> Result<()> {
        let reserved = self.reserved()?;
        let target = reserved.target();
        if !reserved.is_current()
            || !reserved.matches_original_preparation(&root.handle)
            || native.conversation_id != *reserved.conversation()
            || native.session_id != *reserved.session()
            || native.connection_id != root.handle.intent().replacement
            || native.native.logical_stream_id != target.logical_id
            || native.transport_stream_id != target.transport_id
            || native.stream_generation != target.stream_generation
            || native.connection_generation != target.connection_generation
            || native.physical_writer_incarnation != target.physical_writer_incarnation
            || native.realm_incarnation != reserved.realm_incarnation()
        {
            return Err(Error::forbidden());
        }
        if let Some(original) = self.negotiated.get() {
            return if original == &native {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        self.negotiated.set(native).map_err(|_| Error::forbidden())
    }
    pub(super) fn own_warm(
        &self,
        root: &Arc<PreparedReplacement>,
        plan: &rvoip_core::orchestrator::participant_control::ManagedAudioDeviceReplacementPlan,
        warm: rvoip_core::staged_streams::direct_subscriptions::PublicVideoWarmPreparation,
    ) -> Result<()> {
        let reserved = self.reserved()?;
        let scopes = warm.scopes();
        if !warm.is_current()
            || !warm.matches_original_plan(plan)
            || scopes.len() != 1
            || scopes[0].original_intent() != root.handle.intent()
            || scopes[0].target() != reserved.target()
            || scopes[0].frontier().operation_id() != root.handle.intent().operation_id
            || self
                .cut
                .get()
                .is_none_or(|cut| !std::ptr::eq(scopes[0].frontier(), Arc::as_ref(&cut.frontier)))
            || scopes[0].is_published()
            || self.negotiated.get().is_none()
        {
            return Err(Error::forbidden());
        }
        drop(scopes);
        self.warm
            .set(Arc::new(warm))
            .map_err(|_| Error::forbidden())
    }
    pub(super) fn warm_matches(
        &self,
        scope: &rvoip_core::staged_streams::direct_subscriptions::PostCutPublicVideoScope,
    ) -> bool {
        self.warm.get().is_some_and(|warm| {
            warm.is_current()
                && warm
                    .scopes()
                    .iter()
                    .any(|original| std::ptr::eq(*original, scope))
        })
    }
}

impl Preparation {
    pub(super) fn original_restore_plan(
        &self,
    ) -> Option<rvoip_core::orchestrator::participant_control::ManagedAudioDeviceReplacementPlan>
    {
        self.cut.get().map(|cut| cut.plan.clone())
    }
    pub(super) fn original_frontier(
        &self,
        frontier: &rvoip_core::staged_streams::old_video_freeze::FrozenOldVideoFrontier,
    ) -> bool {
        self.cut
            .get()
            .is_some_and(|cut| std::ptr::eq(frontier, Arc::as_ref(&cut.frontier)))
    }
    pub(super) fn original_discard_reason(
        &self,
        deadline: tokio::time::Instant,
    ) -> wire::DiscardReason {
        *self.discard_reason.get_or_init(|| {
            if tokio::time::Instant::now() >= deadline {
                wire::DiscardReason::Expired
            } else {
                wire::DiscardReason::Refused
            }
        })
    }
    pub(super) fn old_restored(&self) -> bool {
        self.old_restored.load(Ordering::Acquire)
    }
    pub(super) fn record_old_restored(&self) {
        self.old_restored.store(true, Ordering::Release);
    }
}

impl Preparation {
    // Actual successful WHOLE post-F native keyframe writes, never start,
    // sink-ready or an untrusted remote count. This immutable receipt also
    // survives the staged W3 transfer; it grants no current media authority.
    pub(super) fn require_native_keyframe_submission(&self) -> Result<()> {
        let cut = self.cut.get().ok_or_else(Error::forbidden)?;
        let warm = self.warm.get().ok_or_else(Error::forbidden)?;
        let scopes = warm.scopes();
        if scopes.len() != 1
            || !warm.matches_original_plan(&cut.plan)
            || !std::ptr::eq(scopes[0].frontier(), Arc::as_ref(&cut.frontier))
        {
            return Err(Error::forbidden());
        }
        let submitted = scopes[0].successful_keyframe_submission().ok_or_else(|| {
            Error::new(
                "authority-busy",
                "Original B keyframe has not completed its native writes",
            )
        })?;
        let native = self.negotiated.get().ok_or_else(Error::forbidden)?;
        if !submitted.matches_scope(scopes[0])
            || submitted.native_ssrc() != native.native.primary_ssrc
            || submitted.payload_type() != native.native.payload_type
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(super) fn require_commit_ready(
        &self,
        root: &PreparedReplacement,
        plan: &rvoip_core::orchestrator::participant_control::ManagedAudioDeviceReplacementPlan,
    ) -> Result<()> {
        if !self.required() {
            return Ok(());
        }
        if !self.ready_recorded.load(Ordering::Acquire) {
            return Err(Error::forbidden());
        }
        let cut = self.cut.get().ok_or_else(Error::forbidden)?;
        let warm = self.warm.get().ok_or_else(Error::forbidden)?;
        if !warm.matches_original_plan(plan) || !warm.matches_original_plan(&cut.plan) {
            return Err(Error::forbidden());
        }
        self.require_native_keyframe_submission()?;
        let marker = self.ready.get().ok_or_else(Error::forbidden)?;
        let private_nonce = self.private_nonce.get().ok_or_else(Error::forbidden)?;
        marker.validate(
            &self.context(root, private_nonce, false)?,
            &self.phase(root, private_nonce, false)?,
        )?;
        Ok(())
    }
}
