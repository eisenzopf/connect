//! Exact historical confirmation after the original native custodian retired.
//! This returns metadata scopes only, never an old-source media/scene grant.
use super::*;

pub(super) fn authority(
    captured: &NativeReplacementCommitted,
    store: &Store,
    c: &Connection,
    held: Option<&Arc<AuthorityGuardSet>>,
) -> Result<(Vec<AuthorityScope>, i64)> {
    let a = &captured.attempt;
    let start = &a.origin.started.attempt;
    let allocated = &start.allocated;
    let p = &allocated.attempt.prepared;
    local_owner(store, &p.owner, &p.runtime)?;
    if a.private_successor.is_some() {
        return Err(Error::forbidden());
    }
    // The exact same reserved plan and three already-confirmed physical phases
    // remain bound. Only old-source liveness is intentionally not repeated.
    let raw: Option<(String,i64)> = c.query_row(
        "SELECT n.plan_json,n.expires_at FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id AND e.provider='native-replacement-prepare' AND e.correlation_key=n.operation_record_id AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 JOIN operations o ON o.record_id=n.operation_record_id AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7",
        params![p.record,p.runtime,p.reservation,p.hash,p.attempt,p.effect,p.observation],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|_|Error::storage())?;
    let (raw, expires) = raw.ok_or_else(Error::forbidden)?;
    if crate::now() >= expires {
        return Err(Error::forbidden());
    }
    let plan: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    require_confirmed_allocation(c, allocated)?;
    let exact:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id AND e.provider='native-replacement-start' AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 WHERE s.operation_record_id=?1 AND s.phase='start' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json=?4 AND e.observation_json=?5)",
        params![p.record,start.attempt,start.effect,a.origin.started.observation,start.target],|r|r.get(0)
    ).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    let (meta, key, mut scopes, expiry) = crate::peer_authority::checked(store, c, &start.request)?;
    let descriptor: serde_json::Value =
        serde_json::from_str(&start.target).map_err(|_| Error::storage())?;
    let target = &a.target;
    let original = p.selection.origin.request();
    if descriptor["credential_id"] != meta.actor.credential_id
        || descriptor["key_id"] != start.request.key_id
        || descriptor["public_key"] != serde_json::json!(key.public_key)
        || descriptor["device_id"] != key.device_id
        || descriptor["intent"]["offer"]
            .get("private_consultation")
            .is_some()
        || descriptor["intent"]["offer"]
            .get("handoff_preparation")
            .is_some()
        || target.session.actor.identity_id != meta.actor.identity_id
        || target.connection.conversation_id != original.conversation_id
        || target.connection.session_id != original.session_id
        || plan["candidate_connection_id"] != target.connection.connection_id
        || plan["candidate_runtime_id"] != target.runtime_connection_id
        || plan["generation"] != target.connection.generation
        || plan["expected_preparation"]["endpoint"] != target.connection.endpoint_id
        || plan["expected_preparation"]["device"] != target.device_id
        || plan["expected_preparation"]["participant"] != target.connection.participant_id
        || plan["expected_preparation"]["membership_epoch"] != target.connection.membership_epoch
    {
        return Err(Error::forbidden());
    }
    let member =
        crate::audience::member(c, &meta.actor.identity_id, &original.conversation_id, true)?;
    if member.participant_id != target.connection.participant_id
        || member.membership_epoch != target.connection.membership_epoch
    {
        return Err(Error::forbidden());
    }
    // continuation_authority pins frozen endpoint revisions as well. Verify the
    // target device and transport/identity against the exact captured descriptor.
    let endpoint:bool=c.query_row(
        "SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 JOIN operation_endpoints f ON f.endpoint_id=e.id AND f.revision=e.revision AND f.operation_record_id=?1 WHERE e.id=?2 AND e.identity_id=?3 AND e.device_id=?4 AND e.verification_state='verified' AND e.revision=?5 AND d.kind=?6 AND d.platform=?7 AND e.transport=?8)",
        params![p.record,target.connection.endpoint_id,target.session.actor.identity_id,target.device_id,plan["target_endpoint_revision"].as_u64().ok_or_else(Error::forbidden)?,plan["target_device_kind"].as_str().ok_or_else(Error::forbidden)?,plan["target_device_platform"].as_str().ok_or_else(Error::forbidden)?,serde_json::to_value(&target.endpoint_transport).map_err(|_|Error::storage())?.as_str().ok_or_else(Error::forbidden)?],|r|r.get(0)
    ).map_err(|_|Error::storage())?;
    if !endpoint {
        return Err(Error::forbidden());
    }
    let (selection, until) =
        p.selection
            .retired_commit_ledger_authority(store, c, &plan, &meta.actor.identity_id)?;
    scopes.extend(selection);
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        &original.conversation_id,
        crate::AuthorityMode::Read,
    )?);
    let scopes = AuthorityScope::merged(&scopes)?;
    if held.is_some_and(|held| !store.authority_covers(held, &scopes)) {
        return Err(Error::forbidden());
    }
    let until = until
        .min(expiry)
        .min(expires)
        .min(a.source_expires_at)
        .min(p.selection.physical_expiry())
        .min(original.expires_at);
    if crate::now() >= until {
        return Err(Error::forbidden());
    }
    Ok((scopes, until))
}
