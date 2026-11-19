//! Actual original Closed plus independent fresh OLD/source permission.
use super::*;
use connect_protocol::webrtc_public_video_phase::DiscardReason;
use connect_store::{
    NativeReplacementPublicVideoClosedExpected, NativeReplacementPublicVideoClosedGuard,
    NativeReplacementPublicVideoClosedObservation, NativeVideoRestoreRead,
};
use rvoip_core::staged_streams::{
    old_video_freeze::{OldVideoRestoreResourceState, OldVideoRestoreScope},
    ControlResult, MediaAuthorityLease, MediaControl, MediaOwner, MediaPurpose, MediaScope,
    MediaSource, StreamControlError,
};
use std::collections::BTreeMap;

struct ClosedObservation {
    root: Arc<PreparedReplacement>,
    closed: Arc<ClosedOriginalCandidate>,
    reason: DiscardReason,
    adapter: Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter>,
    physical: Arc<WebRtcPhysicalBinding>,
}
struct ClosedGuard<'a> {
    owner: &'a ClosedObservation,
}
impl ClosedObservation {
    fn check(&self) -> Result<()> {
        if self.root.handle.is_committed()
            || !self.closed.matches_preparation(&self.root.handle)
            || !self
                .closed
                .matches_original_binding(&self.adapter, &self.physical)
        {
            return Err(Error::forbidden());
        }
        let owned = self
            .root
            .public_discard
            .try_lock()
            .map_err(|_| Error::new("authority-busy", "Original close metadata is busy"))?;
        if !matches!(&*owned,OwnedDiscard::Closed(original) if Arc::ptr_eq(original,&self.closed)) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl NativeReplacementPublicVideoClosedObservation for ClosedObservation {
    fn hold_closed(
        &self,
        expected: &NativeReplacementPublicVideoClosedExpected<'_>,
    ) -> Result<Option<Box<dyn NativeReplacementPublicVideoClosedGuard + '_>>> {
        self.check()?;
        if !expected.matches_original_reservation(self.root.public_video.reservation()?)
            || expected.operation_id() != self.root.origin.request().operation_id
        {
            return Err(Error::forbidden());
        }
        Ok(Some(Box::new(ClosedGuard { owner: self })))
    }
}
impl NativeReplacementPublicVideoClosedGuard for ClosedGuard<'_> {
    fn reason(&self) -> Result<DiscardReason> {
        self.owner.check()?;
        Ok(self.owner.reason)
    }
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()> {
        // The exact immutable actual Closed token and original Root are held
        // through this local metadata commit, independently of target expiry.
        self.owner.check()?;
        commit()
    }
}
#[derive(Clone)]
pub(crate) struct RestoreInvocation {
    policy: Arc<crate::canonical_media::Policy>,
    root: Arc<PreparedReplacement>,
    closed: Arc<ClosedOriginalCandidate>,
    read: Arc<NativeVideoRestoreRead>,
    old: Arc<crate::canonical_media::Connection>,
    adapter: Arc<rvoip_webrtc::canonical::CanonicalWebRtcAdapter>,
    physical: Arc<WebRtcPhysicalBinding>,
    selected: BTreeMap<String, Arc<crate::canonical_media::Connection>>,
    hints: Arc<[MediaSource]>,
    deadline: tokio::time::Instant,
    cancel: CancellationToken,
}
impl RestoreInvocation {
    pub(crate) fn belongs_to(&self, policy: &crate::canonical_media::Policy) -> bool {
        std::ptr::eq(policy, Arc::as_ptr(&self.policy))
    }
    fn check(&self) -> ControlResult<()> {
        if !self.read.current()
            || self.root.handle.is_committed()
            || !self.closed.matches_preparation(&self.root.handle)
            || !self
                .closed
                .matches_original_binding(&self.adapter, &self.physical)
        {
            return Err(StreamControlError::Authority);
        }
        if self.cancel.is_cancelled() {
            return Err(StreamControlError::Cancelled);
        }
        if tokio::time::Instant::now() >= self.deadline {
            return Err(StreamControlError::Deadline);
        }
        Ok(())
    }
    fn owner(&self, route: bool) -> ControlResult<MediaOwner> {
        self.check()?;
        self.old.owner(
            self.read.old().target(),
            &self.old.principal,
            route && self.read.old().permits_subscriptions(),
        )
    }
    pub(crate) fn control(
        &self,
        scope: &MediaScope<'_>,
        original: &OldVideoRestoreScope,
    ) -> ControlResult<MediaControl> {
        self.check()?;
        if !matches!(scope.purpose, MediaPurpose::Control)
            || scope.connection != &self.root.handle.intent().source
            || scope.conversation != original.conversation()
            || scope.session != original.session()
            || !original.matches_original_preparation(&self.root.handle)
            || original.original_intent() != self.root.handle.intent()
            || !self
                .root
                .public_video
                .original_frontier(original.frontier())
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.old.principal,
            )
            || original.selected_sources().len() != self.hints.len()
            || original.selected_sources().iter().any(|selected| {
                !self.hints.iter().any(|hint| {
                    crate::canonical_replacement::continuation::same_public_video_source(
                        selected.source(),
                        hint,
                    )
                })
            })
        {
            return Err(StreamControlError::Authority);
        }
        let lease = RestoreLease {
            owner: self.owner(false)?,
            invocation: self.clone(),
        };
        // This is NEW ordinary OLD authority, not renewed cancelled target or
        // operator permission. Core still verifies actual frozen tokens/Ports.
        MediaControl::new(Box::new(lease), self.cancel.clone(), self.deadline)
    }
}
struct RestoreLease {
    owner: MediaOwner,
    invocation: RestoreInvocation,
}
impl MediaAuthorityLease for RestoreLease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        self.invocation.owner(false).map(Box::new)
    }
    fn control_receive(&self, source: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        i.check()?;
        if !i.hints.iter().any(|hint| {
            crate::canonical_replacement::continuation::same_public_video_source(source, hint)
        }) {
            return Err(StreamControlError::Authority);
        }
        let entry = i
            .selected
            .get(source.connection.as_str())
            .ok_or(StreamControlError::Authority)?;
        let pair = i
            .read
            .received_from(source.connection.as_str())
            .ok_or(StreamControlError::Authority)?;
        crate::canonical_media::source_matches(
            source,
            entry,
            pair.source().ok_or(StreamControlError::Authority)?,
        )?;
        // Exact pair includes current independent recipient ceilings and named
        // subscribe permission, not just the OLD unary credential.
        i.old
            .owner(
                pair.target(),
                &i.old.principal,
                pair.permits_subscriptions(),
            )
            .map(Box::new)
    }
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::from_timestamp(self.invocation.read.old().expires_at(), 0)
    }
}
impl Runtime {
    pub(in crate::canonical_replacement) async fn finish_original_public_video_abort(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
    ) -> Result<()> {
        self.discard_original_public_video_candidate(root, EndReason::Normal)
            .await?;
        let closed = {
            let owned = root
                .public_discard
                .try_lock()
                .map_err(|_| Error::new("authority-busy", "Original close owner busy"))?;
            let OwnedDiscard::Closed(closed) = &*owned else {
                return Err(Error::forbidden());
            };
            closed.clone()
        };
        // Capture the exact original physical/adapter outside every Store or
        // Core fence; later callbacks inspect only these original identities.
        let physical = root
            .allocated
            .lock()
            .map_err(|_| Error::storage())?
            .as_ref()
            .and_then(|allocated| allocated.webrtc.clone())
            .ok_or_else(Error::forbidden)?;
        let adapter = self.webrtc.get().ok_or_else(Error::forbidden)?.adapter();
        let observation = Arc::new(ClosedObservation {
            root: root.clone(),
            closed: closed.clone(),
            adapter: adapter.clone(),
            physical: physical.clone(),
            reason: root.public_video.original_discard_reason(root.deadline),
        });
        if !root.public_video.discarded_recorded() {
            let store = self.media.store.clone();
            let original = root.receipt.clone();
            let observed = observation.clone();
            let negative = super::original_not_submitted(root)?;
            // Already joined actual close: this separate fresh interval grants
            // historical bookkeeping only. No Reader was held across native joins.
            store_call(move || {
                let wait = connect_store::AuthorityWait::new(Duration::from_secs(2));
                let read = if let Some(captured) = negative {
                    store.native_replacement_not_submitted_unpublished_read(
                        original, &captured, &wait,
                    )?
                } else {
                    store.native_replacement_known_unpublished_read(original, &wait)?
                };
                store
                    .record_native_replacement_public_video_discarded(&read, observed.as_ref())
                    .map(|_| ())
            })
            .await?;
            root.public_video.record_discarded_observation();
        }
        if root.public_video.old_restored() {
            return Ok(());
        }
        let Some(plan) = root.public_video.original_restore_plan() else {
            return Ok(());
        };
        let restore_deadline = root.public_video.original_restore_deadline();
        if tokio::time::Instant::now() >= restore_deadline {
            return Err(Error::new(
                "cleanup-incomplete",
                "Original OLD restoration admission budget elapsed with its owner still retained",
            ));
        }

        let readiness = match self.core.staged_old_video_restore_readiness(&plan) {
            Ok(readiness) => readiness,
            Err(error) if super::super::projection::is_core_metadata_busy(&error) => {
                return Err(super::super::projection::core_plan_error(error))
            }
            Err(error) => {
                tracing::debug!(target:"connect_service::canonical_replacement",?error,
                    "Original candidate closed; changed OLD resources remain fenced");
                return Ok(()); // NOT a successful restoration or a new grant
            }
        };
        let hints = readiness.sources().to_vec();
        let paused =
            readiness.resource_state() == OldVideoRestoreResourceState::SelectedSourcePaused;
        let entries = self
            .media
            .scene_entries(&root.origin.request().session_id)
            .map_err(|_| Error::forbidden())?;
        let old = entries
            .get(&root.handle.intent().source)
            .cloned()
            .ok_or_else(Error::forbidden)?;
        let mut selected = BTreeMap::new();
        for hint in &hints {
            let entry = entries
                .get(&hint.connection)
                .cloned()
                .ok_or_else(Error::forbidden)?;
            selected.insert(hint.connection.as_str().to_owned(), entry);
        }
        let publishers = selected
            .values()
            .map(|entry| entry.origin.clone())
            .collect::<Vec<_>>();
        let store = self.media.store.clone();
        let origin = old.origin.clone();
        let read = match store_call(move || {
            store.try_native_video_restore_read(&origin, &publishers)
        })
        .await
        {
            Ok(read) => Arc::new(read),
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "forbidden" | "unauthorized" | "expired"
                ) =>
            {
                tracing::debug!(target:"connect_service::canonical_replacement",code=%error.code,
                    "Original candidate closed; current OLD/source permission refuses restoration");
                return Ok(()); // cleanup completes without reviving revoked media
            }
            Err(error) => return Err(error),
        };
        if !read.current() {
            drop(read);
            return Ok(()); // independently expired/revoked OLD/source facts
        }
        if tokio::time::Instant::now() >= restore_deadline {
            drop(read);
            return Err(Error::new(
                "cleanup-incomplete",
                "Original OLD restoration cutoff elapsed during its current permission read",
            ));
        }
        // Even a known paused source must first pass independent current OLD
        // and every selected publisher authorization. No cancelled TARGET is
        // renewed, and no Store Reader is retained while waiting for resume.
        if paused {
            drop(read);
            return Err(Error::new(
                "authority-busy",
                "Original OLD restoration awaits the same selected source resume",
            ));
        }
        let deadline = restore_deadline;
        let invocation = RestoreInvocation {
            policy: self.media.clone(),
            root: root.clone(),
            closed,
            adapter,
            physical,
            read,
            old,
            selected,
            hints: hints.into(),
            deadline,
            cancel: self.stop.child_token(),
        };
        let restored = self
            .core
            .restore_staged_device_replacement_video_with_held(&plan, &invocation);
        let still_authorized = invocation.read.current();
        drop(invocation);
        // Preserve a known successful physical restoration even if this task
        // is descheduled after its guarded effect. An unresolved deadline may
        // never fall through to generic successful cleanup.
        if restored.is_err() && still_authorized && tokio::time::Instant::now() >= restore_deadline
        {
            return Err(Error::new(
                "cleanup-incomplete",
                "Original OLD restoration cutoff elapsed without confirmed restoration",
            ));
        }
        match restored {
            Ok(()) => root.public_video.record_old_restored(),
            // Retain custody on exclusion; the next attempt MUST first obtain
            // the typed SAME Frozen/held snapshot above. A taken/touched owner
            // cannot pass that snapshot, so an uncertain effect is not replayed.
            Err(error) if super::super::projection::is_core_metadata_busy(&error) => {
                return Err(super::super::projection::core_plan_error(error))
            }
            Err(error)
                if matches!(
                    &error,
                    rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                        StreamControlError::NotReady
                    )
                ) =>
            {
                match self.core.staged_old_video_restore_readiness(&plan) {
                    // Generic NotReady alone never justifies retry. This new
                    // exact metadata query proves the original frozen set and
                    // guards are STILL owned; current Store auth is reacquired
                    // independently on the next watcher interval.
                    Ok(_) => return Err(Error::new(
                        "authority-busy",
                        "Original OLD resources remain retained for fresh restoration admission",
                    )),
                    Err(error) if super::super::projection::is_core_metadata_busy(&error) => {
                        return Err(super::super::projection::core_plan_error(error))
                    }
                    Err(error) => {
                        tracing::debug!(target:"connect_service::canonical_replacement",?error,
                        "Original OLD restore owner is no longer recoverable; media remains fenced")
                    }
                }
            }
            Err(error) => tracing::debug!(target:"connect_service::canonical_replacement",?error,
                "Original candidate closed; OLD restoration refused and original guards remain fail-closed"),
        }
        Ok(())
    }
}
