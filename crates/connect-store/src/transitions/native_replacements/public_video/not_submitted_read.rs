//! Historical cleanup ONLY for an exact still-owned pre-Core negative capture.
//! Ordinary historical()/CommitUnknown admission is intentionally untouched.
use super::*;
impl Store {
    /// No current target/source grant or preparation renewal. The SAME actual
    /// negative token must first be durably recorded; a raw failed/Unknown row
    /// cannot construct this interval. Physical whole-candidate close and fresh
    /// OLD restoration authority remain separate mandatory facts.
    pub fn native_replacement_not_submitted_unpublished_read(
        &self,
        original: Arc<NativeReplacementPrepared>,
        captured: &NativeReplacementCommitNotSubmitted,
        wait: &crate::AuthorityWait,
    ) -> Result<NativeReplacementUnpublishedRead> {
        wait.check()?;
        let lineage = captured.unpublished_lineage(&original)?;
        let (saved, scopes) = {
            let c = self.try_connection_with_wait(wait)?;
            historical_not_submitted(self, &c, &original, &lineage)?
        };
        let held = self.acquire_authority(&scopes, wait)?;
        // No SQL wait with a retained Reader; exact metadata is sealed again.
        let c = self.try_connection()?;
        let (actual, required) = historical_not_submitted(self, &c, &original, &lineage)?;
        if actual != saved || !self.authority_covers(&held, &required) {
            return Err(Error::forbidden());
        }
        wait.check()?;
        let(actor_identity,original_expiry):(String,i64)=c.query_row(
            "SELECT o.owner_id,n.expires_at FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1",
            [&original.record],|row|Ok((row.get(0)?,row.get(1)?)),
        ).map_err(|_|Error::storage())?;
        Ok(NativeReplacementUnpublishedRead {
            original,
            saved,
            held,
            wait: wait.clone(),
            actor_identity,
            original_expiry,
            not_submitted: Some(lineage),
        })
    }
}
pub(super) fn historical_not_submitted(
    store: &Store,
    c: &Connection,
    p: &Arc<NativeReplacementPrepared>,
    lineage: &NativeReplacementNotSubmittedLineage,
) -> Result<(serde_json::Value, Vec<AuthorityScope>)> {
    local_owner(store, &p.owner, &p.runtime)?;
    lineage.require_durable(c, p)?;
    // This separate closed query permits generic recovery Unknown only after
    // its exact durable owned NotSubmitted fact proves no Core movement. Actual
    // CommitUnknown/confirmed/submitting cannot satisfy require_durable above.
    let raw:Option<(String,String,String)>=c.query_row(
        "SELECT n.plan_json,o.owner_id,o.conversation_id FROM native_replacements n
         JOIN operations o ON o.record_id=n.operation_record_id
         JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare'
         JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id
         WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4
         AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7 AND e.state='confirmed'
         AND o.command_type='connection.replace' AND o.state IN ('preparing','cancelled','failed','unknown')",
        params![p.record,p.runtime,p.reservation,p.hash,p.attempt,p.effect,p.observation],
        |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional().map_err(|_|Error::storage())?;
    let (raw, owner, cid) = raw.ok_or_else(Error::forbidden)?;
    let saved: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    for key in ["candidate_connection_id", "candidate_runtime_id"] {
        id(string(&saved, key)?)?;
    }
    for key in ["participant", "endpoint", "device"] {
        id(string(&saved["expected_preparation"], key)?)?;
    }
    never_public_candidate(c, p, &saved)?;
    let (intent, _) = stored_intent(c, p, &saved)?.ok_or_else(Error::forbidden)?;
    intent.validate()?;
    let mut scopes = vec![
        AuthorityScope::identity(&owner, AuthorityMode::Read)?,
        AuthorityScope::conversation(&cid, AuthorityMode::Read)?,
        AuthorityScope::record(&p.record, AuthorityMode::Read)?,
    ];
    for connection in [
        p.selection
            .origin
            .request()
            .connection_id
            .as_deref()
            .ok_or_else(Error::storage)?,
        p.selection.origin.request().intent.retained(),
        string(&saved, "candidate_connection_id")?,
    ] {
        scopes.push(AuthorityScope::connection(connection, AuthorityMode::Read)?);
    }
    Ok((saved, AuthorityScope::merged(&scopes)?))
}
