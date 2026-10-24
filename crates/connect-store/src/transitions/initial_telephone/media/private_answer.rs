//! Own consent for an answered original SIP candidate, never public presence.
use super::*;

fn current(
    store: &Store,
    c: &Connection,
    capture: &InitialTelephoneAnswerCapture,
    held: &AuthorityGuardSet,
) -> Result<()> {
    let origin = capture.signal.origin();
    if !origin.source.is_private_candidate() {
        return Err(Error::forbidden());
    }
    let (endpoint, _, _) = origin.current(store, c)?;
    endpoint.require("session.accept")?;
    if !store.authority_covers(
        held,
        &super::super::private::allocation_scopes(store, c, origin, true)?,
    ) {
        return Err(Error::forbidden());
    }
    Ok(())
}

pub(super) fn accept(
    store: &Store,
    c: &Connection,
    capture: &InitialTelephoneAnswerCapture,
    held: &AuthorityGuardSet,
) -> Result<()> {
    current(store, c, capture, held)?;
    let origin = capture.signal.origin();
    let s = &origin.source;
    c.execute("INSERT INTO initial_telephone_private_acceptances(operation_record_id,consultation_record_id,endpoint_credential_id,endpoint_id,endpoint_revision,participant_id,membership_epoch,codec,audience_json,accepted_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![origin.record,s.parent.record_id(),origin.policy.endpoint_credential_id,s.endpoint_id,s.endpoint_revision,s.target.participant_id,s.target.membership_epoch,capture.codec,serde_json::to_string(&s.audience).map_err(|_|Error::storage())?,crate::now()]).map_err(|_|Error::storage())?;
    Ok(())
}

pub(super) fn require(
    store: &Store,
    c: &Connection,
    capture: &InitialTelephoneAnswerCapture,
    held: &AuthorityGuardSet,
) -> Result<()> {
    current(store, c, capture, held)?;
    let origin = capture.signal.origin();
    let s = &origin.source;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_private_acceptances own JOIN initial_telephone_answers a ON a.operation_record_id=own.operation_record_id WHERE own.operation_record_id=?1 AND own.consultation_record_id=?2 AND own.endpoint_credential_id=?3 AND own.endpoint_id=?4 AND own.endpoint_revision=?5 AND own.participant_id=?6 AND own.membership_epoch=?7 AND own.codec=?8 AND own.audience_json=?9 AND a.observation_json=?10 AND a.accepted=1 AND a.acceptance_kind='private-connection' AND a.acceptance_record_id=own.operation_record_id AND a.retired_at IS NULL)",params![origin.record,s.parent.record_id(),origin.policy.endpoint_credential_id,s.endpoint_id,s.endpoint_revision,s.target.participant_id,s.target.membership_epoch,capture.codec,serde_json::to_string(&s.audience).map_err(|_|Error::storage())?,capture.observation],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
