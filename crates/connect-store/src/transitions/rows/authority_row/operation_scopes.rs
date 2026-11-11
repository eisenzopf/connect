//! Current operation provenance without constructing a public status page.
//! The private row never escapes as a reply or retained authorization grant.
use super::*;
use crate::transitions::{admission, peer};
use crate::{AuthorityMode, AuthorityScope, PeerOperationClaim};
use std::collections::BTreeSet;

pub(in crate::transitions) fn operation_source_scopes(
    c: &Connection,
    actor: &Actor,
    claim: &PeerOperationClaim,
    require_permission: bool,
) -> Result<Vec<AuthorityScope>> {
    let (target, scope) = peer::action_scope(&claim.action);
    id(target)?;
    if require_permission {
        actor.require(scope)?;
    }
    let tenant = authority::tenant(c, actor)?;
    let row = rows::find_for_authority(c, &tenant, &actor.identity_id, target)?
        .ok_or_else(Error::forbidden)?;
    let row = &row.0;
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
    let mut loaded;
    let mut current = row;
    loop {
        if seen.len() >= 32 || !seen.insert(current.record.record_id.clone()) {
            return Err(Error::new(
                "capacity-exceeded",
                "Operation source depth exceeded",
            ));
        }
        if matches!(&claim.action, OperationAction::Get { .. }) {
            sources.extend(admission::row_scopes(current)?);
        } else {
            sources.extend(admission::mutation_row_scopes(current)?);
        }
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
            sources.push(AuthorityScope::identity(
                &identity.map_err(|_| Error::storage())?,
                AuthorityMode::Read,
            )?);
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
            peer::projection_scopes(c, current, &mut sources)?;
        }
        // Match the fence manager's bounded distinct resource footprint without
        // counting duplicate source/member entries as additional authority.
        sources = AuthorityScope::merged(&sources)?;
        let Some((next, _)) = next else {
            break;
        };
        loaded = rows::for_authority(c, &next)?;
        current = &loaded.0;
    }
    // Validate current typed provenance only after the bounded decision walk;
    // authority::read follows those links recursively.
    authority::read(c, actor, row)?;
    Ok(sources)
}
