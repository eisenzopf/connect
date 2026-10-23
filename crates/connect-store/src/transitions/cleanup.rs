//! Explicit backend repair for an abandoned owner's effect; never a model verb.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CleanupSettlement {
    /// A verified provider probe establishes that an unconfirmed effect is absent.
    Absent {
        effect_id: String,
        provider: String,
        submission_generation: u64,
    },
    /// An attributed probe establishes cleanup of an already confirmed creation.
    /// The creation fact remains confirmed; this records a separate compensation.
    Compensated {
        effect_id: String,
        provider: String,
        submission_generation: u64,
    },
}
impl CleanupSettlement {
    fn fields(&self) -> (&str, &str, u64, bool) {
        match self {
            Self::Absent {
                effect_id,
                provider,
                submission_generation,
            } => (effect_id, provider, *submission_generation, false),
            Self::Compensated {
                effect_id,
                provider,
                submission_generation,
            } => (effect_id, provider, *submission_generation, true),
        }
    }
}
impl Store {
    /// Protected inspection for a current operational backend. This does not grant
    /// conversation access; both explicit cleanup and current source-read scope apply.
    pub fn inspect_transition_for_cleanup(
        &self,
        credential: &str,
        record: &str,
    ) -> Result<OperationRecord> {
        id(record)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = rows::by_record(&tx, record)?;
        authorize(&tx, &actor, &row)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation result has expired"));
        }
        actor_from_connection(&tx, credential)?;
        Ok(row.record)
    }
    /// Trusted composition after verified provider/local probes, not user-supplied
    /// attestation or a request body. No provider call or route commit occurs here.
    /// The repair has its own stable decision ID and attributed causal journal event.
    pub fn settle_abandoned_transition(
        &self,
        credential: &str,
        record: &str,
        decision: &str,
        event: &str,
        source: EvidenceSource,
        settlements: &[CleanupSettlement],
        outcome: ReconcileOutcome,
    ) -> Result<OperationRecord> {
        for value in [record, decision, event] {
            id(value)?;
        }
        if outcome == ReconcileOutcome::Completed
            || source == EvidenceSource::OperatorAttestation
            || settlements.len() > 32
        {
            return Err(invalid());
        }
        let mut settlements = settlements.to_vec();
        settlements.sort_by(|a, b| a.fields().0.cmp(b.fields().0));
        if settlements
            .windows(2)
            .any(|v| v[0].fields().0 == v[1].fields().0)
        {
            return Err(invalid());
        }
        let authority = self.transition_cleanup_authority(credential, record)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = rows::by_record(&tx, record)?;
        authorize(&tx, &actor, &row)?;
        admission::require_row(self, &authority, &row)?;
        if !self.authority_covers(&authority, &admission::scopes(&actor.identity_id, None)?) {
            return Err(Error::forbidden());
        }
        if row.tombstoned {
            return Err(Error::new("gone", "Operation full result has expired"));
        }
        if decision == row.record.operation_id && actor.identity_id == row.owner {
            return Err(invalid());
        }
        crate::operations::reject_control_id(&tx, &actor, decision)?;
        let value = serde_json::json!({"target_record_id":record,"source":source.as_str(),"event_id":event,"settlements":settlements,"outcome":outcome});
        let (hash, intent) = digest(&value)?;
        if let Some(old) = rows::find(&tx, &row.tenant, &actor.identity_id, decision)? {
            authority::read(&tx, &actor, &old)?;
            if old.digest != hash || old.record.command_type != "operation.cleanup" {
                return Err(conflict());
            }
            admission::require_row(self, &authority, &old)?;
            let result = rows::view(&tx, &actor, old)?;
            actor_from_connection(&tx, credential)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        if row.record.state.terminal() {
            return Err(conflict());
        }
        let mut seen = std::collections::HashSet::new();
        for settlement in &settlements {
            let (effect, provider, generation, compensated) = settlement.fields();
            id(effect)?;
            id(provider)?;
            if !seen.insert(effect) {
                return Err(invalid());
            }
            let old = row
                .record
                .effects
                .iter()
                .find(|e| e.id == effect)
                .ok_or_else(Error::forbidden)?;
            if old.provider != provider
                || old.submission_generation != generation
                || old.compensates_effect_id.is_some()
            {
                return Err(Error::forbidden());
            }
            if compensated {
                if old.state != EffectState::Confirmed {
                    return Err(conflict());
                }
                let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE compensates_effect_id=?1 AND state IN ('prepared','submitting','unknown'))",[effect],|r|r.get(0)).map_err(|_|Error::storage())?;
                if pending {
                    return Err(Error::new(
                        "reconciliation-required",
                        "An existing compensation must be reconciled first",
                    ));
                }
                let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE compensates_effect_id=?1 AND state='confirmed')",[effect],|r|r.get(0)).map_err(|_|Error::storage())?;
                if !already {
                    let total: u64 = tx
                        .query_row(
                            "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
                            [record],
                            |r| r.get(0),
                        )
                        .map_err(|_| Error::storage())?;
                    if total >= 64 {
                        return Err(Error::new(
                            "capacity-exceeded",
                            "Repair effect capacity exceeded",
                        ));
                    }
                    let key = hex::encode(Sha256::digest(
                        format!("{record}:{effect}:{event}").as_bytes(),
                    ));
                    let id_ = format!("cleanup_{key}");
                    tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,observation_json,compensates_effect_id) VALUES(?1,?2,?3,?4,'status-query','confirmed',0,?5,?6)",params![id_,record,provider,key,intent,effect]).map_err(|_|Error::storage())?;
                }
            } else {
                if old.state == EffectState::Confirmed {
                    return Err(conflict());
                }
                if !matches!(old.state, EffectState::Failed | EffectState::Cancelled) {
                    tx.execute("UPDATE operation_effects SET state='failed',observation_json=?2 WHERE id=?1",params![effect,intent]).map_err(|_|Error::storage())?;
                }
            }
        }
        // A partial probe cannot acknowledge settlement of an effect it did not prove.
        if rows::unsettled(&tx, record)? {
            return Err(Error::new(
                "reconciliation-required",
                "Other external effects remain unresolved; no repair accepted",
            ));
        }
        capacity_after_terminal(&tx, self, &row)?;
        let observation = format!("obs_{}", uuid::Uuid::new_v4());
        let old: Option<String> = tx
            .query_row(
                "SELECT digest FROM operation_observations WHERE source=?1 AND source_event_id=?2",
                params![format!("cleanup:{}", source.as_str()), event],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if old.is_some() {
            return Err(conflict());
        }
        tx.execute(
            "INSERT INTO operation_observations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                observation,
                record,
                format!("cleanup:{}", source.as_str()),
                event,
                hash,
                actor.identity_id,
                intent,
                crate::now()
            ],
        )
        .map_err(|_| Error::storage())?;
        let next = outcome.state();
        if !row.record.state.permits(next) {
            return Err(conflict());
        }
        tx.execute("UPDATE operation_effects SET state='cancelled' WHERE operation_record_id=?1 AND state='prepared'",[record]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operations SET state=?2,revision=revision+1,updated_at=?3,cancel_requested=1 WHERE record_id=?1",params![record,next.as_str(),crate::now()]).map_err(|_|Error::storage())?;
        workers::invalidate(&tx, record)?;
        let decision_record = format!("opr_{}", uuid::Uuid::new_v4());
        let result = serde_json::json!({"target_record_id":record,"state_at_decision":next,"disposition":"verified_cleanup","evidence_id":observation});
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,intent_json,state,result_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'operation.cleanup',?6,?7,?8,'completed',?9,?10,?10)",params![decision_record,row.tenant,actor.identity_id,decision,hash,row.record.conversation_id,row.record.session_id,intent,result.to_string(),crate::now()]).map_err(|_|Error::storage())?;
        tx.execute(
            "INSERT INTO operation_decisions VALUES(?1,?2,'cleanup')",
            params![decision_record, record],
        )
        .map_err(|_| Error::storage())?;
        rows::event(&tx, &decision_record, "operation_cleanup", self)?;
        actor_from_connection(&tx, credential)?;
        let result = rows::by_record(&tx, &decision_record)?.record;
        fault("cleanup-decision:before-commit", &row.record.operation_id);
        tx.commit().map_err(|_| Error::storage())?;
        fault("cleanup-decision:after-commit", &row.record.operation_id);
        Ok(result)
    }
}
fn capacity_after_terminal(c: &Connection, store: &Store, row: &rows::Saved) -> Result<()> {
    // The target's one receipt reserve will be released by the same transaction.
    let used: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM operations)+(SELECT COUNT(*) FROM task_requests)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !row.ledger || !crate::tasks::budget::receipts(c, store.limits.operation_receipts, used, 0)?
    {
        return Err(Error::new(
            "capacity-exceeded",
            "Cleanup decision capacity exceeded",
        ));
    }
    Ok(())
}
pub(super) fn authorize(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    actor.require("operation.cleanup")?;
    actor.require("session_get")?;
    if !row.ledger
        || row.initial_telephone
        || (row.native_consultation || row.native_provider)
        || (row.native_return || row.native_handoff || row.native_replacement)
        || authority::tenant(c, actor)? != row.tenant
    {
        return Err(Error::forbidden());
    }
    crate::audience::require(
        c,
        actor,
        row.record
            .conversation_id
            .as_deref()
            .ok_or_else(Error::storage)?,
        connect_protocol::membership::ContentKind::Session,
        row.record
            .session_id
            .as_deref()
            .ok_or_else(Error::storage)?,
    )?;
    let mut st = c
        .prepare("SELECT identity_id FROM operation_members WHERE operation_record_id=?1")
        .map_err(|_| Error::storage())?;
    for value in st
        .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
    {
        if !actor.permits_recipient(&value.map_err(|_| Error::storage())?) {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
