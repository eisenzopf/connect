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
    /// the operation preparing/unknown until exact candidate cleanup is observed.
    /// A still-owned negative capture can resolve generic recovery uncertainty;
    /// an uncertain physical-result capture cannot be converted into this type.
    pub fn record_native_replacement_commit_not_submitted(
        &self,
        captured: &NativeReplacementCommitNotSubmitted,
    ) -> Result<OperationRecord> {
        let a = &captured.attempt;
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let (held, mut c) = self.try_replacement_capture_projection(None, |c| {
            let mut required = admission::row_scopes(&rows::by_record(c, &p.record)?)?;
            required.push(crate::AuthorityScope::record(
                &p.record,
                crate::AuthorityMode::Write,
            )?);
            crate::AuthorityScope::merged(&required)
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
            || !matches!(state.as_str(), "submitting" | "unknown")
            || !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            )
        {
            return Err(conflict());
        }
        let recorded_at = crate::now();
        // This exact owned negative proves Core was never invoked. Recording
        // it changes only these three historical ledger rows, not media grants.
        let permit = self
            .authority_writer_boundary
            .native_commit_not_submitted_ledger(self, &tx, &held, captured, recorded_at)?;
        #[cfg(test)]
        observe_scoped_not_submitted(&tx);
        tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",
            params![p.record,NOT_SUBMITTED]).map_err(|_| Error::storage())?;
        tx.execute(
            "UPDATE operation_effects SET state='failed' WHERE id=?1 AND state IN ('submitting','unknown')",
            [&a.effect],
        )
        .map_err(|_| Error::storage())?;
        native_handoffs::touch_at(&tx, &p.record, self, recorded_at)?;
        let record = rows::by_record(&tx, &p.record)?.record;
        drop(permit);
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

impl NativeReplacementCommitNotSubmitted {
    // Crate-private semantic constructor for a closed three-row delta. No
    // caller supplies states, row IDs, a cleanup claim or a synthetic receipt.
    pub(crate) fn scoped_metadata_mutation(
        &self,
        store: &Store,
        c: &Connection,
        recorded_at: i64,
    ) -> Result<crate::authority_writer_boundary::phase_ledger::scoped::NotSubmittedMutation> {
        use crate::authority_writer_boundary::phase_ledger::{
            scoped::{self, Row, Table, Transition},
            Key, Stage,
        };
        use rusqlite::types::Value;
        let a = &self.attempt;
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(store, &p.owner, &p.runtime)?;
        let exact: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacements n
             JOIN operations o ON o.record_id=n.operation_record_id
             JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit'
             JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id
             WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4
             AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json IS NULL
             AND e.observation_json=?7 AND e.provider='native-replacement-commit' AND e.correlation_key=e.id
             AND e.idempotency_mode='none' AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL
             AND e.state IN ('submitting','unknown') AND e.attempt=1 AND e.submission_generation=1
             AND o.command_type='connection.replace' AND o.state IN ('preparing','unknown') AND o.tombstoned=0)",
            params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,a.expected], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let original = rows::by_record(c, &p.record)?;
        let mut required = admission::row_scopes(&original)?;
        required.push(crate::AuthorityScope::record(
            &p.record,
            crate::AuthorityMode::Write,
        )?);
        let required = crate::AuthorityScope::merged(&required)?;
        let row = |table, keys: &[Value]| -> Result<Row> {
            scoped::row_snapshot(c, table, keys)?.ok_or_else(Error::forbidden)
        };
        let old_step = row(
            Table::ReplacementSteps,
            &[Value::Text(p.record.clone()), Value::Text("commit".into())],
        )?;
        let mut new_step = old_step.clone();
        new_step.set("observation_json", Value::Text(NOT_SUBMITTED.into()))?;
        let old_effect = row(Table::Effects, &[Value::Text(a.effect.clone())])?;
        let mut new_effect = old_effect.clone();
        new_effect.set("state", Value::Text("failed".into()))?;
        let old_operation = row(Table::Operations, &[Value::Text(p.record.clone())])?;
        let mut new_operation = old_operation.clone();
        let Value::Integer(revision) = old_operation.get("revision")? else {
            return Err(Error::forbidden());
        };
        if *revision >= 9_007_199_254_740_990 {
            return Err(Error::forbidden());
        }
        new_operation.set(
            "revision",
            Value::Integer(revision.checked_add(1).ok_or_else(Error::forbidden)?),
        )?;
        new_operation.set("updated_at", Value::Integer(recorded_at))?;
        Ok(scoped::NotSubmittedMutation {
            key: Key {
                record: p.record.clone(),
                phase: "scoped-commit-not-submitted",
                attempt: a.attempt.clone(),
                effect: a.effect.clone(),
                expected: NOT_SUBMITTED.into(),
                runtime: p.runtime.clone(),
                reservation: p.reservation.clone(),
                digest: p.hash.clone(),
                stage: Stage::Confirm,
            },
            required,
            transitions: vec![
                Transition::new(c, Some(&old_step), Some(&new_step))?,
                Transition::new(c, Some(&old_effect), Some(&new_effect))?,
                Transition::new(c, Some(&old_operation), Some(&new_operation))?,
            ],
        })
    }
}

#[cfg(test)]
thread_local! {
    static NOT_SUBMITTED_PROBE: std::cell::RefCell<Option<Box<dyn FnOnce(&Connection)>>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
fn observe_scoped_not_submitted(c: &Connection) {
    if let Some(probe) = NOT_SUBMITTED_PROBE.with(|slot| slot.borrow_mut().take()) {
        probe(c);
    }
}
#[cfg(test)]
impl Store {
    pub(crate) fn with_native_replacement_not_submitted_probe_for_test<T>(
        probe: impl FnOnce(&Connection) + 'static,
        call: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                NOT_SUBMITTED_PROBE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        NOT_SUBMITTED_PROBE.with(|slot| {
            assert!(
                slot.borrow().is_none(),
                "nested original-not-submitted ledger probe"
            );
            *slot.borrow_mut() = Some(Box::new(probe));
        });
        let _reset = Reset;
        call()
    }
}

/// Private immutable lineage minted only from the still-owned negative token.
/// No Deserialize/public constructor/current permission or native-close grant.
pub(in crate::transitions::native_replacements) struct NativeReplacementNotSubmittedLineage {
    original: std::sync::Weak<()>,
    attempt: String,
    effect: String,
    expected: String,
}
impl NativeReplacementCommitNotSubmitted {
    pub(in crate::transitions::native_replacements) fn unpublished_lineage(
        &self,
        original: &NativeReplacementPrepared,
    ) -> Result<NativeReplacementNotSubmittedLineage> {
        let p = &self
            .attempt
            .origin
            .started
            .attempt
            .allocated
            .attempt
            .prepared;
        if !p.same_capture(original) {
            return Err(Error::forbidden());
        }
        Ok(NativeReplacementNotSubmittedLineage {
            original: Arc::downgrade(&p.capture_identity),
            attempt: self.attempt.attempt.clone(),
            effect: self.attempt.effect.clone(),
            expected: self.attempt.expected.clone(),
        })
    }
}
impl NativeReplacementNotSubmittedLineage {
    pub(in crate::transitions::native_replacements) fn require_durable(
        &self,
        c: &Connection,
        p: &NativeReplacementPrepared,
    ) -> Result<()> {
        if !self
            .original
            .upgrade()
            .is_some_and(|original| Arc::ptr_eq(&original, &p.capture_identity))
        {
            return Err(Error::forbidden());
        }
        // A failed effect alone, a persisted started marker or generic Unknown
        // is insufficient. This is the exact SAME captured pre-Core negative.
        let exact:bool=c.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacements n
             JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit'
             JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id
             WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4
             AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7
             AND e.state='failed' AND e.observation_json=?8 AND e.provider='native-replacement-commit'
             AND e.correlation_key=e.id AND e.attempt=1 AND e.submission_generation=1
             AND NOT EXISTS(SELECT 1 FROM operation_effects other WHERE other.operation_record_id=n.operation_record_id
                 AND other.provider='native-replacement-commit' AND other.id!=e.id))",
            params![p.record,p.runtime,p.reservation,p.hash,self.attempt,self.effect,NOT_SUBMITTED,self.expected],
            |row|row.get(0),
        ).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
