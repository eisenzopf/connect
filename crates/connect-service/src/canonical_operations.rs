//! Canonical recovery uses the same durable operation decisions as other facades.
mod preparation;
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::{
    transitions::{OperationAction, OperationRecord, ReconcileOutcome},
    Error, Result,
};
pub(crate) use preparation::project as project_preparation;
use rvoip_uctp::v2::{payloads::OperationReconcileOutcome, Envelope, Payload, Scope};

/// The top-level ID identifies a cancel/reconcile decision; the payload ID
/// always identifies its target. Parsing once gives policy and handler one intent.
pub(crate) fn action(request: &Envelope) -> Result<Option<OperationAction>> {
    if !matches!(
        request.payload(),
        Payload::OperationGet(_) | Payload::OperationCancel(_) | Payload::OperationReconcile(_)
    ) {
        return Ok(None);
    }
    if request.expected_revision().is_some() {
        return Err(Error::new(
            "unsupported-command",
            "Operation revision assertions are not installed",
        ));
    }
    let own = || {
        request
            .scope(Scope::Operation)
            .map(str::to_owned)
            .ok_or_else(|| Error::new("invalid-request", "Missing decision operation ID"))
    };
    Ok(Some(match request.payload() {
        Payload::OperationGet(input) => {
            if request
                .scope(Scope::Operation)
                .is_some_and(|id| id != input.operation_id.as_str())
            {
                return Err(Error::forbidden());
            }
            OperationAction::Get {
                operation_id: input.operation_id.as_str().into(),
            }
        }
        Payload::OperationCancel(input) => OperationAction::Cancel {
            operation_id: own()?,
            target_operation_id: input.operation_id.as_str().into(),
            reason: input.reason.clone(),
        },
        Payload::OperationReconcile(input) => OperationAction::Reconcile {
            operation_id: own()?,
            target_operation_id: input.operation_id.as_str().into(),
            outcome: match input.outcome {
                OperationReconcileOutcome::Completed => ReconcileOutcome::Completed,
                OperationReconcileOutcome::Cancelled => ReconcileOutcome::Cancelled,
                OperationReconcileOutcome::Failed => ReconcileOutcome::Failed,
            },
            evidence_id: input.evidence_id.as_str().into(),
        },
        _ => return Ok(None),
    }))
}
/// Current original operation plus its specialized metadata projection. Neither
/// field grants media permission or proves a physical phase on its own.
pub(crate) struct OperationObservation {
    pub record: OperationRecord,
    intent: Option<connect_protocol::webrtc_video_receive_reservation::Intent>,
    phase: Option<connect_protocol::webrtc_public_video_phase::Phase>,
    bridge: Option<connect_protocol::webrtc_public_video_bridge::Snapshot>,
}
impl OperationObservation {
    pub(crate) fn project_public_video(&self, payload: &mut serde_json::Value) {
        if let Some(intent) = &self.intent {
            payload["webrtc_video_receive_reservation"] = serde_json::json!(intent);
        }
        if let Some(phase) = &self.phase {
            let mut value = serde_json::json!(phase);
            if let Some(release) = value
                .as_object_mut()
                .and_then(|object| object.remove("receiver_video_release"))
            {
                payload["receiver_video_release"] = release;
            }
            payload["webrtc_public_video_phase"] = value;
        }
        if let Some(bridge) = &self.bridge {
            payload["webrtc_public_video_bridge"] = serde_json::json!(bridge);
        }
    }
}
impl Service {
    pub(crate) async fn canonical_operation_held(
        &self,
        credential: String,
        claim: connect_store::PeerOperationClaim,
        authority: Option<StoreInvocationAuthority>,
    ) -> Result<OperationObservation> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || {
            let peer = authority
                .as_ref()
                .map(|authority| authority.peer_guard().ok_or_else(Error::forbidden))
                .transpose()?;
            if matches!(&claim.action, OperationAction::Get { .. }) {
                let snapshot = store.observe_operation_get(&credential, &claim, peer)?;
                return Ok(OperationObservation {
                    record: snapshot.operation,
                    intent: snapshot.intent,
                    phase: snapshot.phase,
                    bridge: snapshot.bridge,
                });
            }
            let record = match peer {
                Some(peer) => {
                    store.execute_operation_action_with_peer(&credential, claim.action, peer)
                }
                None => store.execute_operation_action_claim(&credential, claim),
            }?;
            Ok(OperationObservation {
                record,
                intent: None,
                phase: None,
                bridge: None,
            })
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
