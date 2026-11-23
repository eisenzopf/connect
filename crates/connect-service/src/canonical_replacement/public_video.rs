//! Original public-video setup metadata; never an audience or packet grant.
use super::*;
use connect_store::NativeReplacementPublicVideoReservation;
use std::sync::OnceLock;
mod bridge;
mod bridge_activation;
pub(super) mod bridge_journal;
pub(super) mod installation;
mod negotiation;
mod observation;
mod pipeline;
mod post_cut_observation;
mod ready;
mod reservation;
#[cfg(feature = "test-support")]
mod test_support;
use bridge::Bridge;
use connect_protocol::webrtc_public_video_phase as wire;
use pipeline::Cut;

/// This state belongs to one original Root; its test-only shadow shares the
/// SAME Arc. It cannot opt in later or mint a new identity after expiry.
pub(super) struct Preparation {
    #[cfg(feature = "test-support")]
    decoded_metadata_probe: std::sync::Mutex<Option<std::sync::Weak<test_support::Probe>>>,
    required: bool,
    bridge_journal: bridge_journal::Journal,
    private_nonce: OnceLock<String>,
    cut: OnceLock<Cut>,
    // Retain the exact plan/freeze slot BEFORE Core can freeze OLD. A late
    // refusal may leave actual Frozen custody without a successful public Cut.
    original_freeze_plan:
        OnceLock<rvoip_core::orchestrator::participant_control::ManagedAudioDeviceReplacementPlan>,
    // Historical SAME-F metadata for dedicated Closed+fresh-OLD restoration
    // only. It cannot supply a Cut, Warm owner, public phase or native receipt.
    cleanup_frontier:
        OnceLock<Arc<rvoip_core::staged_streams::old_video_freeze::FrozenOldVideoFrontier>>,
    bridge: Bridge,
    bridge_scope: OnceLock<
        Arc<rvoip_core::staged_streams::direct_subscriptions::PreCutPublicVideoBridgeScope>,
    >,
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
            bridge_journal: bridge_journal::Journal::new(),
            private_nonce: OnceLock::new(),
            cut: OnceLock::new(),
            original_freeze_plan: OnceLock::new(),
            cleanup_frontier: OnceLock::new(),
            bridge: Bridge::default(),
            bridge_scope: OnceLock::new(),
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
    /// Own the exact Core route before the original candidate writer starts.
    /// A transport name or caller's readiness cannot register a route.
    pub(super) fn own_bridge_scope(
        &self,
        root: &PreparedReplacement,
        scope: Arc<rvoip_core::staged_streams::direct_subscriptions::PreCutPublicVideoBridgeScope>,
    ) -> Result<()> {
        if !self.required
            || scope.original_intent() != root.handle.intent()
            || scope.target() != self.reserved()?.target()
            || scope.conversation().as_str() != root.origin.request().conversation_id
            || scope.session().as_str() != root.origin.request().session_id
        {
            scope.close();
            return Err(Error::forbidden());
        }
        match self.bridge_scope.set(scope) {
            Ok(()) => Ok(()),
            Err(same) if self.bridge_matches(same.as_ref()) => Ok(()),
            Err(other) => {
                other.close();
                Err(Error::forbidden())
            }
        }
    }

    pub(super) fn bridge_matches(
        &self,
        scope: &rvoip_core::staged_streams::direct_subscriptions::PreCutPublicVideoBridgeScope,
    ) -> bool {
        self.bridge_scope
            .get()
            .is_some_and(|original| std::ptr::eq(original.as_ref(), scope))
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
        self.cut
            .get()
            .map(|cut| cut.plan.clone())
            .or_else(|| self.original_freeze_plan.get().cloned())
    }
    pub(super) fn own_original_freeze_plan(
        &self,
        root: &PreparedReplacement,
        plan: &rvoip_core::orchestrator::participant_control::ManagedAudioDeviceReplacementPlan,
    ) -> Result<()> {
        let original = root.handle.intent();
        let request = plan.media().request();
        if !self.required()
            || !std::ptr::eq(self, root.public_video.as_ref())
            || request.operation_id != original.operation_id
            || request.source != original.source
            || request.admitted != original.replacement
            || request.retained != original.retained
        {
            return Err(Error::forbidden());
        }
        if let Some(saved) = self.original_freeze_plan.get() {
            return if saved.same_intent(plan) {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        match self.original_freeze_plan.set(plan.clone()) {
            Ok(()) => Ok(()),
            Err(plan) => {
                if self
                    .original_freeze_plan
                    .get()
                    .is_some_and(|saved| saved.same_intent(&plan))
                {
                    Ok(())
                } else {
                    Err(Error::forbidden())
                }
            }
        }
    }
    // Called only after SAME actual candidate Closed and durable Discarded.
    // Core supplies the actual slot-owned Arc; matching scalar IDs alone never
    // construct or substitute this opaque frontier.
    pub(super) fn own_cleanup_frontier(
        &self,
        root: &PreparedReplacement,
        frontier: &Arc<rvoip_core::staged_streams::old_video_freeze::FrozenOldVideoFrontier>,
    ) -> Result<()> {
        let original = root.handle.intent();
        if !std::ptr::eq(self, root.public_video.as_ref())
            || self.original_freeze_plan.get().is_none()
            || frontier.operation_id() != original.operation_id
            || frontier.source_connection() != &original.source
            || frontier.target_connection() != &original.replacement
            || frontier.realm_incarnation() != self.reserved()?.realm_incarnation()
            || self
                .cut
                .get()
                .is_some_and(|cut| !Arc::ptr_eq(frontier, &cut.frontier))
        {
            return Err(Error::forbidden());
        }
        if let Some(saved) = self.cleanup_frontier.get() {
            return if Arc::ptr_eq(saved, frontier) {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        match self.cleanup_frontier.set(frontier.clone()) {
            Ok(()) => Ok(()),
            Err(frontier) => {
                if self
                    .cleanup_frontier
                    .get()
                    .is_some_and(|saved| Arc::ptr_eq(saved, &frontier))
                {
                    Ok(())
                } else {
                    Err(Error::forbidden())
                }
            }
        }
    }
    // ONLY RestoreInvocation::control calls this; that caller independently
    // verifies actual SAME Closed, original binding and fresh OLD/source auth.
    pub(super) fn original_restore_frontier(
        &self,
        frontier: &rvoip_core::staged_streams::old_video_freeze::FrozenOldVideoFrontier,
    ) -> bool {
        self.cut
            .get()
            .is_some_and(|cut| std::ptr::eq(frontier, Arc::as_ref(&cut.frontier)))
            || self
                .cleanup_frontier
                .get()
                .is_some_and(|original| std::ptr::eq(frontier, Arc::as_ref(original)))
    }
    // Only the exact Core metadata contention may await another snapshot;
    // its original two-second cutoff never renews across cleanup intervals.
    pub(super) fn original_freeze_custody_error(
        &self,
        error: rvoip_core::orchestrator::participant_control::ParticipantControlError,
    ) -> Error {
        if super::projection::is_core_metadata_busy(&error) {
            self.original_freeze_custody_pending()
        } else {
            Error::new(
                "cleanup-incomplete",
                "Original OLD freeze custody could not be confirmed",
            )
        }
    }
    pub(super) fn original_freeze_custody_pending(&self) -> Error {
        if tokio::time::Instant::now() < self.original_restore_deadline() {
            Error::new(
                "authority-busy",
                "Original OLD freeze custody is still pending",
            )
        } else {
            Error::new(
                "cleanup-incomplete",
                "Original OLD freeze custody cutoff elapsed",
            )
        }
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

#[cfg(test)]
mod original_freeze_custody_tests {
    use super::*;
    use rvoip_core::orchestrator::participant_control::ParticipantControlError;
    use rvoip_core::staged_streams::StreamControlError;

    #[test]
    fn original_freeze_cleanup_metadata_keeps_cutoff_and_never_supplies_public_cut_or_grant() {
        // Service error/cutoff contract only. Core's separate real slot/State
        // controls produce the opaque custody/actual F; this test fabricates
        // no plan, frontier, Closed token or restoration authority.
        let owner = Preparation::new(true);
        let cutoff = owner.original_restore_deadline();
        for _ in 0..3 {
            assert_eq!(
                owner.original_freeze_custody_pending().code,
                "authority-busy"
            );
            assert_eq!(
                owner
                    .original_freeze_custody_error(ParticipantControlError::Media(
                        StreamControlError::MetadataBusy
                    ))
                    .code,
                "authority-busy"
            );
            assert_eq!(
                owner.original_restore_deadline(),
                cutoff,
                "same original budget, never renewed"
            );
        }
        for error in [
            StreamControlError::NotReady,
            StreamControlError::Authority,
            StreamControlError::Cancelled,
            StreamControlError::Deadline,
            StreamControlError::Backpressure,
            StreamControlError::Unsupported,
        ] {
            assert_eq!(
                owner.original_freeze_custody_error(error.into()).code,
                "cleanup-incomplete"
            );
        }
        let expired = Preparation::new(true);
        expired
            .restore_deadline
            .set(tokio::time::Instant::now())
            .unwrap();
        let original = expired.original_restore_deadline();
        assert_eq!(
            expired.original_freeze_custody_pending().code,
            "cleanup-incomplete"
        );
        assert_eq!(
            expired
                .original_freeze_custody_error(ParticipantControlError::Media(
                    StreamControlError::MetadataBusy
                ))
                .code,
            "cleanup-incomplete"
        );
        assert_eq!(expired.original_restore_deadline(), original);
        for preparation in [&owner, &expired] {
            assert!(preparation.cut.get().is_none());
            assert!(preparation.original_freeze_plan.get().is_none());
            assert!(preparation.cleanup_frontier.get().is_none());
            assert!(preparation.warm.get().is_none());
            assert!(preparation.original_restore_plan().is_none());
            assert!(!preparation.old_restored());
            assert!(!preparation.ready_recorded.load(Ordering::Acquire));
            assert!(preparation.require_native_keyframe_submission().is_err());
        }
    }
}
