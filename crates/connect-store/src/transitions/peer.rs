//! Canonical operation recovery retains exact metadata writers and projection readers.
use super::*;
use crate::{AuthorityMode, AuthorityScope, PeerOperationClaim};
use connect_protocol::provisioning::ProvisioningOutcome;
use std::collections::BTreeSet;

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
fn projection_scopes(
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
    let (target, scope) = action_scope(&claim.action);
    id(target)?;
    if require_permission {
        actor.require(scope)?;
    }
    let tenant = authority::tenant(c, actor)?;
    let row = rows::find(c, &tenant, &actor.identity_id, target)?.ok_or_else(Error::forbidden)?;
    for (asserted, actual) in [
        (&claim.conversation_id, &row.record.conversation_id),
        (&claim.session_id, &row.record.session_id),
        (&claim.connection_id, &row.record.connection_id),
    ] {
        if asserted
            .as_ref()
            .is_some_and(|id| Some(id) != actual.as_ref())
        {
            return Err(Error::forbidden());
        }
    }
    let mut sources = vec![];
    let mut seen = BTreeSet::new();
    let mut current = rows::by_record(c, &row.record.record_id)?;
    loop {
        if seen.len() >= 32 || !seen.insert(current.record.record_id.clone()) {
            return Err(Error::new(
                "capacity-exceeded",
                "Operation source depth exceeded",
            ));
        }
        sources.extend(admission::row_scopes(&current)?);
        if let Some(cid) = &current.record.conversation_id {
            sources.extend(crate::authority_admission::conversation_scopes(
                c,
                cid,
                AuthorityMode::Read,
            )?);
        }
        let mut members=c.prepare("SELECT identity_id FROM operation_members WHERE operation_record_id=?1 ORDER BY identity_id").map_err(|_|Error::storage())?;
        for identity in members
            .query_map([&current.record.record_id], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
        {
            add_identity(&mut sources, &identity.map_err(|_| Error::storage())?)?;
        }
        let next: Option<(String,String)> = c
            .query_row(
                "SELECT target_record_id,action FROM operation_decisions WHERE operation_record_id=?1",
                [&current.record.record_id],
                |r| Ok((r.get(0)?,r.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some((_, action)) = &next {
            if current.ledger || current.record.command_type != format!("operation.{action}") {
                return Err(Error::forbidden());
            }
        }
        if next.is_none() && !current.ledger {
            projection_scopes(c, &current, &mut sources)?;
        }
        // Match the fence manager's bounded distinct resource footprint without
        // counting duplicate source/member entries as additional authority.
        sources = AuthorityScope::merged(&sources)?;
        let Some((next, _)) = next else {
            break;
        };
        current = rows::by_record(c, &next)?;
    }
    // Validate current typed provenance only after the bounded decision walk;
    // authority::read follows those links recursively.
    authority::read(c, actor, &row)?;
    Ok(sources)
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
