//! Current fences for ordinary operations. The same derivation is used before
//! ordinary and nested task transactions, never by the engine inside SQLite.
use super::conversation_scopes;
use crate::{Actor, AuthorityMode, AuthorityScope, Store};
use connect_protocol::{Error, Operation, Result};
use rusqlite::Connection;

fn conversation(operation: &Operation) -> Option<&str> {
    match operation {
        Operation::ConnectionPrepare { request } => Some(&request.intent.conversation_id),
        Operation::SessionControl { request } => Some(&request.conversation_id),
        Operation::SessionDecision { request } => Some(&request.conversation_id),
        Operation::Close {
            conversation_id, ..
        }
        | Operation::SessionAccept {
            conversation_id, ..
        }
        | Operation::SessionReject {
            conversation_id, ..
        }
        | Operation::SessionCancel {
            conversation_id, ..
        }
        | Operation::SessionEnd {
            conversation_id, ..
        }
        | Operation::CallEnd {
            conversation_id, ..
        }
        | Operation::SipEnd {
            conversation_id, ..
        }
        | Operation::ArtifactDelete {
            conversation_id, ..
        }
        | Operation::ReceiptPreferencesSet {
            conversation_id, ..
        } => Some(conversation_id),
        _ => None,
    }
}
pub(crate) fn operation_writes_authority(operation: &Operation) -> bool {
    // Inserting an unready own slot cannot grant media or change its accepted
    // parent authorization. Keep parent readers through this dormant phase.
    if matches!(operation, Operation::ConnectionPrepare { .. }) {
        return false;
    }
    matches!(operation, Operation::Membership { .. })
        || conversation(operation).is_some()
        || matches!(
            operation,
            Operation::ContactPairingStart { .. }
                | Operation::ContactPairingComplete { .. }
                | Operation::ContactPairingRevoke { .. }
        )
}
pub(crate) fn operation_scopes(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    store: &Store,
) -> Result<Vec<AuthorityScope>> {
    if let Operation::Membership { request } = operation {
        return crate::membership::control::scopes(c, actor, request);
    }
    let preparation = matches!(operation, Operation::ConnectionPrepare { .. });
    if !operation_writes_authority(operation) && !preparation {
        return Ok(vec![]);
    }
    let mode = if preparation {
        AuthorityMode::Read
    } else {
        AuthorityMode::Write
    };
    actor.require(operation.required_scope())?;
    let targets = crate::operations::resources::authorize(c, actor, operation, store)?;
    if targets.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    let mut scopes = if let Some(cid) = conversation(operation) {
        conversation_scopes(c, cid, mode)?
    } else {
        vec![]
    };
    for target in targets {
        let scope = AuthorityScope::identity(&target, mode)?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    Ok(scopes)
}
