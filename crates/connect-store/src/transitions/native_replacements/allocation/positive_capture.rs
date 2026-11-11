//! Positive native allocation/Start metadata. No live transport or new grant.
use super::*;
use crate::{authority_writer_boundary::phase_ledger, AuthorityGuardSet, AuthorityWait};

pub(super) struct PositiveResourceCapture<'a> {
    pub(super) prepared: &'a NativeReplacementPrepared,
    pub(super) phase: &'static str,
    pub(super) attempt: &'a str,
    pub(super) effect: &'a str,
    pub(super) target: &'a str,
    pub(super) observation: &'a str,
}

#[derive(Clone, Copy)]
enum CaptureState {
    Recorded,
    Submitting,
}

impl PositiveResourceCapture<'_> {
    fn state(&self, c: &Connection) -> Result<Option<CaptureState>> {
        let prepared = self.prepared;
        let exact: Option<(Option<String>, String, bool)> = c.query_row(
            "SELECT s.observation_json,e.state,(o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND o.lease_owner IS n.reservation_id AND o.lease_expires_at IS n.expires_at AND NOT EXISTS(SELECT 1 FROM native_replacement_steps later WHERE later.operation_record_id=s.operation_record_id AND (later.phase='commit' OR (s.phase='allocate' AND later.phase='start')))) FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE n.operation_record_id=?1 AND o.command_type='connection.replace' AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.phase=?5 AND s.attempt_id=?6 AND s.effect_id=?7 AND s.started=1 AND e.provider='native-replacement-'||s.phase AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.observation_json=?8 AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL",
            params![prepared.record,prepared.runtime,prepared.reservation,prepared.hash,self.phase,self.attempt,self.effect,self.target],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|_| Error::storage())?;
        Ok(match exact {
            Some((Some(old), state, _)) if old == self.observation && state == "confirmed" => {
                Some(CaptureState::Recorded)
            }
            Some((None, state, true)) if state == "submitting" => Some(CaptureState::Submitting),
            _ => None,
        })
    }
}

impl Store {
    // A known replay only observes; a first immutable positive capture can
    // confirm under exact Read while the root is Preparing with no later cut.
    // Every other outcome goes through the original mutation path.
    pub(super) fn try_record_positive_native_resource(
        &self,
        capture: PositiveResourceCapture<'_>,
        interval: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<Option<OperationRecord>> {
        let prepared = capture.prepared;
        let (mut c, held, initial) = if let Some(held) = interval {
            let c = self.try_connection()?;
            let Some(initial) = capture.state(&c)? else {
                return Ok(None);
            };
            // The caller already owns the actual signed scope. Reuse its
            // original single primary SQL admission through the transaction.
            (c, held.clone(), initial)
        } else {
            let (initial, scopes) = {
                let c = self.try_connection()?;
                let Some(initial) = capture.state(&c)? else {
                    return Ok(None);
                };
                (
                    initial,
                    admission::row_scopes(&rows::by_record(&c, &prepared.record)?)?,
                )
            };
            let held = self.acquire_authority(
                &scopes,
                &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            )?;
            (self.try_connection()?, held, initial)
        };
        let tx = c
            .transaction_with_behavior(match initial {
                CaptureState::Recorded => TransactionBehavior::Deferred,
                CaptureState::Submitting => TransactionBehavior::Immediate,
            })
            .map_err(|_| Error::storage())?;
        local_owner(self, &prepared.owner, &prepared.runtime)?;
        let row = rows::by_record(&tx, &prepared.record)?;
        admission::require_row(self, &held, &row)?;
        match capture.state(&tx)? {
            Some(CaptureState::Recorded) => return Ok(Some(row.record)),
            Some(CaptureState::Submitting) if matches!(initial, CaptureState::Submitting) => {}
            _ => return Ok(None),
        }
        let permit = self
            .authority_writer_boundary
            .native_resource_capture_ledger(
                self,
                &tx,
                &held,
                phase_ledger::Key {
                    record: prepared.record.clone(),
                    phase: capture.phase,
                    attempt: capture.attempt.into(),
                    effect: capture.effect.into(),
                    expected: capture.observation.into(),
                    runtime: prepared.runtime.clone(),
                    reservation: prepared.reservation.clone(),
                    digest: prepared.hash.clone(),
                    stage: phase_ledger::Stage::Confirm,
                },
                Some(capture.target),
            )?;
        let observed = tx.execute(
            "UPDATE native_replacement_steps SET observation_json=?5 WHERE operation_record_id=?1 AND phase=?2 AND attempt_id=?3 AND effect_id=?4 AND started=1 AND observation_json IS NULL",
            params![prepared.record,capture.phase,capture.attempt,capture.effect,capture.observation],
        ).map_err(|_| Error::storage())?;
        let confirmed = tx.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND operation_record_id=?2 AND state='submitting' AND attempt=1 AND submission_generation=1",
            params![capture.effect,prepared.record],
        ).map_err(|_| Error::storage())?;
        if observed != 1 || confirmed != 1 {
            return Err(conflict());
        }
        native_handoffs::touch(&tx, &prepared.record, self)?;
        let result = rows::by_record(&tx, &prepared.record)?.record;
        drop(permit);
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(result))
    }
}
