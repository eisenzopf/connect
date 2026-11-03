//! An owned pre-Core refusal is different from an unknown physical outcome.
use super::*;

pub struct NativeReplacementCommitNotSubmitted {
    attempt: NativeReplacementCommitAttempt,
}
redacted!(NativeReplacementCommitNotSubmitted);
pub(super) const NOT_SUBMITTED: &str = r#"{"submission":"not-started"}"#;

impl NativeReplacementCommitAttempt {
    /// Consume the live reservation only when Core has never been invoked.
    /// This cannot be reconstructed from a timeout, an uncertain Core return,
    /// persisted started marker or successor-runtime absence.
    pub fn not_submitted(self) -> NativeReplacementCommitNotSubmitted {
        NativeReplacementCommitNotSubmitted { attempt: self }
    }
}
impl Store {
    /// Persist a definitive pre-Core refusal under the original attempt. Keep
    /// the operation preparing until exact candidate cleanup is observed.
    pub fn record_native_replacement_commit_not_submitted(
        &self,
        captured: &NativeReplacementCommitNotSubmitted,
    ) -> Result<OperationRecord> {
        let a = &captured.attempt;
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let (held, mut c) = self.try_replacement_capture_projection(None, |c| {
            admission::row_scopes(&rows::by_record(c, &p.record)?)
        })?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let saved: Option<(Option<String>, String)> = tx.query_row(
            "SELECT s.observation_json,e.state FROM native_replacements n
             JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit'
             JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=n.operation_record_id
             WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4
             AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND e.observation_json=?7
             AND e.provider='native-replacement-commit' AND e.correlation_key=e.id
             AND e.attempt=1 AND e.submission_generation=1",
            params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,a.expected],
            |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(|_| Error::storage())?;
        let (observed, state) = saved.ok_or_else(Error::forbidden)?;
        if observed.as_deref() == Some(NOT_SUBMITTED) && state == "failed" {
            return Ok(row.record);
        }
        if observed.is_some()
            || state != "submitting"
            || !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            )
        {
            return Err(conflict());
        }
        tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",
            params![p.record,NOT_SUBMITTED]).map_err(|_| Error::storage())?;
        tx.execute(
            "UPDATE operation_effects SET state='failed' WHERE id=?1 AND state='submitting'",
            [&a.effect],
        )
        .map_err(|_| Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        let record = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}

/// A failed effect alone is insufficient: only our exact durable pre-Core
/// observation permits the existing started-candidate cleanup path.
pub(in super::super) fn permits_candidate_cleanup(c: &Connection, record: &str) -> Result<bool> {
    c.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM native_replacement_steps s
         WHERE s.operation_record_id=?1 AND s.phase='commit' AND s.started=1
         AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.id=s.effect_id
          AND e.operation_record_id=s.operation_record_id AND e.provider='native-replacement-commit'
          AND e.correlation_key=e.id AND e.attempt=1 AND e.submission_generation=1
          AND e.state='failed' AND s.observation_json=?2))",
        params![record, NOT_SUBMITTED],
        |r| r.get(0),
    )
    .map_err(|_| Error::storage())
}
