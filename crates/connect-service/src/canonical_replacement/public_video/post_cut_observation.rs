//! Positive original B output after the owned OLD frontier. This only gives
//! the authorized candidate a receiver coordinate to validate while hidden.
use super::*;
use connect_protocol::{
    replacement_receiver_video_release::{
        OriginalFrameFloor, PresentationFloor, ReceiverVideoRelease, POST_CUT_PROFILE,
    },
    transitions::{OperationRecord, OperationState},
};

impl Runtime {
    pub(crate) fn receiver_video_post_cut(
        &self,
        record: &OperationRecord,
    ) -> Option<ReceiverVideoRelease> {
        if record.state != OperationState::Preparing || record.cancel_requested {
            return None;
        }
        // One control observation only. Missing/busy physical metadata is
        // omitted; the original authorized status poll may observe it later.
        let root = self
            .replacements
            .try_lock()
            .ok()?
            .get(&record.record_id)?
            .clone();
        if root.origin.request().operation_id != record.operation_id
            || root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= root.deadline
        {
            return None;
        }
        root.public_video.receiver_video_boundary(&root, false)
    }
}

impl Preparation {
    pub(super) fn receiver_video_boundary(
        &self,
        root: &PreparedReplacement,
        published: bool,
    ) -> Option<ReceiverVideoRelease> {
        if !self.bridge_decoded_recorded() || !self.cut_recorded.load(Ordering::Acquire) {
            return None;
        }
        let scope = self.bridge_scope.get()?;
        let reserved = self.reserved().ok()?;
        let reservation = self.reservation().ok()?;
        let output = self.bridge.output.get()?;
        let negotiated = self.negotiated.get()?;
        let cut = self.cut.get()?;
        let warm = self.warm.get()?;
        let scopes = warm.scopes();
        let [current] = scopes.as_slice() else {
            return None;
        };
        if current.is_published() != published
            || !current.is_current()
            || (!published && !warm.is_current())
            || !warm.matches_original_plan(&cut.plan)
            || !Arc::ptr_eq(current.frontier_owner(), &cut.frontier)
            || current.target() != scope.target()
            || current.target() != reserved.target()
            || output.target() != current.target()
            || output.primary_ssrc() != negotiated.native.primary_ssrc
            || reservation.operation_record_id() != root.origin.record_id()
        {
            return None;
        }
        let frame = output.first_complete()?;
        let original = frame.original_point();
        let intent = root.handle.intent();
        let observation = ReceiverVideoRelease {
            profile: if published {
                connect_protocol::replacement_receiver_video_release::PROFILE
            } else {
                POST_CUT_PROFILE
            }
            .to_owned(),
            operation_record_id: root.origin.record_id().into(),
            operation_id: intent.operation_id.clone(),
            reservation_id: reservation.intent().reservation_id.clone(),
            source_connection_id: root.origin.request().connection_id.clone()?,
            target_connection_id: reservation.candidate_connection_id().into(),
            stream_id: scope.target().logical_id.as_str().into(),
            stream_generation: scope.target().stream_generation,
            connection_generation: scope.target().connection_generation,
            target_generation: reservation.target_generation(),
            source_cut_id: cut.id.clone(),
            original_frame: OriginalFrameFloor {
                realm: format!("{:032x}", original.incarnation()),
                last_old_ordinal: cut.frontier.ordinal().to_string(),
                ordinal: original.ordinal().to_string(),
            },
            floor: PresentationFloor::WebrtcRtp {
                physical_writer_incarnation: format!(
                    "{:032x}",
                    scope.target().physical_writer_incarnation
                ),
                primary_ssrc: output.primary_ssrc(),
                first_public_rtp_timestamp: frame.rtp_timestamp(),
            },
        };
        if original.incarnation() != cut.frontier.realm_incarnation() {
            return None;
        }
        if published {
            observation.validate().ok()?;
        } else {
            observation.validate_post_cut().ok()?;
        }
        Some(observation)
    }
}
