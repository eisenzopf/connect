//! Activate B under the signed pre-cut bridge grant while OLD keeps playing.
use super::super::start::StartedReplacement;
use super::{observation::RuntimeObservation, pipeline::stream_error, *};
use connect_protocol::webrtc_public_video_bridge as wire_bridge;
use connect_store::{
    NativeReplacementPublicVideoBridgeExpected as Expected,
    NativeReplacementPublicVideoBridgeGuard as StoreGuard,
    NativeReplacementPublicVideoBridgeMetadata as Metadata,
    NativeReplacementPublicVideoBridgeObservation as StoreObservation,
    NativeReplacementPublicVideoBridgeStage as Stage,
};
use rvoip_core::{
    peer_admission::PeerAdmission,
    staged_streams::direct_subscriptions::PreCutPublicVideoBridgeScope,
};
use rvoip_webrtc::media::core_video::ReceiverReleaseFrame;
use std::sync::Weak;
use tokio::time::Instant;

struct BridgeObservation {
    root: Weak<PreparedReplacement>,
    started: Weak<StartedReplacement>,
    scope: Arc<PreCutPublicVideoBridgeScope>,
}

struct HeldBridgeObservation {
    runtime: RuntimeObservation,
    scope: Arc<PreCutPublicVideoBridgeScope>,
    decoded: Option<(wire_bridge::ReadyMarker, Arc<ReceiverReleaseFrame>)>,
}

impl HeldBridgeObservation {
    fn require_current(&self) -> Result<()> {
        let root = &self.runtime.root;
        let p = &root.public_video;
        if !self.scope.is_current()
            || !p.bridge_matches(self.scope.as_ref())
            || p.cut.get().is_some()
            || p.warm.get().is_some()
            || p.ready.get().is_some()
            || root.handle.is_committed()
            || root.handle.cancellation().is_cancelled()
            || Instant::now() >= self.scope.deadline()
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}

impl StoreObservation for BridgeObservation {
    fn hold_current(&self, expected: &Expected<'_>) -> Result<Option<Box<dyn StoreGuard + '_>>> {
        let root = self.root.upgrade().ok_or_else(Error::forbidden)?;
        let started = self.started.upgrade().ok_or_else(Error::forbidden)?;
        if !root
            .public_video
            .reservation()?
            .same_original(expected.reservation())
            || root.origin.request().operation_id != expected.operation_id()
        {
            return Err(Error::forbidden());
        }
        let decoded = match expected.stage() {
            Stage::Feeding => None,
            Stage::DecodedReady => {
                let feeding = expected.feeding().ok_or_else(Error::forbidden)?;
                let ready = expected.signed_ready().ok_or_else(Error::forbidden)?;
                ready.validate(feeding)?;
                let output = root.public_video.bridge.output.get().ok_or_else(|| {
                    Error::new("authority-busy", "Original B writer startup is pending")
                })?;
                if output.target() != self.scope.target()
                    || output.primary_ssrc() != ready.bridge_readiness.binding.primary_ssrc
                {
                    return Err(Error::forbidden());
                }
                let original = output
                    .pre_cut_keyframe(ready.bridge_readiness.decoded_rtp_timestamp)
                    .ok_or_else(Error::forbidden)?;
                Some((ready.clone(), original))
            }
        };
        let held = HeldBridgeObservation {
            runtime: RuntimeObservation { root, started },
            scope: self.scope.clone(),
            decoded,
        };
        held.require_current()?;
        Ok(Some(Box::new(held)))
    }
}

impl StoreGuard for HeldBridgeObservation {
    fn metadata(&self) -> Result<Metadata> {
        self.require_current()?;
        let (binding, private_readiness_nonce) = self.runtime.current_negotiated_binding()?;
        match &self.decoded {
            Some((ready, written)) => Ok(Metadata::DecodedReady {
                binding,
                private_readiness_nonce,
                ready: ready.clone(),
                written_rtp_timestamp: written.rtp_timestamp(),
            }),
            None => Ok(Metadata::Feeding {
                binding,
                private_readiness_nonce,
            }),
        }
    }

    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()> {
        let activation = self
            .runtime
            .started
            .policy
            .activation
            .get()
            .ok_or_else(Error::forbidden)?;
        activation
            .try_publish_installed(
                self.runtime.root.handle.prepared_binding().generation(),
                || {
                    self.require_current()?;
                    commit()
                },
            )
            .map_err(stream_error)?
    }
}

impl Runtime {
    pub(in crate::canonical_replacement) async fn activate_pre_cut_webrtc_bridge(
        &self,
        root: &Arc<PreparedReplacement>,
        started: &Arc<StartedReplacement>,
        peer: &Arc<PeerAdmission>,
    ) -> Result<()> {
        if wire_bridge::OptIn::from_request(root.origin.request())?.is_none() {
            return Ok(());
        }
        let p = &root.public_video;
        let scope = self
            .prepare_pre_cut_public_video_bridge(root, started)
            .await?;
        let until = scope.deadline();
        let cancel = scope.cancellation();
        let observation = Arc::new(BridgeObservation {
            root: Arc::downgrade(root),
            started: Arc::downgrade(started),
            scope: scope.clone(),
        });
        p.own_bridge_journal_observation(root, observation)?;
        // Existing metadata retry owns no native grant or coordinator. The
        // same original feeding row must commit before the writer is opened.
        super::super::capture_recording::retry_captured_metadata(&self.stop, until, true, || {
            self.record_pre_cut_public_video_bridge_feeding(root)
        })
        .await?;
        tracing::debug!(target: "connect_service::canonical_replacement",
            boundary = "public-video-bridge", stage = "feeding-recorded",
            operation = %root.handle.intent().operation_id, source = %root.handle.intent().source,
            target = %root.handle.intent().replacement, "Original candidate video bridge boundary");
        let adapter = self.webrtc.get().ok_or_else(Error::forbidden)?.adapter();
        let output = tokio::select! {biased;
            _ = self.stop.cancelled() => return Err(Error::forbidden()),
            _ = cancel.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(until) => return Err(Error::new("expired", "Original bridge startup cutoff elapsed")),
            output = adapter.start_pre_cut_public_video(peer, &scope) => output.map_err(stream_error)?,
        };
        p.bridge
            .output
            .set(output)
            .map_err(|_| Error::forbidden())?;
        tracing::debug!(target: "connect_service::canonical_replacement",
            boundary = "public-video-bridge", stage = "candidate-writer-started",
            operation = %root.handle.intent().operation_id, source = %root.handle.intent().source,
            target = %root.handle.intent().replacement, "Original candidate video bridge boundary");
        p.wait_for_bridge_decoded(&self.stop, &cancel, until)
            .await?;
        tracing::debug!(target: "connect_service::canonical_replacement",
            boundary = "public-video-bridge", stage = "candidate-decoded-before-withdrawal",
            operation = %root.handle.intent().operation_id, source = %root.handle.intent().source,
            target = %root.handle.intent().replacement, "Original candidate video bridge boundary");
        Ok(())
    }
}
