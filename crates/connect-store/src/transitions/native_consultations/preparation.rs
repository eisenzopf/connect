//! Recording-only abandonment of an exact preparation that never began IO.
use super::*;

pub struct NativeConsultationPreparation {
    owner: Weak<crate::InstallationOwnership>,
    runtime: String,
    record: String,
    attempt: String,
    effect: String,
    hash: String,
}
impl std::fmt::Debug for NativeConsultationPreparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationPreparation(redacted)")
    }
}
pub(super) fn capture(lease: &NativeConsultationLease) -> NativeConsultationPreparation {
    let origin = &lease.selection.origin;
    NativeConsultationPreparation {
        owner: origin.owner.clone(),
        runtime: origin.runtime.clone(),
        record: origin.record.clone(),
        attempt: lease.attempt.clone(),
        effect: lease.effect.clone(),
        hash: lease.hash.clone(),
    }
}
impl Store {
    /// Settle an exact no-IO preparation even after its execution grant expires
    /// or is revoked. The durable started bit always wins over this old receipt.
    /// A provider that may still exist leaves the root unknown independently.
    pub fn settle_native_consultation_preparation(
        &self,
        proof: &NativeConsultationPreparation,
    ) -> Result<bool> {
        let owner = self.require_runtime_owner()?;
        if proof.runtime != self.runtime_epoch()
            || !owner.current()
            || !proof
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &owner))
        {
            return Err(Error::forbidden());
        }
        // Changing a floor changes packet authority. Drain current readers;
        // ordinary metadata's read-only footprint is insufficient here.
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &proof.record)?;
                admission::scopes(&row.owner, row.record.conversation_id.as_deref())
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &proof.record)?;
        if !self.authority_covers(
            &held,
            &admission::scopes(&row.owner, row.record.conversation_id.as_deref())?,
        ) {
            return Err(Error::forbidden());
        }
        let saved:Option<(bool,bool,String,u64)>=tx.query_row("SELECT n.started,n.applied_json IS NOT NULL,e.state,e.attempt FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.effect_id=?3 AND n.runtime_epoch=?4 AND n.plan_digest=?5",params![proof.record,proof.attempt,proof.effect,proof.runtime,proof.hash],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let (started, applied, state, attempt) = saved.ok_or_else(Error::forbidden)?;
        if started || applied {
            return Ok(false);
        }
        if row.record.state.terminal() {
            return Ok(false);
        }
        if attempt != 0 || !matches!(state.as_str(), "prepared" | "cancelled") {
            return Err(Error::forbidden());
        }
        if state == "cancelled" {
            return Ok(false);
        }
        tx.execute("UPDATE operation_effects SET state='cancelled' WHERE id=?1 AND state='prepared' AND attempt=0",[&proof.effect]).map_err(|_|Error::storage())?;
        let next = if rows::unsettled(&tx, &proof.record)? {
            OperationState::Unknown
        } else {
            OperationState::Failed
        };
        tx.execute("UPDATE operations SET state=?2,problem_json=COALESCE(problem_json,?3) WHERE record_id=?1",params![proof.record,next.as_str(),serde_json::json!({"code":"native-consultation-preflight-stopped","source":"native-consultation"}).to_string()]).map_err(|_|Error::storage())?;
        touch(&tx, &proof.record, self)?;
        workers::invalidate(&tx, &proof.record)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}

/// Called in the actual provider-termination observation transaction. It can
/// settle only a previously abandoned no-IO native preparation, never a possible
/// audience submission or an unconfirmed provider shutdown.
pub(crate) fn settle_after_provider_closed(c: &Connection, root: &str) -> Result<()> {
    let safe:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1 AND n.started=0 AND n.applied_json IS NULL AND e.state='cancelled' AND e.attempt=0)",[root],|r|r.get(0)).map_err(|_|Error::storage())?;
    if safe && !rows::unsettled(c, root)? {
        c.execute(
            "UPDATE operations SET state='failed' WHERE record_id=?1 AND state='unknown'",
            [root],
        )
        .map_err(|_| Error::storage())?;
    }
    Ok(())
}
