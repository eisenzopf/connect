use super::*;
impl Store {
    /// Protected backend intent. Effect IDs and correlations are stable across uncertainty.
    pub fn prepare_transition_effect(
        &self,
        credential: &str,
        lease: &TransitionLease,
        id_: &str,
        provider: &str,
        correlation: &str,
        mode: IdempotencyMode,
        compensates: Option<&str>,
    ) -> Result<OperationEffect> {
        for v in [id_, provider, correlation].into_iter().chain(compensates) {
            id(v)?;
        }
        let authority = self.transition_lease_authority(credential, lease)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = workers::lease_authority(&tx, &actor, lease, self, true)?;
        admission::require_row(self, &authority, &row)?;
        if row.record.state != OperationState::Preparing
            && !(lease.cleanup && compensates.is_some())
        {
            return Err(conflict());
        }
        if let Some(old) = row.record.effects.iter().find(|e| e.id == id_) {
            if old.provider != provider
                || old.correlation_key != correlation
                || old.idempotency != mode
                || old.compensates_effect_id.as_deref() != compensates
            {
                return Err(conflict());
            }
            let result = old.clone();
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        // Reserve a separate bounded lane for compensation. Filling the normal
        // effect budget must not prevent cleanup of already confirmed creations.
        if row.record.effects.len() >= 64
            || (compensates.is_none()
                && row
                    .record
                    .effects
                    .iter()
                    .filter(|effect| effect.compensates_effect_id.is_none())
                    .count()
                    >= 32)
        {
            return Err(Error::new(
                "capacity-exceeded",
                "Operation effect capacity exceeded",
            ));
        }
        if let Some(target) = compensates {
            if !row.record.effects.iter().any(|e| {
                e.id == target
                    && e.provider == provider
                    && e.state == EffectState::Confirmed
                    && e.compensates_effect_id.is_none()
            }) || row.record.effects.iter().any(|e| {
                e.compensates_effect_id.as_deref() == Some(target)
                    && !matches!(e.state, EffectState::Failed | EffectState::Cancelled)
            }) {
                return Err(conflict());
            }
        }
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,compensates_effect_id) VALUES(?1,?2,?3,?4,?5,'prepared',?6)",params![id_,lease.record_id,provider,correlation,mode.as_str(),compensates]).map_err(|e|if matches!(e,rusqlite::Error::SqliteFailure(ref v,_) if v.code==rusqlite::ErrorCode::ConstraintViolation){conflict()}else{Error::storage()})?;
        touch(&tx, &lease.record_id, self)?;
        actor_from_connection(&tx, credential)?;
        let result = rows::by_record(&tx, &lease.record_id)?
            .record
            .effects
            .into_iter()
            .find(|e| e.id == id_)
            .ok_or_else(Error::storage)?;
        fault("effect-prepare:before-commit", &row.record.operation_id);
        tx.commit().map_err(|_| Error::storage())?;
        fault("effect-prepare:after-commit", &row.record.operation_id);
        Ok(result)
    }
    /// Commit submission intent before invoking a provider outside the transaction.
    /// A duplicate never authorizes a second call, even for provider-key adapters.
    pub fn begin_transition_effect(
        &self,
        credential: &str,
        lease: &TransitionLease,
        effect: &str,
    ) -> Result<OperationEffect> {
        let authority = self.transition_lease_authority(credential, lease)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = workers::lease_authority(&tx, &actor, lease, self, true)?;
        admission::require_row(self, &authority, &row)?;
        if row.record.state != OperationState::Preparing && !lease.cleanup {
            return Err(conflict());
        }
        let old = row
            .record
            .effects
            .iter()
            .find(|e| e.id == effect)
            .ok_or_else(Error::forbidden)?;
        if lease.cleanup && old.compensates_effect_id.is_none() {
            return Err(Error::forbidden());
        }
        if old.state != EffectState::Prepared || old.attempt != 0 {
            return Err(Error::new(
                "reconciliation-required",
                "Effect already attempted; do not submit again",
            ));
        }
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=?2 WHERE id=?1",params![effect,lease.generation]).map_err(|_|Error::storage())?;
        touch(&tx, &lease.record_id, self)?;
        actor_from_connection(&tx, credential)?;
        let result = rows::by_record(&tx, &lease.record_id)?
            .record
            .effects
            .into_iter()
            .find(|e| e.id == effect)
            .ok_or_else(Error::storage)?;
        fault("effect-submit:before-commit", &row.record.operation_id);
        tx.commit().map_err(|_| Error::storage())?;
        fault("effect-submit:after-commit", &row.record.operation_id);
        Ok(result)
    }
    /// Adapter calls this only after verifying the provider origin/correlation. A
    /// request body cannot select this capability. Recheck current owner authority
    /// before duplicates; retain evidence after worker loss without resubmitting.
    pub fn observe_transition_effect(
        &self,
        credential: &str,
        operation: &str,
        effect: &str,
        provider: &str,
        event: &str,
        submission_generation: u64,
        state: EffectState,
        provider_ref: Option<&str>,
    ) -> Result<OperationRecord> {
        for v in [operation, effect, provider, event]
            .into_iter()
            .chain(provider_ref)
        {
            id(v)?;
        }
        if matches!(state, EffectState::Prepared | EffectState::Submitting) {
            return Err(invalid());
        }
        let authority = self.transition_operation_authority(
            credential,
            operation,
            &["operation.observe"],
            None,
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("operation.observe")?;
        let tenant = authority::tenant(&tx, &actor)?;
        let row = rows::find(&tx, &tenant, &actor.identity_id, operation)?
            .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &authority, &row)?;
        if (row.initial_telephone || row.native_consultation || row.native_provider)
            || (row.native_return || row.native_handoff || row.native_replacement)
        {
            return Err(Error::forbidden());
        }
        let old = row
            .record
            .effects
            .iter()
            .find(|e| e.id == effect)
            .ok_or_else(Error::forbidden)?;
        if old.provider != provider
            || old.submission_generation != submission_generation
            || submission_generation == 0
            || old.attempt != 1
        {
            return Err(Error::forbidden());
        }
        let binding: Option<String> = tx
            .query_row(
                "SELECT provider_ref FROM operation_effects WHERE id=?1",
                [effect],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if binding
            .as_deref()
            .zip(provider_ref)
            .is_some_and(|(a, b)| a != b)
        {
            return Err(conflict());
        }
        let value = serde_json::json!({"record_id":row.record.record_id,"effect_id":effect,"submission_generation":submission_generation,"state":state,"provider_ref":provider_ref});
        let (hash, data) = digest(&value)?;
        if observation(
            &tx,
            &row,
            &format!("provider:{provider}"),
            event,
            &hash,
            &data,
            &actor,
            self,
        )? {
            let terminal = matches!(
                old.state,
                EffectState::Confirmed | EffectState::Failed | EffectState::Cancelled
            );
            if terminal && state != old.state {
                if state != EffectState::Unknown {
                    return Err(conflict());
                }
            } else if !row.record.state.terminal() || old.state == state {
                tx.execute("UPDATE operation_effects SET state=?2,provider_ref=COALESCE(provider_ref,?3),observation_json=?4 WHERE id=?1",params![effect,state.as_str(),provider_ref,data]).map_err(|_|Error::storage())?;
                touch(&tx, &row.record.record_id, self)?;
            } else {
                return Err(conflict());
            }
        }
        actor_from_connection(&tx, credential)?;
        let result = rows::by_record(&tx, &row.record.record_id)?.record;
        fault("effect-observe:before-commit", &row.record.operation_id);
        tx.commit().map_err(|_| Error::storage())?;
        fault("effect-observe:after-commit", &row.record.operation_id);
        Ok(result)
    }
    /// Protected attributed probe. Public reconciliation names only this evidence;
    /// it cannot manufacture provider delivery or local commit by supplying JSON.
    pub fn record_transition_evidence(
        &self,
        credential: &str,
        operation: &str,
        event: &str,
        source: EvidenceSource,
        outcome: ReconcileOutcome,
        completion: Option<TransitionCompletion>,
    ) -> Result<String> {
        id(operation)?;
        id(event)?;
        if outcome == ReconcileOutcome::Completed
            && (source != EvidenceSource::LocalState || completion.is_none())
        {
            return Err(invalid());
        }
        if outcome != ReconcileOutcome::Completed && completion.is_some() {
            return Err(invalid());
        }
        let authority = self.transition_operation_authority(
            credential,
            operation,
            &["operation.observe"],
            None,
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("operation.observe")?;
        let tenant = authority::tenant(&tx, &actor)?;
        let row = rows::find(&tx, &tenant, &actor.identity_id, operation)?
            .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &authority, &row)?;
        if !row.ledger
            || (row.initial_telephone || row.native_consultation || row.native_provider)
            || (row.native_return || row.native_handoff || row.native_replacement)
            || row.tombstoned
        {
            return Err(Error::forbidden());
        }
        let value = serde_json::json!({"record_id":row.record.record_id,"source_kind":source.as_str(),"outcome":outcome,"completion":completion});
        let (hash, data) = digest(&value)?;
        let source_ = format!("reconciliation:{}", source.as_str());
        let old:Option<(String,String)>=tx.query_row("SELECT id,digest FROM operation_observations WHERE source=?1 AND source_event_id=?2",params![source_,event],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        if let Some((id_, digest)) = old {
            if digest != hash {
                return Err(conflict());
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(id_);
        }
        if let Some(completion) = &completion {
            workers::completion_scope(&tx, &row, completion, false, self)?;
        }
        let count: u64 = tx
            .query_row(
                "SELECT COUNT(*) FROM operation_evidence WHERE operation_record_id=?1",
                [&row.record.record_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if count >= 32 {
            return Err(Error::new(
                "capacity-exceeded",
                "Reconciliation evidence capacity exceeded",
            ));
        }
        let id_ = format!("obs_{}", uuid::Uuid::new_v4());
        tx.execute(
            "INSERT INTO operation_observations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id_,
                row.record.record_id,
                source_,
                event,
                hash,
                actor.identity_id,
                data,
                crate::now()
            ],
        )
        .map_err(|_| Error::storage())?;
        tx.execute(
            "INSERT INTO operation_evidence VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                id_,
                row.record.record_id,
                row.record.revision,
                row.generation,
                outcome.state().as_str(),
                source.as_str(),
                completion
                    .map(|v| serde_json::to_string(&v).map_err(|_| Error::storage()))
                    .transpose()?
            ],
        )
        .map_err(|_| Error::storage())?;
        actor_from_connection(&tx, credential)?;
        fault("evidence:before-commit", operation);
        tx.commit().map_err(|_| Error::storage())?;
        fault("evidence:after-commit", operation);
        Ok(id_)
    }
}
pub(super) fn touch(c: &Connection, record: &str, store: &Store) -> Result<()> {
    let changed=c.execute("UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1 AND revision<9007199254740991",params![record,crate::now()]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(conflict());
    }
    rows::event(c, record, "operation_observed", store)
}
fn observation(
    c: &Connection,
    row: &rows::Saved,
    source: &str,
    event: &str,
    hash: &str,
    data: &str,
    actor: &Actor,
    _store: &Store,
) -> Result<bool> {
    let old: Option<String> = c
        .query_row(
            "SELECT digest FROM operation_observations WHERE source=?1 AND source_event_id=?2",
            params![source, event],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if let Some(old) = old {
        return if old == hash {
            Ok(false)
        } else {
            Err(conflict())
        };
    }
    let count:u64=c.query_row("SELECT COUNT(*) FROM operation_observations WHERE operation_record_id=?1 AND source LIKE 'provider:%'",[&row.record.record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if count >= 256 {
        return Err(Error::new(
            "capacity-exceeded",
            "Provider observation capacity exceeded",
        ));
    }
    c.execute(
        "INSERT INTO operation_observations VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            format!("obs_{}", uuid::Uuid::new_v4()),
            row.record.record_id,
            source,
            event,
            hash,
            actor.identity_id,
            data,
            crate::now()
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(true)
}
