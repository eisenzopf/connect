use super::*;
impl Store {
    /// Trusted worker composition only. Claims never submit a remote effect.
    pub fn claim_transition(
        &self,
        credential: &str,
        operation: &str,
        ttl: u32,
    ) -> Result<Option<(TransitionLease, OperationRecord)>> {
        if ttl == 0 || ttl > MAX_WORKER_LEASE {
            return Err(invalid());
        }
        id(operation)?;
        let authority = self.transition_operation_authority(
            credential,
            operation,
            &["operation.execute"],
            None,
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("operation.execute")?;
        let tenant = authority::tenant(&tx, &actor)?;
        let mut row = rows::find(&tx, &tenant, &actor.identity_id, operation)?
            .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &authority, &row)?;
        recover_one(&tx, &row, &self.transition_runtime, self)?;
        row = rows::by_record(&tx, &row.record.record_id)?;
        if !row.ledger
            || (row.initial_telephone || row.native_consultation || row.native_provider) || (row.native_return || row.native_handoff || row.native_replacement || admission::candidate_row(&row)?)
            || tx.query_row("SELECT EXISTS(SELECT 1 FROM consultation_candidate_bindings WHERE operation_record_id=?1)",[&row.record.record_id],|r|r.get::<_,bool>(0)).map_err(|_|Error::storage())?
            || !matches!(
                row.record.state,
                OperationState::Accepted | OperationState::Interrupted
            )
            || rows::unsettled(&tx, &row.record.record_id)?
        {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        authority::execution(&tx, &actor, &row, self)?;
        let reservations = reservations(&tx, &row)?;
        if row.generation >= 9007199254740990 {
            return Err(conflict());
        }
        let worker = uuid::Uuid::new_v4().to_string();
        let generation = row.generation + 1;
        tx.execute("UPDATE operations SET lease_owner=?2,lease_expires_at=?3,worker_generation=?4,worker_runtime=?5,worker_cleanup=0 WHERE record_id=?1",params![row.record.record_id,worker,(crate::now()+i64::from(ttl)).min(row.record.expires_at.ok_or_else(Error::storage)?),generation,self.transition_runtime]).map_err(|_|Error::storage())?;
        rows::change(&tx, &row, OperationState::Preparing, None, self)?;
        actor_from_connection(&tx, credential)?;
        let record = rows::by_record(&tx, &row.record.record_id)?.record;
        let lease = TransitionLease {
            record_id: row.record.record_id,
            worker,
            generation,
            runtime: self.transition_runtime.clone(),
            cleanup: false,
            reservations,
        };
        fault("claim:before-commit", operation);
        tx.commit().map_err(|_| Error::storage())?;
        fault("claim:after-commit", operation);
        Ok(Some((lease, record)))
    }
    pub fn renew_transition_lease(
        &self,
        credential: &str,
        lease: &TransitionLease,
        ttl: u32,
    ) -> Result<()> {
        if ttl == 0 || ttl > MAX_WORKER_LEASE {
            return Err(invalid());
        }
        let authority = self.transition_lease_authority(credential, lease)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = lease_authority(&tx, &actor, lease, self, true)?;
        admission::require_row(self, &authority, &row)?;
        tx.execute(
            "UPDATE operations SET lease_expires_at=?2 WHERE record_id=?1",
            params![
                lease.record_id,
                (crate::now() + i64::from(ttl)).min(if lease.cleanup {
                    actor.expires_at
                } else {
                    row.record.expires_at.ok_or_else(Error::storage)?
                })
            ],
        )
        .map_err(|_| Error::storage())?;
        actor_from_connection(&tx, credential)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Protected coordinator state, never exposed as an ordinary operation verb.
    pub fn advance_transition(
        &self,
        credential: &str,
        lease: &TransitionLease,
        next: OperationState,
    ) -> Result<OperationRecord> {
        if lease.cleanup {
            return Err(Error::forbidden());
        }
        if matches!(
            next,
            OperationState::Completed
                | OperationState::Cancelled
                | OperationState::Accepted
                | OperationState::Preparing
        ) {
            return Err(invalid());
        }
        let authority = self.transition_lease_authority(credential, lease)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = lease_authority(&tx, &actor, lease, self, true)?;
        admission::require_row(self, &authority, &row)?;
        if next == OperationState::Failed && rows::unsettled(&tx, &lease.record_id)? {
            return Err(conflict());
        }
        rows::change(&tx, &row, next, None, self)?;
        if next.terminal() || matches!(next, OperationState::Interrupted | OperationState::Unknown)
        {
            invalidate(&tx, &lease.record_id)?;
        }
        actor_from_connection(&tx, credential)?;
        let result = rows::by_record(&tx, &lease.record_id)?.record;
        fault(
            &format!("advance-{}:before-commit", next.as_str()),
            &row.record.operation_id,
        );
        tx.commit().map_err(|_| Error::storage())?;
        fault(
            &format!("advance-{}:after-commit", next.as_str()),
            &row.record.operation_id,
        );
        Ok(result)
    }
    /// Bounded startup/worker recovery. This settles no provider effect or live media.
    pub fn recover_transition_work(&self, limit: u32) -> Result<u32> {
        let kind = admission::BatchKind::Recover;
        let (ids, authority) = self.transition_batch_authority(kind, limit)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let selected = self.transition_batch_rows(&tx, kind, &ids, &authority)?;
        let mut changed = 0;
        for selected in selected {
            if recover_one(
                &tx,
                &rows::by_record(&tx, &selected.id)?,
                &self.transition_runtime,
                self,
            )? {
                changed += 1;
            }
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(changed)
    }
    /// Coordinators in this crate commit their local graph/role decision atomically
    /// with completion. The closure may perform SQL only; remote effects precede it.
    #[allow(dead_code)] // Atomic hook for the G6-039 coordinator; crash fixtures exercise it now.
    pub(crate) fn commit_transition<F>(
        &self,
        credential: &str,
        lease: &TransitionLease,
        apply: F,
    ) -> Result<OperationRecord>
    where
        F: FnOnce(&Connection, &TransitionRequest) -> Result<TransitionCompletion>,
    {
        if lease.cleanup {
            return Err(Error::forbidden());
        }
        let _authority = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                let row = lease_authority(&c, &actor, lease, self, true)?;
                let request = rows::request(&row)?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &request.conversation_id,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = lease_authority(&tx, &actor, lease, self, true)?;
        if row.record.state != OperationState::Committing
            || row
                .record
                .effects
                .iter()
                .any(|e| e.state != EffectState::Confirmed)
        {
            return Err(conflict());
        }
        let request = rows::request(&row)?;
        let completion = apply(&tx, &request)?;
        completion_authority(&tx, &row, &completion, self)?;
        // No callback/network await can fit between this fence and the local commit.
        authority::read(&tx, &actor, &row)?;
        actor_from_connection(&tx, credential)?;
        if row.record.expires_at.is_some_and(|v| v <= crate::now())
            || row.lease_until.is_some_and(|v| v <= crate::now())
        {
            return Err(Error::forbidden());
        }
        rows::change(
            &tx,
            &row,
            OperationState::Completed,
            Some(serde_json::to_value(completion).map_err(|_| Error::storage())?),
            self,
        )?;
        invalidate(&tx, &lease.record_id)?;
        let result = rows::by_record(&tx, &lease.record_id)?.record;
        fault("local-commit:before-commit", &row.record.operation_id);
        tx.commit().map_err(|_| Error::storage())?;
        fault("local-commit:after-commit", &row.record.operation_id);
        Ok(result)
    }
}
pub(crate) fn invalidate(c: &Connection, record: &str) -> Result<()> {
    c.execute("UPDATE operations SET lease_owner=NULL,lease_expires_at=NULL,worker_runtime=NULL,worker_generation=MIN(worker_generation+1,9007199254740991) WHERE record_id=?1",[record]).map_err(|_|Error::storage())?;
    Ok(())
}
fn reservations(c: &Connection, row: &rows::Saved) -> Result<Vec<(String, String, u64)>> {
    let request = rows::request(row)?;
    let mut needed = vec![
        ("session", request.session_id.as_str()),
        ("connection", request.intent.retained()),
    ];
    if let Some(cid) = request.connection_id.as_deref() {
        needed.push(("connection", cid));
    }
    if let TransitionIntent::Handoff {
        role_scope: RoleScope::Conversation,
        ..
    } = request.intent
    {
        needed.push(("conversation", &request.conversation_id));
    }
    needed.into_iter().map(|(kind,id)|{
  let generation:Option<u64>=c.query_row("SELECT fencing_generation FROM operation_resources WHERE tenant_id=?1 AND resource_kind=?2 AND resource_id=?3 AND operation_record_id=?4",params![row.tenant,kind,id,row.record.record_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
  Ok((kind.to_owned(),id.to_owned(),generation.ok_or_else(conflict)?))
 }).collect()
}
pub(super) fn lease_authority(
    c: &Connection,
    actor: &Actor,
    lease: &TransitionLease,
    store: &Store,
    execution: bool,
) -> Result<rows::Saved> {
    actor.require("operation.execute")?;
    let row = rows::by_record(c, &lease.record_id)?;
    authority::read(c, actor, &row)?;
    if (row.initial_telephone || row.native_consultation || row.native_provider)
        || (row.native_return
            || row.native_handoff
            || row.native_replacement
            || admission::candidate_row(&row)?)
    {
        return Err(Error::forbidden());
    }
    if lease.cleanup != row.cleanup
        || lease.runtime != store.transition_runtime
        || row.runtime.as_deref() != Some(&lease.runtime)
        || row.worker.as_deref() != Some(&lease.worker)
        || row.generation != lease.generation
        || row.lease_until.is_none_or(|v| v <= crate::now())
        || row.record.state.terminal()
        || (!lease.cleanup
            && matches!(
                row.record.state,
                OperationState::Unknown | OperationState::Interrupted
            ))
    {
        return Err(Error::forbidden());
    }
    if reservations(c, &row)? != lease.reservations {
        return Err(Error::forbidden());
    }
    if lease.cleanup {
        actor.require("operation.compensate")?;
    } else if execution {
        authority::execution(c, actor, &row, store)?;
    }
    Ok(row)
}
pub(super) fn recover_one(
    c: &Connection,
    row: &rows::Saved,
    runtime: &str,
    store: &Store,
) -> Result<bool> {
    if row.received_dtmf {
        return super::received_dtmf::recover_one(c, row, store);
    }
    if row.initial_telephone {
        return super::initial_telephone::recover_one(c, row, store);
    }
    if row.native_replacement {
        return super::native_replacements::recover_one(c, row, store);
    }
    if row.native_handoff {
        return super::native_handoffs::recover_one(c, row, store);
    }
    if row.native_return {
        return super::native_returns::recover_one(c, row, store);
    }
    if row.native_provider && !row.native_consultation {
        return crate::private_provider::recover_one(c, row, store);
    }
    if row.native_consultation {
        return super::native_consultations::recover_one(c, row, store);
    }
    if row.native_stream {
        super::native_streams::recover_one(c, row, store)?;
        return Ok(
            rows::by_record(c, &row.record.record_id)?.record.revision != row.record.revision
        );
    }
    if row.native {
        super::native_offers::recover_one(c, row, store)?;
        return Ok(
            rows::by_record(c, &row.record.record_id)?.record.revision != row.record.revision
        );
    }
    if !row.ledger || row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(false);
    }
    let expired = row.record.expires_at.is_some_and(|v| v <= crate::now());
    let stale = row.worker.is_some()
        && ((store.recover_foreign_runtime && row.runtime.as_deref() != Some(runtime))
            || row.lease_until.is_none_or(|v| v <= crate::now()));
    if !expired && !stale {
        return Ok(false);
    }
    let unsettled = rows::unsettled(c, &row.record.record_id)?;
    let next = if row.record.state == OperationState::Committing
        || (matches!(
            row.record.state,
            OperationState::Accepted | OperationState::Preparing
        ) && unsettled)
        || (row.record.state == OperationState::Interrupted && unsettled)
    {
        OperationState::Unknown
    } else if expired && !unsettled {
        OperationState::Failed
    } else {
        OperationState::Interrupted
    };
    let interrupted;
    let row = if row.record.state == OperationState::Accepted && next == OperationState::Unknown {
        rows::change(c, row, OperationState::Interrupted, None, store)?;
        interrupted = rows::by_record(c, &row.record.record_id)?;
        &interrupted
    } else {
        row
    };
    c.execute("UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND state='submitting'",[&row.record.record_id]).map_err(|_|Error::storage())?;
    rows::change(c, row, next, None, store)?;
    invalidate(c, &row.record.record_id)?;
    Ok(true)
}
pub(super) fn completion_authority(
    c: &Connection,
    row: &rows::Saved,
    result: &TransitionCompletion,
    store: &Store,
) -> Result<()> {
    completion_scope(c, row, result, true, store)
}
pub(super) fn completion_scope(
    c: &Connection,
    row: &rows::Saved,
    result: &TransitionCompletion,
    committed: bool,
    store: &Store,
) -> Result<()> {
    let request = rows::request(row)?;
    let changed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operation_members m JOIN participants p ON p.participant_id=m.participant_id JOIN identities i ON i.id=p.identity_id WHERE m.operation_record_id=?1 AND (p.active=0 OR p.identity_id!=m.identity_id OR p.membership_epoch!=m.membership_epoch OR i.revoked=1)) OR EXISTS(SELECT 1 FROM sessions s JOIN conversations v ON v.id=s.conversation_id WHERE s.id=?2 AND (v.closed=1 OR s.state NOT IN ('active','accepted','ready')))",params![row.record.record_id,request.session_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if changed {
        return Err(Error::forbidden());
    }
    let revision: u64 = c
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id=?1",
            [&request.session_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let base_revision = super::consultation_source::effective_revision(c, row)?;
    if result.session_revision != base_revision + 1
        || if committed {
            revision != result.session_revision
        } else {
            revision != base_revision && revision != result.session_revision
        }
        || !result
            .retained_connection_ids
            .iter()
            .any(|v| v == request.intent.retained())
        || result.retired_connection_ids.iter().any(|v| {
            result.retained_connection_ids.contains(v) || result.admitted_connection_ids.contains(v)
        })
    {
        return Err(conflict());
    }
    let mut all = std::collections::HashSet::new();
    for id in result
        .retained_connection_ids
        .iter()
        .chain(&result.admitted_connection_ids)
        .chain(&result.retired_connection_ids)
    {
        connect_protocol::transitions::id(id)?;
        if !all.insert(id) {
            return Err(invalid());
        }
        let allowed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN operation_members m ON m.participant_id=v.participant_id AND m.membership_epoch=v.membership_epoch AND m.identity_id=v.identity_id WHERE v.id=?1 AND v.session_id=?2 AND m.operation_record_id=?3)",params![id,request.session_id,row.record.record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !allowed {
            return Err(Error::forbidden());
        }
        let (state, runtime, endpoint): (String, Option<String>, String) = c
            .query_row(
                "SELECT state,observed_runtime_id,endpoint_id FROM connections WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| Error::storage())?;
        if result.retired_connection_ids.contains(id) {
            if !(if committed {
                ["retired", "failed"].contains(&state.as_str())
            } else {
                ["retiring", "retired", "failed"].contains(&state.as_str())
            }) {
                return Err(conflict());
            }
        } else {
            if runtime.as_deref() != Some(store.runtime_epoch())
                || if result.admitted_connection_ids.contains(id) {
                    if committed {
                        state != "active"
                    } else {
                        !["active", "ready"].contains(&state.as_str())
                    }
                } else {
                    !authority::retained_ready(c, id, &state, store)?
                }
            {
                return Err(conflict());
            }
            let verified:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id WHERE e.id=?1 AND e.verification_state='verified' AND d.revoked=0)",[endpoint],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !verified {
                return Err(Error::forbidden());
            }
        }
    }
    if all.len() > 100 {
        return Err(invalid());
    }
    if let TransitionIntent::Handoff {
        role_scope: RoleScope::Conversation,
        expected_conversation_revision,
        ..
    } = request.intent
    {
        let actual: u64 = c
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                [request.conversation_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if result.conversation_revision != expected_conversation_revision.map(|v| v + 1)
            || if committed {
                Some(actual) != result.conversation_revision
            } else {
                Some(actual) != expected_conversation_revision
                    && Some(actual) != result.conversation_revision
            }
        {
            return Err(conflict());
        }
    }
    Ok(())
}
impl Store {
    /// A fresh cleanup-only lease may settle prepared external resources after
    /// cancellation/deadline. It cannot continue or commit the original transition.
    pub fn claim_transition_cleanup(
        &self,
        credential: &str,
        operation: &str,
        ttl: u32,
    ) -> Result<Option<TransitionLease>> {
        if ttl == 0 || ttl > MAX_WORKER_LEASE {
            return Err(invalid());
        }
        id(operation)?;
        let authority = self.transition_operation_authority(
            credential,
            operation,
            &["operation.execute", "operation.compensate"],
            None,
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("operation.execute")?;
        actor.require("operation.compensate")?;
        let tenant = authority::tenant(&tx, &actor)?;
        let row = rows::find(&tx, &tenant, &actor.identity_id, operation)?
            .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &authority, &row)?;
        if !row.ledger
            || (row.initial_telephone || row.native_consultation || row.native_provider) || (row.native_return || row.native_handoff || row.native_replacement || admission::candidate_row(&row)?)
            || tx.query_row("SELECT EXISTS(SELECT 1 FROM consultation_candidate_bindings WHERE operation_record_id=?1)",[&row.record.record_id],|r|r.get::<_,bool>(0)).map_err(|_|Error::storage())?
            || row.tombstoned
            || !matches!(
                row.record.state,
                OperationState::Unknown | OperationState::Interrupted
            )
        {
            return Err(conflict());
        }
        if row.worker.is_some()
            && row.runtime.as_deref() == Some(&self.transition_runtime)
            && row.lease_until.is_some_and(|v| v > crate::now())
        {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        let reservations = reservations(&tx, &row)?;
        if row.generation >= 9007199254740990 {
            return Err(conflict());
        }
        tx.execute("UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND state='submitting'",[&row.record.record_id]).map_err(|_|Error::storage())?;
        let worker = uuid::Uuid::new_v4().to_string();
        let generation = row.generation + 1;
        tx.execute("UPDATE operations SET worker_generation=?2,worker_runtime=?3,worker_cleanup=1,lease_owner=?4,lease_expires_at=?5 WHERE record_id=?1",params![row.record.record_id,generation,self.transition_runtime,worker,(crate::now()+i64::from(ttl)).min(actor.expires_at)]).map_err(|_|Error::storage())?;
        effects::touch(&tx, &row.record.record_id, self)?;
        actor_from_connection(&tx, credential)?;
        let lease = TransitionLease {
            record_id: row.record.record_id,
            worker,
            generation,
            runtime: self.transition_runtime.clone(),
            cleanup: true,
            reservations,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(lease))
    }
}
impl Store {
    /// Runtime/repair entry point. Revoke transient worker authority in its own
    /// transaction before attempting business recovery, independent of hint room.
    /// This append-only fence evidence is part of the operation ledger, not a claim
    /// that media/provider work was stopped or that the operation was cancelled.
    pub fn fence_transition_workers(&self, limit: u32) -> Result<u32> {
        let _runtime = self.require_runtime_owner()?;
        let kind = admission::BatchKind::Fence;
        let (ids, authority) = self.transition_batch_authority(kind, limit)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let selected = self.transition_batch_rows(&tx, kind, &ids, &authority)?;
        for row in &selected {
            let record = &row.id;
            let generation = row.generation;
            let value = serde_json::json!({"record_id":record,"previous_worker_generation":generation,"fenced_worker_generation":generation.saturating_add(1).min(9007199254740991),"runtime":self.transition_runtime});
            let (hash, data) = digest(&value)?;
            tx.execute(
                "INSERT INTO operation_observations VALUES(?1,?2,'worker-fence',?3,?4,NULL,?5,?6)",
                params![
                    format!("obs_{}", uuid::Uuid::new_v4()),
                    record,
                    format!("{record}-{generation}"),
                    hash,
                    data,
                    crate::now()
                ],
            )
            .map_err(|_| Error::storage())?;
            tx.execute("UPDATE operations SET worker_runtime='fenced',lease_expires_at=?2,worker_generation=MIN(worker_generation+1,9007199254740991) WHERE record_id=?1",params![record,crate::now()-1]).map_err(|_|Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(selected.len() as u32)
    }
}
impl Store {
    /// A verified graph probe can settle an uncertain external commit. Its local
    /// role/topology decision and reconciliation receipt still share one transaction.
    /// The purpose-specific coordinator owns the SQL callback, never a request body.
    #[allow(dead_code)] // G6-039/040 coordinator integration; exercised by crash fixtures.
    pub(crate) fn reconcile_transition_commit<F>(
        &self,
        credential: &str,
        operation: &str,
        decision: &str,
        evidence: &str,
        apply: F,
    ) -> Result<OperationRecord>
    where
        F: FnOnce(&Connection, &TransitionRequest) -> Result<TransitionCompletion>,
    {
        for value in [operation, decision, evidence] {
            id(value)?;
        }
        if operation == decision {
            return Err(invalid());
        }
        let _authority = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                actor.require("operation.execute")?;
                actor.require("operation.reconcile")?;
                let tenant = authority::tenant(&c, &actor)?;
                let row = rows::find(&c, &tenant, &actor.identity_id, operation)?
                    .ok_or_else(Error::forbidden)?;
                authority::read(&c, &actor, &row)?;
                let request = rows::request(&row)?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    &request.conversation_id,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("operation.execute")?;
        actor.require("operation.reconcile")?;
        let tenant = authority::tenant(&tx, &actor)?;
        let row = rows::find(&tx, &tenant, &actor.identity_id, operation)?
            .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        crate::operations::reject_control_id(&tx, &actor, decision)?;
        let action = OperationAction::Reconcile {
            operation_id: decision.into(),
            target_operation_id: operation.into(),
            outcome: ReconcileOutcome::Completed,
            evidence_id: evidence.into(),
        };
        let mut value = serde_json::to_value(action).map_err(|_| invalid())?;
        value
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("operation_id");
        let (hash, intent) = digest(&value)?;
        if let Some(old) = rows::find(&tx, &tenant, &actor.identity_id, decision)? {
            authority::read(&tx, &actor, &old)?;
            if old.digest != hash || old.record.command_type != "operation.reconcile" {
                return Err(conflict());
            }
            let result = rows::view(&tx, &actor, old)?;
            actor_from_connection(&tx, credential)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        if !row.ledger
            || (row.initial_telephone || row.native_consultation || row.native_provider)
            || (row.native_return
                || row.native_handoff
                || row.native_replacement
                || admission::candidate_row(&row)?)
            || row.tombstoned
            || !matches!(
                row.record.state,
                OperationState::Unknown | OperationState::Interrupted
            )
            || row
                .record
                .effects
                .iter()
                .any(|e| e.state != EffectState::Confirmed)
        {
            return Err(conflict());
        }
        let proof:Option<(u64,u64,String)>=tx.query_row("SELECT operation_revision,worker_generation,completion_json FROM operation_evidence WHERE observation_id=?1 AND operation_record_id=?2 AND source_kind='local-state' AND outcome='completed'",params![evidence,row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let (revision, generation, expected) = proof.ok_or_else(Error::forbidden)?;
        if revision != row.record.revision || generation != row.generation {
            return Err(conflict());
        }
        let expected: TransitionCompletion =
            serde_json::from_str(&expected).map_err(|_| Error::storage())?;
        completion_scope(&tx, &row, &expected, false, self)?;
        let actual = apply(&tx, &rows::request(&row)?)?;
        if actual != expected {
            return Err(conflict());
        }
        completion_authority(&tx, &row, &actual, self)?;
        authority::read(&tx, &actor, &row)?;
        actor_from_connection(&tx, credential)?;
        rows::change(
            &tx,
            &row,
            OperationState::Completed,
            Some(serde_json::to_value(actual).map_err(|_| Error::storage())?),
            self,
        )?;
        invalidate(&tx, &row.record.record_id)?;
        super::capacity(&tx, self, 1)?;
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        let result = serde_json::json!({"target_operation_id":operation,"state_at_decision":"completed","disposition":"reconciled","evidence_id":evidence});
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,intent_json,state,result_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'operation.reconcile',?6,?7,?8,'completed',?9,?10,?10)",params![record,tenant,actor.identity_id,decision,hash,row.record.conversation_id,row.record.session_id,intent,result.to_string(),crate::now()]).map_err(|_|Error::storage())?;
        tx.execute(
            "INSERT INTO operation_decisions VALUES(?1,?2,'reconcile')",
            params![record, row.record.record_id],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("UPDATE journal SET data_json=json_set(data_json,'$.decision_record_id',?2) WHERE operation_record_id=?1 AND resource_revision=?3",params![row.record.record_id,record,row.record.revision+1]).map_err(|_|Error::storage())?;
        let result = rows::by_record(&tx, &record)?.record;
        fault("reconcile-local:before-commit", operation);
        tx.commit().map_err(|_| Error::storage())?;
        fault("reconcile-local:after-commit", operation);
        Ok(result)
    }
}
