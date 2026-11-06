//! Domain eligibility for genuine receiver-epoch metadata, never public Ready.
use super::*;

pub(super) fn require_current(
    c: &Connection,
    origin: &NativeReplacementMediaOrigin,
    saved: &serde_json::Value,
) -> Result<()> {
    // media_facts already resolved this same captured start and current target
    // key/credential, original operation, source/retained scene and expiry under
    // the enclosing version seal. Metadata cannot add a receive permission.
    let start = &origin.started.attempt;
    let prepared = &start.allocated.attempt.prepared;
    let credential = start
        .request
        .credentials
        .last()
        .ok_or_else(Error::forbidden)?;
    actor_from_connection(c, credential)?.require("stream.subscribe")?;

    let advanced: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1) OR EXISTS(SELECT 1 FROM native_replacement_lifetimes WHERE operation_record_id=?1)",
        [&prepared.record], |row| row.get(0),
    ).map_err(|_| Error::storage())?;
    if advanced {
        // This exact private capability has ended. The same original writer
        // may wait for its ordinary policy; it cannot reuse this private read.
        return Err(Error::new(
            "authority-busy",
            "Original private epoch phase advanced",
        ));
    }
    let candidate = saved["candidate_runtime_id"]
        .as_str()
        .ok_or_else(Error::storage)?;
    let generation = saved["generation"].as_u64().ok_or_else(Error::storage)?;
    let confirmed: u64 = c.query_row(
        "SELECT count(*) FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase IN ('attach','activate') AND s.started=1 AND s.observation_json IS NOT NULL AND s.observation_json=e.observation_json AND e.provider='native-replacement-'||s.phase AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 AND json_extract(s.observation_json,'$.candidate')=?2 AND json_extract(s.observation_json,'$.generation')=?3 AND json_extract(s.observation_json,'$.phase')=s.phase AND json_extract(s.observation_json,'$.attached')=1 AND json_extract(s.observation_json,'$.private')=1 AND json_extract(s.observation_json,'$.activated_muted')=(s.phase='activate') AND json_extract(s.observation_json,'$.media_ready')=0",
        params![prepared.record,candidate,generation], |row| row.get(0),
    ).map_err(|_| Error::storage())?;
    if confirmed != 2 {
        return Err(Error::new(
            "not-ready",
            "Original private resources are not confirmed active",
        ));
    }
    // Decoded readiness is deliberately not a prerequisite for the very epoch
    // metadata that lets the target decode its original private video. The
    // Service separately verifies a genuine receiver-only observation against
    // the actual current Core preroll and selected source/stream incarnation.
    Ok(())
}
