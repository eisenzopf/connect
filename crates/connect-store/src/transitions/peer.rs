//! Canonical operation recovery retains exact metadata writers and projection readers.
use super::*;
use crate::{AuthorityMode, AuthorityScope, PeerOperationClaim};
use connect_protocol::provisioning::ProvisioningOutcome;

pub(crate) fn action_scope(action: &OperationAction) -> (&str, &'static str) {
    match action {
        OperationAction::Get { operation_id } => (operation_id, "operation.get"),
        OperationAction::Cancel {
            target_operation_id,
            ..
        } => (target_operation_id, "operation.cancel"),
        OperationAction::Reconcile {
            target_operation_id,
            ..
        } => (target_operation_id, "operation.reconcile"),
    }
}
fn add_identity(result: &mut Vec<AuthorityScope>, identity: &str) -> Result<()> {
    result.push(AuthorityScope::identity(identity, AuthorityMode::Read)?);
    Ok(())
}
pub(super) fn projection_scopes(
    c: &Connection,
    row: &rows::Saved,
    result: &mut Vec<AuthorityScope>,
) -> Result<()> {
    if row.session_codecs {
        result.extend(super::session_codecs::projection_scopes(c, row)?);
        return Ok(());
    }
    if row.received_dtmf {
        result.extend(super::received_dtmf::projection_scopes(c, row)?);
        return Ok(());
    }
    if row.native || row.native_stream {
        return Ok(());
    }
    if row.control {
        result.extend(crate::operations::receipts::projection_scopes(
            c,
            &row.record.record_id,
        )?);
        return Ok(());
    }
    if row.tombstoned {
        return Ok(());
    }
    let Some(value) = row.record.result.clone() else {
        return Ok(());
    };
    let outcome: ProvisioningOutcome =
        serde_json::from_value(value).map_err(|_| Error::storage())?;
    match outcome {
        ProvisioningOutcome::Identity { identity_id, .. }
        | ProvisioningOutcome::RecordReadRole { identity_id, .. }
        | ProvisioningOutcome::Credential { identity_id, .. }
        | ProvisioningOutcome::Device { identity_id, .. }
        | ProvisioningOutcome::SigningKey { identity_id, .. }
        | ProvisioningOutcome::Endpoint { identity_id, .. } => add_identity(result, &identity_id)?,
        ProvisioningOutcome::Participant {
            identity_id,
            conversation_id,
            ..
        } => {
            add_identity(result, &identity_id)?;
            result.push(AuthorityScope::conversation(
                &conversation_id,
                AuthorityMode::Read,
            )?);
        }
    }
    Ok(())
}
/// Walk stored decision provenance defensively, including malformed nested
/// links that the public API refuses. Never admit unbounded recursion or an
/// unbounded lock set. No copied wire field selects a source.
pub(crate) fn peer_operation_scopes(
    c: &Connection,
    actor: &Actor,
    claim: &PeerOperationClaim,
) -> Result<Vec<AuthorityScope>> {
    operation_source_scopes(c, actor, claim, true)
}
pub(crate) fn operation_source_scopes(
    c: &Connection,
    actor: &Actor,
    claim: &PeerOperationClaim,
    require_permission: bool,
) -> Result<Vec<AuthorityScope>> {
    rows::authority_operation_scopes(c, actor, claim, require_permission)
}

/// Protected reply projection after an exact admitted control mutation. The
/// public operation.get floor is separate; original command authority still applies.
pub(crate) fn current_control_record(
    c: &Connection,
    actor: &Actor,
    id: &str,
) -> Result<OperationRecord> {
    let row = rows::find(c, &authority::tenant(c, actor)?, &actor.identity_id, id)?
        .ok_or_else(Error::forbidden)?;
    if !row.control {
        return Err(Error::forbidden());
    }
    authority::read(c, actor, &row)?;
    let mut result = row.record.clone();
    result.result = authority::local_result(c, actor, &row)?;
    Ok(result)
}
