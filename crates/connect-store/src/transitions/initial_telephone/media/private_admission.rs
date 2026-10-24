//! Bounded authority for the original quarantined SIP candidate, not a public
//! media read or the continuing authority of an established private context.
use super::*;

fn source(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
) -> Result<((Actor, Actor, i64), Vec<AuthorityScope>)> {
    historical_lifetime(store, c, &origin.signal)?;
    let original = origin.signal.origin();
    if !original.source.is_private_candidate() || crate::now() >= original.expires_at {
        return Err(Error::forbidden());
    }
    let caller = actor_from_connection(c, &original.credential)?;
    let ((source, endpoint, route, expiry), required) = super::super::private::admission_source(
        store,
        c,
        &caller,
        &original.connection,
        &original.policy,
    )?;
    let unclaimed:bool=c.query_row("SELECT state='accepted' AND lease_owner IS NULL AND cancel_requested=0 AND NOT EXISTS(SELECT 1 FROM operation_effects WHERE operation_record_id=?1) FROM operations WHERE record_id=?1",[original.source.parent.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if source != original.source || !unclaimed {
        return Err(Error::forbidden());
    }
    Ok(((endpoint, route, expiry), required))
}
pub(super) fn scopes(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
) -> Result<Vec<AuthorityScope>> {
    let mut required = current_scopes(c, origin.signal.origin(), false)?;
    required.extend(source(store, c, origin)?.1);
    AuthorityScope::merged(&required)
}
pub(super) fn projection(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
    until: i64,
) -> Result<InitialTelephoneProjection> {
    let ((endpoint, route, expiry), _) = source(store, c, origin)?;
    let original = origin.signal.origin();
    let s = &original.source;
    let (connected,valid):(bool,bool)=c.query_row("SELECT a.connected_at IS NOT NULL,a.accepted=1 AND a.acceptance_kind='private-connection' AND a.acceptance_record_id=n.operation_record_id AND a.observation_json=?2 AND a.codec=?3 AND a.retired_at IS NULL AND own.consultation_record_id=n.parent_record_id AND own.endpoint_credential_id=?4 AND own.endpoint_id=?5 AND own.endpoint_revision=?6 AND own.participant_id=?7 AND own.membership_epoch=?8 AND own.codec=a.codec AND own.audience_json=json_extract(n.source_json,'$.audience') AND child.cancel_requested=0 AND ((a.connected_at IS NULL AND child.state='preparing') OR (a.connected_at IS NOT NULL AND child.state='completed' AND json_extract(a.connected_json,'$.private_audio')=1 AND json_extract(a.connected_json,'$.private_consultation_record_id')=n.parent_record_id)) FROM initial_telephone_operations n JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id JOIN initial_telephone_private_acceptances own ON own.operation_record_id=n.operation_record_id JOIN operations child ON child.record_id=n.operation_record_id WHERE n.operation_record_id=?1",params![original.record,origin.observation,origin.codec,original.policy.endpoint_credential_id,s.endpoint_id,s.endpoint_revision,s.target.participant_id,s.target.membership_epoch],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    if !valid {
        return Err(Error::forbidden());
    }
    let session = crate::session_control::view(c, &s.conversation_id, &s.session_id)?;
    if !matches!(session.state.as_str(), "accepted" | "ready" | "active") {
        return Err(Error::forbidden());
    }
    let (role,revision):(String,u64)=c.query_row("SELECT p.role,v.revision FROM session_participants p JOIN conversations v ON v.id=p.conversation_id WHERE p.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND p.state='present' AND v.closed=0",params![s.session_id,s.target.participant_id,s.target.membership_epoch],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::forbidden())?;
    let expires = until.min(expiry).min(original.expires_at);
    if expires <= crate::now() {
        return Err(Error::forbidden());
    }
    Ok(InitialTelephoneProjection {
        endpoint_actor: endpoint,
        route_actor: route,
        member: s.target.clone(),
        session,
        role,
        accepted: json!({"audio":[origin.codec]}),
        conversation_revision: revision,
        expires,
        connected,
    })
}
