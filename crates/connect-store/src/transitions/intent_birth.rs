//! Accepted endpoint-replacement intent freezes current sources, never a route.
use super::*;
use crate::{AuthorityMode, AuthorityScope};

pub(super) fn endpoint(request: &TransitionRequest) -> bool {
    matches!(
        request.intent,
        TransitionIntent::Replace {
            endpoint_id: Some(_),
            candidate_connid: None,
            ..
        }
    ) && request.connection_id.is_some()
}

pub(super) fn eligible(
    c: &Connection,
    actor: &Actor,
    request: &TransitionRequest,
    store: &Store,
) -> Result<bool> {
    if !endpoint(request)
        || !crate::session_participant_lifecycle::reconcile_is_unchanged(
            c,
            store,
            &request.session_id,
        )?
    {
        return Ok(false);
    }
    // A new intent cannot satisfy, replace, or alter enrollment completion.
    let enrollment: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM client_enrollments WHERE tenant_id=?1 AND issuer_identity_id=?2 AND complete_operation_id=?3)",
        params![authority::tenant(c, actor)?, actor.identity_id, request.operation_id], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    Ok(!enrollment)
}

pub(super) fn readers(
    c: &Connection,
    actor: &Actor,
    request: &TransitionRequest,
) -> Result<Vec<AuthorityScope>> {
    // Reconcile's no-op witness reads all current appearances in this Session.
    // Retain their current authority through the same SQL transaction.
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &request.conversation_id,
        AuthorityMode::Read,
    )?;
    scopes.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    Ok(scopes)
}

// Pure replay is deliberately limited to this untouched Accepted namespace.
// Any worker, effect, child reservation or due recovery takes the old Writer path.
pub(super) fn accepted_replay(c: &Connection, row: &rows::Saved, store: &Store) -> Result<bool> {
    if !row.ledger
        || row.native_replacement
        || row.tombstoned
        || row.record.cancel_requested
        || row.record.state != OperationState::Accepted
        || row.worker.is_some()
        || row.generation != 0
        || row.record.expires_at.is_none_or(|t| t <= crate::now())
        || !row.record.effects.is_empty()
        || !endpoint(&rows::request(row)?)
    {
        return Ok(false);
    }
    let child: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacements WHERE operation_record_id=?1) OR EXISTS(SELECT 1 FROM native_replacement_candidates WHERE operation_record_id=?1)",
        [&row.record.record_id], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    let _ = store;
    Ok(!child)
}

/// Recompute exact frozen tuples under the actual primary write transaction.
/// The constructor calls this independently of the admission caller's source set.
pub(crate) fn facts(
    c: &Connection,
    store: &Store,
    credential: &str,
    request: &TransitionRequest,
    record: &str,
) -> Result<(Vec<AuthorityScope>, String, String)> {
    let actor = actor_from_connection(c, credential)?;
    actor.require(request.intent.required_scope())?;
    if !eligible(c, &actor, request, store)?
        || request.expires_at <= crate::now()
        || request.expires_at - crate::now() > MAX_OPERATION_TTL
    {
        return Err(Error::forbidden());
    }
    let exists: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM operations WHERE record_id=?1 OR (tenant_id=?2 AND owner_id=?3 AND operation_id=?4))",
        params![record, authority::tenant(c, &actor)?, actor.identity_id, request.operation_id], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    if exists {
        return Err(conflict());
    }
    crate::operations::reject_control_id(c, &actor, &request.operation_id)?;
    let sources = authority::admission(c, &actor, request, store)?;
    let expected = authority::intent_birth_rows(c, record, &sources)?;
    let mut value = serde_json::to_value(request).map_err(|_| invalid())?;
    value
        .as_object_mut()
        .ok_or_else(invalid)?
        .remove("operation_id");
    let (hash, intent) = digest(&value)?;
    let root: String = c
        .query_row(
            "SELECT json_array(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                record,
                authority::tenant(c, &actor)?,
                actor.identity_id,
                request.operation_id,
                hash,
                request.conversation_id,
                request.session_id,
                request.connection_id,
                request.expected_revision,
                intent,
                request.expires_at
            ],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok((readers(c, &actor, request)?, expected, root))
}
