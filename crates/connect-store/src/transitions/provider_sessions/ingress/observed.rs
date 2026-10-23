//! Common observed inbound resource admission; never submits provider creation.
use super::*;
pub(super) fn insert(
    c: &Connection,
    store: &Store,
    scan: &ProviderVoiceScan,
    receipt: &InboundVoiceReceipt,
    held: &crate::AuthorityGuardSet,
) -> Result<String> {
    let actor = actor_from_connection(c, &scan.ingress.policy.assistant_credential_id)?;
    let r = &receipt.call;
    let request = request(scan, &r.provider_id);
    let operation = Operation::SessionControl {
        request: request.clone(),
    };
    let result = crate::operations::execute_prepared(
        c,
        &actor,
        &operation,
        &crate::DiscoveryRuntime::default(),
        store,
        Some(held),
        None,
    )?;
    result.outcome?;
    let (source, expiry) = super::super::source(
        c,
        &actor,
        &request,
        &scan.binding.target.participant_id,
        &scan.ingress.policy,
    )?;
    let id = format!(
        "pvoice_{}",
        hex::encode(Sha256::digest(format!(
            "{}\0{}",
            source.invitation_record_id, source.target.participant_id
        )))
    );
    let (total, active): (u64, u64) = c
        .query_row(
            "SELECT count(*),COALESCE(SUM(closed_at IS NULL),0) FROM provider_session_voice",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    if total >= store.limits.operation_receipts
        || (active >= store.limits.connections as u64 && !State::Voice(receipt.state).terminal())
    {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming provider call budget exhausted",
        ));
    }
    let mut projection =
        Projection::new(id.clone(), State::Voice(VoiceState::Starting)).map_err(|_| invalid())?;
    projection
        .apply(
            Observation {
                mutation_id: id.clone(),
                external_id: Some(r.provider_id.clone()),
                event_id: receipt.event_id.clone(),
                state: State::Voice(receipt.state),
                verification: receipt.verification,
            },
            true,
        )
        .map_err(|_| invalid())?;
    let expires = expiry.min(crate::now() + i64::from(scan.ingress.policy.timeout_seconds));
    c.execute("INSERT INTO provider_session_voice(id,invitation_record_id,caller_credential_id,target_participant_id,endpoint_id,source_json,policy_json,projection_json,expires_at,direction,ingress_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'inbound',?10)",params![id,source.invitation_record_id,actor.credential_id,source.target.participant_id,source.endpoint_id,lifecycle::encoded(&source)?,lifecycle::encoded(&scan.ingress.policy)?,lifecycle::encoded(&projection)?,expires,scan.ingress.id]).map_err(|_|Error::storage())?;
    let observed = format!("pvobserve_{id}");
    c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,provider_ref) VALUES(?1,?2,?3,?4,'status-query','confirmed',1,1,?5)",params![observed,source.invitation_record_id,scan.ingress.policy.provider,id,r.provider_id]).map_err(|_|Error::storage())?;
    c.execute("UPDATE provider_session_voice SET create_started=1,create_effect_id=?2,attempt_runtime=?3,provider_id=?4,control_url=?5,ended_reason=?6,observed_at=?7 WHERE id=?1",params![id,observed,store.runtime_epoch(),r.provider_id,r.control_url,r.ended_reason,crate::now()]).map_err(|_|Error::storage())?;
    if State::Voice(receipt.state).terminal() {
        let end = format!("pvend_{id}");
        c.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id,provider_ref) VALUES(?1,?2,?3,'cleanup_'||?4,'status-query','confirmed',1,1,?5,?6)",params![end,source.invitation_record_id,scan.ingress.policy.provider,id,observed,r.provider_id]).map_err(|_|Error::storage())?;
        c.execute("UPDATE provider_session_voice SET end_started=1,end_effect_id=?2,end_runtime=?3,closed_at=?4 WHERE id=?1",params![id,end,store.runtime_epoch(),crate::now()]).map_err(|_|Error::storage())?;
    }
    lifecycle::touch(c, &id, store)?;
    Ok(id)
}
pub(super) fn origin(
    c: &Connection,
    store: &Store,
    scan: &ProviderVoiceScan,
    call_id: String,
) -> Result<ProviderSessionVoiceOrigin> {
    let source = load_source(c, &call_id)?;
    let expires: i64 = c
        .query_row(
            "SELECT expires_at FROM provider_session_voice WHERE id=?1",
            [&call_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(ProviderSessionVoiceOrigin {
        owner: Arc::downgrade(&scan.owner),
        runtime: store.runtime_epoch().into(),
        credential: scan.ingress.policy.assistant_credential_id.clone(),
        id: call_id,
        source,
        policy: scan.ingress.policy.clone(),
        expires,
        direction: ProviderCallDirection::Inbound,
    })
}
