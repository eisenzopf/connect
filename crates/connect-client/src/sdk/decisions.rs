//! Durable decisions use their own logical IDs and retain only observed target data.
use super::*;
use crate::canonical::Payload;
pub use connect_protocol::transitions::ReconcileOutcome;
use serde::Deserialize;
#[derive(Clone, Debug)]
pub struct DecisionReceipt {
    pub decision: OperationKey,
    pub target: OperationId,
    pub observed_state: OperationState,
    pub disposition: String,
    pub evidence_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireReceipt {
    target_operation_id: OperationId,
    state_at_decision: OperationState,
    disposition: String,
    evidence_id: Option<String>,
}
pub(super) fn decode(
    value: Value,
    _client: &Client,
    key: &OperationKey,
) -> Result<DecisionReceipt> {
    let fail = || Error::request(ErrorKind::Protocol, Some(key), true);
    let value: WireReceipt = serde_json::from_value(value).map_err(|_| fail())?;
    if value.target_operation_id == key.operation_id
        || value.disposition.is_empty()
        || value.disposition.len() > 256
        || value.disposition.chars().any(char::is_control)
    {
        return Err(fail());
    }
    match (key.command.as_str(), &value.evidence_id) {
        ("operation.cancel", None) => {}
        ("operation.reconcile", Some(evidence)) => {
            crate::canonical::Identifier::try_from(evidence.clone()).map_err(|_| fail())?;
        }
        _ => return Err(fail()),
    }
    Ok(DecisionReceipt {
        decision: key.clone(),
        target: value.target_operation_id,
        observed_state: value.state_at_decision,
        disposition: value.disposition,
        evidence_id: value.evidence_id,
    })
}
pub(super) fn valid_scope(scope: &OperationScope) -> bool {
    scope.connection_id.is_none() && (scope.session_id.is_none() || scope.conversation_id.is_some())
}
impl<T> Operation<T> {
    /// Request a durable decision. Stopping a local wait is a separate action.
    pub async fn cancel(&self, options: MutationOptions) -> Result<Operation<DecisionReceipt>> {
        self.decide(
            self.client().operation_kinds().operation_cancel,
            None,
            options,
        )
        .await
    }
    /// Submit an attributed evidence reference; the host independently verifies it.
    pub async fn reconcile(
        &self,
        outcome: ReconcileOutcome,
        evidence_id: String,
        options: MutationOptions,
    ) -> Result<Operation<DecisionReceipt>> {
        crate::canonical::Identifier::try_from(evidence_id.clone())
            .map_err(|_| Error::local(ErrorKind::Validation))?;
        self.decide(
            self.client().operation_kinds().operation_reconcile,
            Some((outcome, evidence_id)),
            options,
        )
        .await
    }
    async fn decide(
        &self,
        kind: OperationKind<DecisionReceipt>,
        reconciliation: Option<(ReconcileOutcome, String)>,
        options: MutationOptions,
    ) -> Result<Operation<DecisionReceipt>> {
        let target = self.reference();
        let mut own_scope = target.key().scope.clone();
        own_scope.connection_id = None;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: own_scope,
        };
        if key.operation_id == target.key().operation_id {
            return Err(Error::request(ErrorKind::Validation, Some(&key), false));
        }
        // The installed durable decision model has no extension field in its
        // digest. Refuse rather than silently discard caller intent.
        if options
            .extensions
            .as_ref()
            .is_some_and(|values| !values.is_empty())
        {
            return Err(Error::request(ErrorKind::Unsupported, Some(&key), false));
        }
        let mut body = serde_json::json!({"operation_id":target.key().operation_id});
        if let Some((outcome, evidence)) = &reconciliation {
            body["outcome"] = serde_json::to_value(outcome)
                .map_err(|_| Error::request(ErrorKind::Validation, Some(&key), false))?;
            body["evidence_id"] = serde_json::json!(evidence);
        }
        // Submission asserts the target scope; status/recovery uses the separate
        // decision scope. The current decision ledger intentionally omits connid.
        let batch = self
            .client()
            .request(
                kind.command,
                body,
                target.key().scope.command(Some(&key.operation_id)),
                Some(&key),
                options.request,
            )
            .await?;
        let fail = || Error::request(ErrorKind::Protocol, Some(&key), true);
        if batch.frames.len() != 2 {
            return Err(fail());
        }
        let Some(Payload::OperationUpdated(update)) = batch.frames.first().map(|f| f.payload())
        else {
            return Err(fail());
        };
        if update.operation_id.as_str() != key.operation_id.as_str()
            || update.problem.is_some()
            || update.revision > 9_007_199_254_740_991
            || serde_json::to_value(update.state).map_err(|_| fail())?
                != serde_json::json!("completed")
        {
            return Err(fail());
        }
        let result = decode(update.result.clone().ok_or_else(fail)?, self.client(), &key)?;
        if result.target != target.key().operation_id
            || result.evidence_id.as_ref() != reconciliation.as_ref().map(|(_, e)| e)
        {
            return Err(fail());
        }
        Ok(Operation::new(
            self.client().clone(),
            OperationReference {
                key,
                kind,
                continuation: None,
            },
        ))
    }
}
