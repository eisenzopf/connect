//! Original physical restart custody. Wire dispatch remains separately gated.
use super::*;
use crate::canonical_media::Policy;
use connect_store::{
    AuthorityWait, NativeIceOrigin, NativeStreamCommitted, NativeStreamLease, PeerAuthorityGuard,
};
use rvoip_core::{peer_admission::PeerAdmission, staged_streams::TransportUpdateSlot};
use rvoip_webrtc::peer::RvoipPeerConnection;

/// One owned local attempt. Only the caller's tracked worker may own this
/// value. Idle custody holds the exclusive Core slot, never a Store reader or
/// originating AuthorizedEnvelope. Retain it through answer delivery and the
/// original-generation observation; uncertain recovery must never call start.
pub(super) struct Restart {
    policy: Arc<Policy>,
    adapter: Arc<CanonicalWebRtcAdapter>,
    peer: Arc<PeerAdmission>,
    origin: Arc<NativeIceOrigin>,
    credential: String,
    id: ConnectionId,
    lease: Option<NativeStreamLease>,
    receipt: Option<NativeStreamCommitted>,
    slot: Option<TransportUpdateSlot>,
    rtc: Option<Arc<RvoipPeerConnection>>,
    submitted: bool,
    deadline: Instant,
    stop: CancellationToken,
    physical: CancellationToken,
}
impl Restart {
    /// Consume only an already-begun original Store lease. Failed construction
    /// returns that same lease so its worker can record a pre-IO refusal.
    pub(super) fn new(
        policy: Arc<Policy>,
        adapter: Arc<CanonicalWebRtcAdapter>,
        peer: Arc<PeerAdmission>,
        credential: String,
        proof: &PeerAuthorityGuard,
        lease: NativeStreamLease,
        deadline: Instant,
        stop: CancellationToken,
        physical: CancellationToken,
    ) -> std::result::Result<Self, (Error, NativeStreamLease)> {
        if lease
            .ice_restart()
            .is_none_or(|ice| ice.previous_local_answer().is_some() || ice.answer().is_some())
        {
            return Err((Error::forbidden(), lease));
        }
        let origin = match policy
            .store
            .native_ice_origin_with_peer(&credential, proof, &lease)
        {
            Ok(origin) => origin,
            Err(e) => return Err((e, lease)),
        };
        let deadline = deadline.min(Instant::now() + Duration::from_secs(30)).min(
            Instant::now()
                + Duration::from_secs(
                    (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64
                ),
        );
        let id = ConnectionId::from_string(lease.runtime_connection_id().to_owned());
        Ok(Self {
            policy,
            adapter,
            peer,
            origin,
            credential,
            id,
            lease: Some(lease),
            receipt: None,
            slot: None,
            rtc: None,
            submitted: false,
            deadline,
            stop,
            physical,
        })
    }
    fn lease(&self) -> Result<&NativeStreamLease> {
        self.lease.as_ref().ok_or_else(Error::forbidden)
    }
    fn invocation(&self) -> Result<crate::canonical_media::ice::Invocation> {
        if self.stop.is_cancelled()
            || self.physical.is_cancelled()
            || Instant::now() >= self.deadline
        {
            return Err(Error::forbidden());
        }
        let proof = self.policy.store.resume_native_ice_authority(
            &self.credential,
            &self.origin,
            self.lease()?,
            &AuthorityWait::new(Duration::from_millis(100)),
        )?;
        self.policy.ice_invocation(
            Arc::new(proof),
            self.lease()?,
            self.peer.clone(),
            self.deadline,
        )
    }
    /// Exactly one physical invocation, never called by status/replay. Every
    /// error after submission remains possible-effect; no regather/retry path.
    /// Returns the persisted original answer, not an ICE-ready receipt.
    pub(super) async fn negotiate(&mut self) -> Result<String> {
        if self.submitted {
            return Err(Error::forbidden());
        }
        let invocation = self.invocation()?;
        let controller = self
            .adapter
            .staged_controller(&self.id)
            .ok_or_else(Error::forbidden)?;
        if self.slot.is_none() {
            let expected = self.lease()?.expected_revision();
            self.slot = Some(
                controller
                    .reserve_transport_update_with_held(expected, self.deadline, &invocation)
                    .map_err(|error| {
                        if matches!(
                            error,
                            rvoip_core::staged_streams::StreamControlError::Model(
                                rvoip_core::stream_lifecycle::MediaTransitionError::Conflict
                            )
                        ) && controller.revision() == expected
                        {
                            Error::new("authority-busy", "Original ICE control slot is busy")
                        } else {
                            media_error(error)
                        }
                    })?,
            );
        }
        let rtc = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            self.adapter
                .original_peer(&self.id, &self.peer, self.deadline),
        )
        .await?
        .map_err(|_| Error::forbidden())?;
        let descriptions = bounded(self.deadline, &self.stop, &self.physical, async {
            (
                rtc.peer_connection().local_description().await,
                rtc.peer_connection().remote_description().await,
            )
        })
        .await?;
        let (Some(local), Some(remote)) = descriptions else {
            return Err(Error::forbidden());
        };
        let proposed = self
            .lease()?
            .ice_restart()
            .ok_or_else(Error::forbidden)?
            .proposed_remote_offer()
            .to_owned();
        let lease = self.lease.as_mut().ok_or_else(Error::forbidden)?;
        self.policy
            .store
            .capture_native_ice_baseline(lease, &local.sdp, &remote.sdp)?;
        let current = self
            .slot
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .current_with_held(&invocation)
            .map_err(media_error)?;
        self.rtc = Some(rtc.clone());
        // Set before the first possible physical effect, including an engine
        // error or cancelled observer. Those outcomes cannot become no-effect.
        self.submitted = true;
        let answer = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            rtc.renegotiate_as_answerer(&proposed),
        )
        .await?
        .map_err(|_| {
            Error::new(
                "reconciliation-required",
                "Original ICE restart outcome is uncertain",
            )
        })?;
        self.slot
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .current_with_held(&invocation)
            .map_err(media_error)?;
        if !self
            .adapter
            .is_peer_webrtc_preparation_current(&self.id, &self.peer)
        {
            return Err(Error::forbidden());
        }
        self.policy.store.capture_native_ice_answer(
            self.lease.as_mut().ok_or_else(Error::forbidden)?,
            &answer,
        )?;
        drop(current);
        drop(invocation);
        Ok(answer)
    }
    /// One short fresh-authority observation, without sleeping or retaining
    /// readers after return. A true result has captured an immutable original
    /// receipt; metadata persistence may retry without another RTC invocation.
    pub(super) async fn observe(&mut self) -> Result<bool> {
        if self.receipt.is_some() {
            return Ok(true);
        }
        let invocation = self.invocation()?;
        let slot = self.slot.as_ref().ok_or_else(Error::forbidden)?;
        let current = slot.current_with_held(&invocation).map_err(media_error)?;
        let rtc = self.rtc.as_ref().ok_or_else(Error::forbidden)?;
        let same = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            self.adapter
                .original_peer(&self.id, &self.peer, self.deadline),
        )
        .await?
        .map_err(|_| Error::forbidden())?;
        if !Arc::ptr_eq(rtc, &same) {
            return Err(Error::forbidden());
        }
        let intent = self.lease()?.ice_restart().ok_or_else(Error::forbidden)?;
        let answer = intent.answer().ok_or_else(Error::forbidden)?;
        let ready = bounded(
            self.deadline,
            &self.stop,
            &self.physical,
            rtc.peer_connection()
                .ice_descriptions_ready(answer, intent.proposed_remote_offer()),
        )
        .await?;
        // Verify the same slot/owner/ports/credential and physical lifetime
        // again after the live core lock observation, before taking the receipt.
        slot.current_with_held(&invocation).map_err(media_error)?;
        if !self
            .adapter
            .is_peer_webrtc_preparation_current(&self.id, &self.peer)
        {
            return Err(Error::forbidden());
        }
        if ready {
            let generation = intent.expected_generation() + 1;
            self.receipt = Some(
                self.lease
                    .take()
                    .ok_or_else(Error::forbidden)?
                    .ice_committed(generation)?,
            );
        }
        drop(current);
        drop(invocation);
        Ok(ready)
    }
    pub(super) fn complete(&self) -> Result<connect_protocol::transitions::OperationRecord> {
        self.policy
            .store
            .complete_native_stream(self.receipt.as_ref().ok_or_else(Error::forbidden)?)
    }
    pub(super) fn uncertain(&self) -> Result<connect_protocol::transitions::OperationRecord> {
        self.policy
            .store
            .native_stream_attempt_uncertain(self.lease()?)
    }
    /// Definitive refusal is possible only before calling the physical engine.
    pub(super) fn refuse(
        self,
        error: Error,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        if self.submitted {
            return Err(Error::forbidden());
        }
        self.policy
            .store
            .fail_native_stream_without_effect(self.lease.ok_or_else(Error::forbidden)?, error)
    }
}
async fn bounded<T>(
    deadline: Instant,
    stop: &CancellationToken,
    physical: &CancellationToken,
    future: impl std::future::Future<Output = T>,
) -> Result<T> {
    tokio::select! { biased;
        _ = stop.cancelled() => Err(Error::forbidden()),
        _ = physical.cancelled() => Err(Error::forbidden()),
        _ = tokio::time::sleep_until(deadline) => Err(Error::new("reconciliation-required", "Original ICE observation expired")),
        result = future => Ok(result),
    }
}

fn media_error(error: rvoip_core::staged_streams::StreamControlError) -> Error {
    use rvoip_core::{
        staged_streams::StreamControlError as E, stream_lifecycle::MediaTransitionError,
    };
    match error {
        E::NotReady | E::Backpressure => {
            Error::new("authority-busy", "ICE current authority is busy")
        }
        E::Model(MediaTransitionError::Conflict) => {
            Error::new("conflict", "Original ICE media binding changed")
        }
        E::Deadline => Error::new("reconciliation-required", "Original ICE control expired"),
        _ => Error::forbidden(),
    }
}
