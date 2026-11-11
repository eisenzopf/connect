//! Exact successful Data attribution does not change any native media grant.
use super::*;
use crate::{AuthorityMode, AuthorityScope};

fn scopes(c: &Connection, row: &rows::Saved, completed: bool) -> Result<Vec<AuthorityScope>> {
    if completed {
        return admission::row_scopes(row);
    }
    let cid = row.record.conversation_id.as_deref().ok_or_else(conflict)?;
    let mut scopes = crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Read)?;
    scopes.push(AuthorityScope::identity(&row.owner, AuthorityMode::Read)?);
    AuthorityScope::merged(&scopes)
}

fn positive(c: &Connection, row: &rows::Saved) -> Result<bool> {
    if row.record.state != OperationState::Preparing
        || row.record.cancel_requested
        || row.tombstoned
        || row.generation != 1
        || row.record.result.is_some()
        || row.record.problem.is_some()
    {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.closed=0 AND n.effect_started=1 AND e.provider='native-stream' AND e.correlation_key=n.operation_record_id AND e.idempotency_mode='none' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.observation_json IS NULL AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM session_codec_targets batch WHERE batch.native_operation_record_id=n.operation_record_id))", [&row.record.record_id], |r| r.get(0)).map_err(|_| Error::storage())
}

impl Store {
    /// None selects the existing Writer path BEFORE DML. In particular Unknown,
    /// cancellation and lifecycle reconciliation never use the positive permit.
    pub(super) fn complete_queued_native_stream(
        &self,
        receipt: &NativeStreamCommitted,
    ) -> Result<Option<OperationRecord>> {
        let CommittedOutcome::Queued { message_id } = &receipt.outcome else {
            return Ok(None);
        };
        let lease = &receipt.lease;
        if lease.command != Command::Data || lease.data_message.as_ref() != Some(message_id) {
            return Err(conflict());
        }
        let owner = self.require_runtime_owner()?;
        let result = serde_json::json!({"connection_id":lease.connection,"message_id":message_id,"core_queue_admission":true});
        let required = {
            let c = self.try_connection()?;
            let row = exact(&c, self, lease)?;
            let completed = row.record.state == OperationState::Completed;
            if !completed && !positive(&c, &row)? {
                return Ok(None);
            }
            scopes(&c, &row, completed)?
        };
        let authority = self.try_acquire_authority(&required)?;
        let mut c = self.try_connection()?;
        // Another owner of this SAME immutable receipt may have settled it
        // between the first SQL interval and this one. Pick the read-only
        // transaction from this actual locked connection, never fall through to
        // a Writer merely because the earlier observation was still Preparing.
        let completed = exact(&c, self, lease)?.record.state == OperationState::Completed;
        let tx = c
            .transaction_with_behavior(if completed {
                TransactionBehavior::Deferred
            } else {
                TransactionBehavior::Immediate
            })
            .map_err(|_| Error::storage())?;
        let row = exact(&tx, self, lease)?;
        if !self.authority_covers(&authority, &scopes(&tx, &row, completed)?) {
            return Ok(None);
        }
        if completed {
            let settled: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id JOIN native_data_messages d ON d.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.closed=1 AND e.provider='native-stream' AND e.correlation_key=n.operation_record_id AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 AND d.message_id=?2)", params![lease.record,message_id], |r| r.get(0)).map_err(|_|Error::storage())?;
            if row.record.state != OperationState::Completed
                || row.record.result.as_ref() != Some(&result)
                || !settled
            {
                return Ok(None);
            }
            if !owner.current() {
                return Err(Error::forbidden());
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(Some(row.record));
        }
        if !positive(&tx, &row)?
            || !crate::session_participant_lifecycle::reconcile_is_unchanged(
                &tx,
                self,
                row.record.session_id.as_deref().ok_or_else(conflict)?,
            )?
        {
            return Ok(None);
        }
        let permit = self
            .authority_writer_boundary
            .native_queued_capture_ledger(
                self,
                &tx,
                &authority,
                crate::authority_writer_boundary::phase_ledger::Key {
                    record: lease.record.clone(),
                    phase: "data-queued",
                    attempt: lease.attempt.clone(),
                    effect: lease.effect.clone(),
                    expected: serde_json::to_string(&result).map_err(|_| Error::storage())?,
                    runtime: lease.runtime.clone(),
                    reservation: lease.parent.clone(),
                    digest: row.digest.clone(),
                    stage: crate::authority_writer_boundary::phase_ledger::Stage::Confirm,
                },
            )?;
        let changed = tx
            .execute(
                "UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state='submitting'",
                [&lease.effect],
            )
            .map_err(|_| Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        rows::change(&tx, &row, OperationState::Committing, None, self)?;
        let row = rows::by_record(&tx, &lease.record)?;
        rows::change(&tx, &row, OperationState::Completed, Some(result), self)?;
        close(&tx, &row)?;
        workers::invalidate(&tx, &lease.record)?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &lease.record)?.record;
        drop(permit);
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(result))
    }
}
