//! Canonical recovery uses the same durable operation decisions as other facades.
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::{
    transitions::{OperationAction, OperationRecord, ReconcileOutcome},
    Error, Result,
};
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
impl Service {
    pub(crate) async fn canonical_operation_held(
        &self,
        credential: String,
        claim: connect_store::PeerOperationClaim,
        authority: Option<StoreInvocationAuthority>,
    ) -> Result<OperationRecord> {
        let store = self.store.clone();
        let result = tokio::task::spawn_blocking(move || match authority {
            Some(authority) => store.execute_operation_action_with_peer(
                &credential,
                claim.action,
                authority.peer_guard().ok_or_else(Error::forbidden)?,
            ),
            None => store.execute_operation_action_claim(&credential, claim),
        })
        .await
        .map_err(|_| Error::storage())?;
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
