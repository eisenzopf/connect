//! Explicit original-conflict custody, separate from ordinary reconciliation.
use super::*;
pub(super) struct Ticket {
    digest: String,
}
fn digest(raw: &str) -> String {
    hex::encode(Sha256::digest(raw.as_bytes()))
}
pub(super) fn admit(c: &Connection, origin: &ProviderSessionVoiceOrigin) -> Result<Ticket> {
    let raw:Option<String>=c.query_row("SELECT p.quarantine_json FROM provider_session_voice p JOIN operation_effects e ON e.id=p.create_effect_id WHERE p.id=?1 AND p.direction='outbound' AND p.provider_id IS NULL AND p.closed_at IS NULL AND e.state='unknown' AND NOT EXISTS(SELECT 1 FROM provider_voice_quarantine_resolutions r WHERE r.call_id=p.id)",[&origin.id],|r|r.get(0)).optional().map_err(|_|Error::storage())?.flatten();
    let raw = raw.ok_or_else(|| {
        Error::new(
            "reconciliation-required",
            "Only an original unresolved binding conflict can obtain resolution custody",
        )
    })?;
    Ok(Ticket {
        digest: digest(&raw),
    })
}
/// True means a replay of an already-resolved original ticket. It never updates
/// a later status or replaces the resource selected by the first positive proof.
pub(super) fn replay(c: &Connection, captured: &ProviderSessionVoiceCapture) -> Result<bool> {
    let ticket = captured.quarantine.as_ref().ok_or_else(Error::forbidden)?;
    let raw: String = c
        .query_row(
            "SELECT quarantine_json FROM provider_session_voice WHERE id=?1",
            [&captured.origin.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    if ticket.digest != digest(&raw) {
        return Err(Error::forbidden());
    }
    let resolved: Option<String> = c
        .query_row(
            "SELECT provider_id FROM provider_voice_quarantine_resolutions WHERE call_id=?1",
            [&captured.origin.id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if let Some(provider) = resolved {
        if captured.provider_id.as_deref() != Some(provider.as_str()) {
            return Err(conflict());
        }
        return Ok(true);
    }
    Ok(false)
}
pub(super) fn record(c: &Connection, captured: &ProviderSessionVoiceCapture) -> Result<()> {
    let ticket = captured.quarantine.as_ref().ok_or_else(Error::forbidden)?;
    let raw: String = c
        .query_row(
            "SELECT quarantine_json FROM provider_session_voice WHERE id=?1",
            [&captured.origin.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    if ticket.digest != digest(&raw)
        || captured.observation.verification != Verification::Reconciliation
    {
        return Err(Error::forbidden());
    }
    let provider = captured.provider_id.as_ref().ok_or_else(Error::forbidden)?;
    let proof = crate::provisioning::canonical_json(
        &serde_json::json!({"observation":captured.observation,"provider_id":provider,"control_url":captured.control_url,"ended_reason":captured.ended_reason}),
    )?;
    c.execute("INSERT INTO provider_voice_quarantine_resolutions(call_id,quarantine_json,provider_id,proof_json,resolved_at) VALUES(?1,?2,?3,?4,?5)",params![captured.origin.id,raw,provider,proof,crate::now()]).map_err(|_|Error::storage())?;
    Ok(())
}
