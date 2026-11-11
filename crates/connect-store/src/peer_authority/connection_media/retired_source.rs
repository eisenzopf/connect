//! Current authority metadata for an exact retired original, never old-source IO.
use super::*;

pub(super) fn authority(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
    original: &crate::NativeReplacementOrigin,
    plan: &serde_json::Value,
) -> Result<(Vec<AuthorityScope>, i64)> {
    current_owner(store, origin)?;
    credential_refresh::require_epoch(store, c, origin)?;
    origin.attempt.historical(c, store)?;
    if original.request().connection_id.as_deref() != Some(origin.connection_id())
        || original.request().conversation_id != origin.conversation_id()
        || original.request().session_id != origin.session_id()
        || plan["source_generation"].as_u64() != Some(origin.generation())
        || plan["expected_preparation"]["source"].as_str() != Some(origin.runtime_connection_id())
        || plan["expected_preparation"]["participant"].as_str() != Some(origin.participant_id())
        || plan["expected_preparation"]["membership_epoch"].as_u64()
            != Some(origin.membership_epoch())
        || origin.intent.offer.get("private_consultation").is_some()
        || origin.intent.offer.get("handoff_preparation").is_some()
        || crate::connection_control::private_consultation::replacement_lineage::floor(
            c,
            origin.connection_id(),
        )?
        .is_some()
    {
        return Err(Error::forbidden());
    }
    let (meta, key, mut scopes, expiry) = checked(store, c, &origin.binding)?;
    if key.public_key != origin.public_key {
        return Err(Error::forbidden());
    }
    meta.actor.require("connection.offer")?;
    let recipients = crate::connection_control::intent_resources(c, &meta.actor, &origin.intent)?;
    if recipients
        .iter()
        .any(|id| !meta.actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    // historical() above binds the opaque original attempt. This additionally
    // proves the EXACT one-generation retirement and current credential epoch,
    // without treating a reused/more-recent/foreign slot as the retired source.
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_admission_sources n JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed' AND o.cancel_requested=0 AND o.tombstoned=0 JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 JOIN native_connection_lifetimes l ON l.operation_record_id=n.operation_record_id AND l.connection_id=n.connection_id AND l.runtime_epoch=n.attempt_runtime AND l.generation=n.generation JOIN connections v ON v.id=n.connection_id JOIN connection_offers mapping ON mapping.connection_id=v.id WHERE n.operation_record_id=?1 AND n.connection_id=?2 AND n.attempt_runtime=?3 AND n.generation=?4 AND n.endpoint_id=?5 AND n.participant_id=?6 AND n.membership_epoch=?7 AND n.credential_id=?8 AND n.key_id=?9 AND n.device_id=?10 AND n.source_expires_at=?11 AND n.effect_started=1 AND l.retired=1 AND v.state='retired' AND v.generation=n.generation+1 AND v.session_id=?12 AND v.participant_id=n.participant_id AND v.membership_epoch=n.membership_epoch AND v.identity_id=?13 AND v.endpoint_id=n.endpoint_id AND v.observed_runtime_id IS NULL AND v.observed_at IS NULL AND v.readiness_json IS NULL AND mapping.runtime_connection_id=?14)",params![origin.offer_record_id(),origin.connection_id(),store.runtime_epoch(),origin.generation(),origin.endpoint_id,origin.participant_id(),origin.membership_epoch(),meta.actor.credential_id,origin.binding.key_id,origin.binding.device.id,origin.expires_at,origin.session_id(),meta.actor.identity_id,origin.runtime_connection_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    let member =
        crate::audience::member(c, &meta.actor.identity_id, origin.conversation_id(), true)?;
    if member.participant_id != origin.participant_id
        || member.membership_epoch != origin.membership_epoch
    {
        return Err(Error::forbidden());
    }
    let role:Option<String>=c.query_row("SELECT sp.role FROM session_participants sp JOIN session_presence_capabilities cap ON cap.session_id=sp.session_id AND cap.participant_id=sp.participant_id AND cap.membership_epoch=sp.membership_epoch JOIN sessions s ON s.id=sp.session_id AND s.conversation_id=?4 WHERE sp.session_id=?1 AND sp.participant_id=?2 AND sp.membership_epoch=?3 AND sp.state='present' AND s.state IN ('accepted','ready','active') AND s.topology_revision=?5",params![origin.session_id(),origin.participant_id(),origin.membership_epoch(),origin.conversation_id(),original.request().expected_revision],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let expected_role = plan["source_role"].as_str().ok_or_else(Error::forbidden)?;
    if role.as_deref() != Some(expected_role) {
        return Err(Error::forbidden());
    }
    let endpoint:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.device_id=?3 AND e.verification_state='verified')",params![origin.endpoint_id,meta.actor.identity_id,origin.binding.device.id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !endpoint {
        return Err(Error::forbidden());
    }
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        origin.conversation_id(),
        AuthorityMode::Read,
    )?);
    Ok((
        AuthorityScope::merged(&scopes)?,
        expiry.min(origin.expires_at),
    ))
}
