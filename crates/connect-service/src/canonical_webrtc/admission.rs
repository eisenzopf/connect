//! Actual original RTC resource admission after the signed SDP handler exits.
use super::*;
use crate::canonical_media::webrtc::RtcInvocation;
use connect_store::NativeReadyObservation;
use rvoip_core::{
    ids::ParticipantId,
    participant_transitions::{ParticipantChange, ParticipantMutation},
    staged_streams::{MediaActivation, MediaOwner, StreamController},
    stream_lifecycle::ReadyMediaBinding,
};
use std::collections::BTreeSet;

pub(super) struct Ready {
    activation: Arc<dyn MediaActivation>,
    controller: Arc<StreamController>,
    owner: MediaOwner,
    runtime: Arc<Runtime>,
    physical: Arc<Physical>,
    peer: Arc<rvoip_core::peer_admission::PeerAdmission>,
    expected: NativeReadyObservation,
    bindings: Vec<ReadyMediaBinding>,
}
impl Ready {
    // Resource observation only. These saved relationship values never grant
    // packet IO; the installed policy still derives fresh current authority.
    pub(super) fn current(&self) -> Result<()> {
        self.observe_resources(false).map(|_| ())
    }
}
impl NativeOfferRuntimeObservation for Ready {
    fn bindings(&self) -> Result<Vec<connect_protocol::native_streams::CodecBinding>> {
        self.observe()?;
        crate::canonical_native_streams::codec_bindings(&self.bindings)
    }
    fn observe(&self) -> Result<NativeOfferObservation> {
        self.observe_resources(true)
    }
}
impl Ready {
    fn observe_resources(&self, original_revision: bool) -> Result<NativeOfferObservation> {
        if self.physical.lifetime.is_cancelled()
            || self.runtime.stop.is_cancelled()
            || self.peer.is_closed()
            || (if original_revision {
                self.controller.revision() != self.expected.stream_revision
            } else {
                self.controller.revision() < self.expected.stream_revision
            })
            || !same_bindings(&self.controller.active_bindings(), &self.bindings)
            || self.activation.confirmed_generation() != Some(self.expected.generation)
        {
            return Err(Error::forbidden());
        }
        let owner = self
            .activation
            .current(self.owner.clone())
            .map_err(denied)?;
        if owner.connection.connection().as_str() != self.expected.runtime_connection_id
            || owner.connection.generation() != self.expected.generation
            || owner.member.participant().as_str() != self.expected.participant_id
            || owner.member.epoch() != self.expected.membership_epoch
            || owner.endpoint.endpoint().as_str() != self.expected.endpoint_id
            || owner.endpoint.device().as_str() != self.expected.device_id
        {
            return Err(Error::forbidden());
        }
        let session = self
            .runtime
            .core
            .session(&SessionId::from_string(self.expected.session_id.clone()))
            .ok_or_else(Error::forbidden)?;
        let session = session
            .try_read()
            .map_err(|_| Error::new("authority-busy", "RTC managed Session observation is busy"))?;
        if session.conversation_id.as_str() != self.expected.conversation_id
            || !session
                .connections
                .get(&ConnectionId::from_string(
                    self.expected.runtime_connection_id.clone(),
                ))
                .is_some_and(|c| c.participant_id.as_str() == self.expected.participant_id)
            || !session.participants.contains(&ParticipantId::from_string(
                self.expected.participant_id.clone(),
            ))
        {
            return Err(Error::forbidden());
        }
        Ok(NativeOfferObservation::Ready(self.expected.clone()))
    }
}
fn same_bindings(current: &[ReadyMediaBinding], expected: &[ReadyMediaBinding]) -> bool {
    current.len() == expected.len()
        && current.iter().zip(expected).all(|(a, b)| {
            a.negotiated == b.negotiated
                && a.transport_id == b.transport_id
                && a.stream_local_id == b.stream_local_id
                && a.generation == b.generation
                && a.connection_generation == b.connection_generation
                && a.source_ready == b.source_ready
                && a.sink_ready == b.sink_ready
        })
}
fn denied<T>(_: T) -> Error {
    Error::forbidden()
}
impl Runtime {
    pub(super) async fn webrtc_admit(
        self: &Arc<Self>,
        owned: &Owned,
        physical: Arc<Physical>,
        credential: &str,
    ) -> Result<Ready> {
        let id = owned.id.as_ref().ok_or_else(Error::forbidden)?;
        let deadline = owned.deadline.ok_or_else(Error::forbidden)?;
        let host = self.webrtc.get().ok_or_else(Error::forbidden)?;
        let adapter = host.adapter();
        let entry = owned.entry.as_ref().ok_or_else(Error::forbidden)?.clone();
        let controller = owned
            .controller
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .clone();
        let attempt = owned.attempt.as_ref().ok_or_else(Error::forbidden)?.clone();
        let peer = owned.peer.as_ref().ok_or_else(Error::forbidden)?.clone();
        // There is no original AuthorizedEnvelope, Store read or Core proposal
        // fence across the actual ICE/DTLS/primary-RTP wait. Dropping this wait
        // cannot discard the Backend's original SourceJob or its cleanup join.
        tokio::select! { biased;
            _=self.stop.cancelled()=>return Err(Error::forbidden()),
            _=physical.lifetime.cancelled()=>return Err(Error::forbidden()),
            _=attempt.closed()=>return Err(Error::new("closed","Original RTC owner retired during resource preparation")),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","RTC resource deadline")),
            result=adapter.wait_initial_media(id,deadline,physical.lifetime.child_token())=>result.map_err(denied)?,
        }
        let store = self.media.store.clone();
        let origin = entry.origin.clone();
        let owner = attempt.clone();
        let credential_for_read = credential.to_owned();
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(2));
        // Await this original bounded blocking acquisition; cancellation must
        // not detach a task that could still return a live authority interval.
        let (proof, read) = tokio::task::spawn_blocking(move || {
            let proof = store.resume_webrtc_offer_authority(
                &credential_for_read,
                &origin,
                owner.original_attempt(),
                &connect_store::AuthorityWait::new(remaining),
            )?;
            let read = store.native_media_read_with_peer(&origin, None, &proof)?;
            Ok::<_, Error>((Arc::new(proof), Arc::new(read)))
        })
        .await
        .map_err(denied)??;
        if self.stop.is_cancelled() || physical.lifetime.is_cancelled() {
            return Err(Error::forbidden());
        }
        let current_plan = crate::canonical_media::offer_plan(
            read.target(),
            &crate::canonical_native::base_profile()?,
            &entry.principal,
        )
        .map_err(denied)?
        .1;
        if current_plan.selected() != entry.plan.selected()
            || current_plan.rejected() != entry.plan.rejected()
        {
            return Err(Error::new(
                "conflict",
                "RTC original negotiated plan is no longer current",
            ));
        }
        let source_remaining = Duration::from_millis(
            proof
                .expires_at()
                .saturating_mul(1000)
                .saturating_sub(chrono::Utc::now().timestamp_millis())
                .max(0) as u64,
        );
        let deadline = deadline
            .min(Instant::now() + source_remaining)
            .min(Instant::now() + Duration::from_secs(5));
        let sid = SessionId::from_string(entry.origin.session_id().to_owned());
        let mut invocation = RtcInvocation {
            policy: self.media.clone(),
            connection: entry.clone(),
            proof,
            read,
            peer: peer.clone(),
            operation: attempt.original_attempt().operation_id().into(),
            correlation: owned.correlation.clone().ok_or_else(Error::forbidden)?,
            participant_revision: self.core.participant_revision(&sid).map_err(denied)?,
            cancel: physical.lifetime.child_token(),
            deadline,
        };
        invocation.check(&self.media).map_err(denied)?;
        controller
            .change_selected_with_held(
                0,
                entry.plan.selected(),
                &BTreeSet::new(),
                deadline,
                invocation.cancel.child_token(),
                Arc::new(invocation.clone()),
            )
            .await
            .map_err(denied)?;
        let mut admitted = None;
        for _ in 0..8 {
            invocation.participant_revision =
                self.core.participant_revision(&sid).map_err(denied)?;
            let mutation = ParticipantMutation {
                operation_id: invocation.operation.clone(),
                correlation_id: invocation.correlation.clone(),
                expected_revision: invocation.participant_revision,
                participant: ParticipantId::from_string(entry.origin.participant_id().to_owned()),
                change: ParticipantChange::Admit {
                    connection: id.clone(),
                    role_override: Some(crate::canonical_runtime::participant_role(
                        &invocation.read.target().presence_role,
                    )),
                },
            };
            match self.core.admit_peer_session_connection_with_held(
                peer.clone(), &sid, entry.principal.clone(), mutation, deadline,
                invocation.cancel.child_token(), &invocation,
            ).await {
                Ok(receipt)=>{admitted=Some(receipt);break;},
                Err(rvoip_core::orchestrator::participant_control::ParticipantControlError::Transition(
                    rvoip_core::participant_transitions::ParticipantTransitionError::Conflict))=>continue,
                Err(error)=>return Err(denied(error)),
            }
        }
        let admitted = admitted
            .ok_or_else(|| Error::new("conflict", "Concurrent RTC admission did not settle"))?;
        if !admitted.admitted.contains(id)
            || admitted.operation_id != invocation.operation
            || admitted.membership_epoch != entry.origin.membership_epoch()
        {
            return Err(Error::forbidden());
        }
        let activation = entry
            .activation()
            .map_err(denied)?
            .ok_or_else(Error::forbidden)?;
        let owner = entry
            .owner(invocation.read.target(), &entry.principal, false)
            .map_err(denied)?;
        let owner = activation.current(owner).map_err(denied)?;
        let bindings = controller.active_bindings();
        let selected = entry.plan.selected();
        if bindings.len() != selected.len()
            || bindings.iter().any(|b| !selected.contains(&b.negotiated))
        {
            return Err(Error::forbidden());
        }
        let ready = Ready {
            activation,
            owner,
            runtime: self.clone(),
            physical,
            peer,
            controller: controller.clone(),
            expected: NativeReadyObservation {
                private_consultation: None,
                handoff_preparation: false,
                conversation_id: entry.origin.conversation_id().into(),
                session_id: entry.origin.session_id().into(),
                runtime_connection_id: id.as_str().into(),
                participant_id: admitted.participant.as_str().into(),
                membership_epoch: admitted.membership_epoch,
                endpoint_id: invocation.read.target().connection.endpoint_id.clone(),
                device_id: invocation.read.target().device_id.clone(),
                generation: entry.origin.generation(),
                stream_revision: controller.revision(),
                participant_revision: admitted.revision,
                streams: selected
                    .iter()
                    .map(|s| s.requested.logical_id.as_str().into())
                    .collect(),
            },
            bindings,
        };
        loop {
            invocation.check(&self.media).map_err(denied)?;
            // Retry only the same sealed observation and original effect; no
            // media preparation, participant mutation or SDP is repeated here.
            match self.media.store.complete_native_offer_with_peer(
                credential,
                attempt.original_attempt(),
                &invocation.proof,
                &ready,
            ) {
                Ok(_) => break,
                Err(e)
                    if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                        && Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(5)).await
                }
                Err(e) => return Err(e),
            }
        }
        // All newly acquired readers disappear here, before idle monitoring.
        drop(invocation);
        Ok(ready)
    }
}
