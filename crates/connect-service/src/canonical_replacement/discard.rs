//! Original cleanup custody only. No target renewal, publication or OLD restore.
use super::*;
use rvoip_core::{
    adapter::EndReason,
    orchestrator::participant_control::{
        ControlResult, ParticipantControlError, ParticipantReplacementAuthority,
        ParticipantReplacementControl, ParticipantReplacementLease, ParticipantReplacementScope,
        ReplacementPhase, UnpublishedCandidateDiscardContext,
    },
    relationships::{EndpointBinding, Membership},
};
use rvoip_webrtc::canonical::{
    ClosedOriginalCandidate, OriginalCandidateDiscard, WebRtcPhysicalBinding,
};
use tokio_util::sync::CancellationToken;

/// These exact original facts are historical labels only. Only the distinct
/// cleanup phase accepts them, under a fresh known-unpublished Store Reader.
#[derive(Clone)]
pub(super) struct OriginalPreparationFacts {
    actor: Membership,
    target: Membership,
    endpoint: EndpointBinding,
}
impl OriginalPreparationFacts {
    pub(super) fn capture(i: &ReplacementPreparationInvocation) -> ControlResult<Self> {
        let context = i.read.context();
        let conversation =
            rvoip_core::ids::ConversationId::from_string(context.conversation_id.clone());
        let session = SessionId::from_string(context.session_id.clone());
        let scope = ParticipantReplacementScope {
            conversation: &conversation,
            session: &session,
            principal: &i.principal,
            intent: &i.intent,
            phase: ReplacementPhase::Prepare,
            plan: None,
        };
        let original = i.current(&scope)?;
        Ok(Self {
            actor: original.actor,
            target: original.target,
            endpoint: original.endpoint,
        })
    }
}

/// Serialized only on this original Root's cold cleanup path. No media path
/// consults it. Retain the sole context/physical/reason across every await.
pub(super) enum OwnedDiscard {
    Empty,
    Context {
        context: UnpublishedCandidateDiscardContext,
        physical: Arc<WebRtcPhysicalBinding>,
        reason: EndReason,
    },
    Joining(OriginalCandidateDiscard),
    Closed(ClosedOriginalCandidate),
}

#[derive(Clone)]
pub(crate) struct Invocation {
    pub(crate) policy: Arc<crate::canonical_media::Policy>,
    root: Arc<PreparedReplacement>,
    read: Arc<connect_store::NativeReplacementUnpublishedRead>,
    deadline: tokio::time::Instant,
    cancellation: CancellationToken,
}
impl Invocation {
    pub(crate) fn control(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> ControlResult<ParticipantReplacementControl> {
        self.current(scope)?;
        // Fresh cleanup wait only. Original target cancellation/expiry is not
        // renewed and can never be used by any other participant/media phase.
        ParticipantReplacementControl::new(
            Box::new(self.clone()),
            self.cancellation.clone(),
            self.deadline,
        )
    }
}
impl ParticipantReplacementLease for Invocation {
    fn current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> ControlResult<ParticipantReplacementAuthority> {
        let facts = &self.root.original_cleanup_facts;
        let read = &self.read;
        if scope.phase != ReplacementPhase::Discard
            || scope.plan.is_some()
            || !read.current()
            || self.cancellation.is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
            || self.root.handle.is_committed()
            || scope.intent != self.root.handle.intent()
            || scope.intent
                != &intent(&self.root.plan).map_err(|_| ParticipantControlError::Authority)?
            || read.operation_record_id() != scope.intent.operation_id
            || read.candidate_runtime_connection_id() != scope.intent.replacement.as_str()
            || read.candidate_connection_id()
                != self.root.plan["candidate_connection_id"]
                    .as_str()
                    .unwrap_or("")
            || read.conversation_id() != scope.conversation.as_str()
            || read.session_id() != scope.session.as_str()
            || read.participant_id() != scope.intent.participant.as_str()
            || read.target_endpoint_id() != scope.intent.endpoint.as_str()
            || read.target_device_id() != scope.intent.device.as_str()
            || read.actor_identity_id() != facts.actor.identity().as_str()
            || read.actor_identity_id() != self.root.operator.subject
            || facts.actor.conversation() != scope.conversation
            || facts.target.conversation() != scope.conversation
            || facts.target.participant() != &scope.intent.participant
            || facts.endpoint.endpoint() != &scope.intent.endpoint
            || facts.endpoint.device() != &scope.intent.device
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.root.operator,
            )
        {
            return Err(ParticipantControlError::Authority);
        }
        Ok(ParticipantReplacementAuthority {
            actor: facts.actor.clone(),
            target: facts.target.clone(),
            endpoint: facts.endpoint.clone(),
            may_send_audio: false,
            may_receive_audio: false,
        })
    }
}

impl Runtime {
    /// A successful result means this SAME Root retains its typed real close
    /// token. It is neither a published operation nor permission to thaw OLD.
    pub(crate) async fn discard_original_public_video_candidate(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        reason: EndReason,
    ) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut owned = tokio::select! { biased;
            _ = self.stop.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(deadline) => return Err(Error::new("authority-busy", "Original cleanup owner is busy")),
            owned = root.public_discard.lock() => owned,
        };
        if let OwnedDiscard::Closed(closed) = &*owned {
            return if closed.matches_preparation(&root.handle) {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        let adapter = self.webrtc.get().ok_or_else(Error::forbidden)?.adapter();
        if matches!(&*owned, OwnedDiscard::Empty) {
            // Capture the SAME allocated physical BEFORE issuing one-shot Core
            // custody. Absence of a route is never a substitute for close proof.
            let physical = root
                .allocated
                .lock()
                .map_err(|_| Error::storage())?
                .as_ref()
                .and_then(|a| a.webrtc.clone())
                .ok_or_else(Error::forbidden)?;
            let wait = connect_store::AuthorityWait::new(
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            );
            let _cancel_wait = wait.cancel_on_drop();
            let store = self.media.store.clone();
            let original = root.receipt.clone();
            let owned_wait = wait.clone();
            let resolution = self.owned_store_call(move || {
                store.native_replacement_known_unpublished_read(original, &owned_wait)
            });
            tokio::pin!(resolution);
            let resolved = tokio::select! { biased;
                _ = self.stop.cancelled() => None,
                _ = tokio::time::sleep_until(deadline) => None,
                read = &mut resolution => Some(read),
            };
            let read = match resolved {
                Some(result) => Arc::new(result?),
                None => {
                    wait.cancel();
                    // Join original blocking custody; discard any late Reader.
                    drop(resolution.await);
                    return Err(Error::new(
                        "authority-busy",
                        "Original cleanup admission elapsed",
                    ));
                }
            };
            let invocation = Invocation {
                policy: self.media.clone(),
                root: root.clone(),
                read: read.clone(),
                deadline,
                cancellation: self.stop.child_token(),
            };
            let context = self.core.discard_staged_device_replacement_with_held(&root.handle, &invocation)
                .map_err(|error| {
                    tracing::debug!(target:"connect_service::canonical_replacement",?error,"Original public candidate cancellation refused");
                    Error::forbidden()
                })?;
            *owned = OwnedDiscard::Context {
                context,
                physical,
                reason,
            };
            // Both aliases and their Store guards drop BEFORE physical/Core
            // terminal joins, driver waits or any later durable observation.
            drop(invocation);
            drop(read);
        }
        if matches!(&*owned, OwnedDiscard::Context { .. }) {
            let OwnedDiscard::Context {
                context,
                physical,
                reason,
            } = std::mem::replace(&mut *owned, OwnedDiscard::Empty)
            else {
                unreachable!()
            };
            match adapter.prepare_original_candidate_discard(context, &physical, reason.clone()) {
                Ok(join) => *owned = OwnedDiscard::Joining(join),
                Err(refusal) => {
                    let (context, error) = refusal.into_parts();
                    *owned = OwnedDiscard::Context {
                        context,
                        physical,
                        reason,
                    };
                    tracing::debug!(target:"connect_service::canonical_replacement",?error,"Original public candidate close preparation refused");
                    return Err(Error::new(
                        "cleanup-unconfirmed",
                        "Original candidate close custody retained",
                    ));
                }
            }
        }
        let OwnedDiscard::Joining(join) = &mut *owned else {
            return Err(Error::forbidden());
        };
        let result = tokio::select! { biased;
            _ = self.stop.cancelled() => return Err(Error::forbidden()),
            _ = tokio::time::sleep_until(deadline) => return Err(Error::new("cleanup-unconfirmed", "Original candidate close remains owned")),
            result = join.join(&adapter) => result,
        };
        match result {
            Ok(closed) => {
                *owned = OwnedDiscard::Closed(closed);
                Ok(())
            }
            Err(error) => {
                tracing::debug!(target:"connect_service::canonical_replacement",?error,"Original public candidate join remains owned");
                Err(Error::new(
                    "cleanup-unconfirmed",
                    "Original candidate close remains owned",
                ))
            }
        }
    }
}
