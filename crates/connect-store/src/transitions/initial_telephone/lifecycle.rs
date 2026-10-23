use super::*;
/// One admitted signaling attempt for one immutable Core route and wire dialog.
/// Retransmissions still require a fresh current packet interval for this binding.
pub struct InitialTelephoneSignaling {
    attempt: Arc<InitialTelephoneAttempt>,
    core: String,
    call_id: String,
}
impl std::fmt::Debug for InitialTelephoneSignaling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InitialTelephoneSignaling(redacted)")
    }
}
impl InitialTelephoneSignaling {
    pub fn origin(&self) -> &InitialTelephoneOrigin {
        &self.attempt.origin
    }
    pub fn runtime_connection_id(&self) -> &str {
        &self.core
    }
    pub fn sip_call_id(&self) -> &str {
        &self.call_id
    }
}
fn allocation(
    c: &Connection,
    attempt: &InitialTelephoneAttempt,
) -> Result<Option<(String, String)>> {
    c.query_row("SELECT runtime_connection_id,sip_call_id FROM initial_telephone_operations WHERE operation_record_id=?1",[&attempt.origin.record],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?))).map_err(|_|Error::storage()).and_then(|v|match v {(None,None)=>Ok(None),(Some(a),Some(b))=>Ok(Some((a,b))),_=>Err(Error::storage())})
}
impl Store {
    pub fn begin_initial_telephone_allocation(
        &self,
        origin: &InitialTelephoneOrigin,
    ) -> Result<Option<InitialTelephoneAttempt>> {
        let _guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                origin.current(self, &c)?;
                scopes(&origin.source, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        origin.current(self, &tx)?;
        let row = rows::by_record(&tx, &origin.record)?;
        let started:bool=tx.query_row("SELECT allocation_started FROM initial_telephone_operations WHERE operation_record_id=?1",[&origin.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if started || row.record.state != OperationState::Accepted {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        let attempt = format!("tattempt_{}", uuid::Uuid::new_v4().simple());
        tx.execute("UPDATE initial_telephone_operations SET allocation_started=1,attempt_id=?2,attempt_runtime=?3 WHERE operation_record_id=?1",params![origin.record,attempt,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operations SET lease_owner=?2,worker_runtime=?3,worker_generation=1,lease_expires_at=expires_at WHERE record_id=?1",params![origin.record,attempt,self.transition_runtime]).map_err(|_|Error::storage())?;
        rows::change(&tx, &row, OperationState::Preparing, None, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(InitialTelephoneAttempt {
            origin: origin.clone(),
            attempt,
        }))
    }
    /// Commit the actual dormant Core allocation and planned SIP Call-ID before
    /// any submission marker. Retrying metadata may only repeat the exact pair.
    pub fn record_initial_telephone_allocation(
        &self,
        attempt: &InitialTelephoneAttempt,
        core: &str,
        call_id: &str,
    ) -> Result<()> {
        id(core)?;
        if call_id.is_empty()
            || call_id.len() > 512
            || call_id.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(invalid());
        }
        let _guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                attempt.require(self, &c)?;
                scopes(&attempt.origin.source, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        attempt.require(self, &tx)?;
        if let Some(old) = allocation(&tx, attempt)? {
            if old != (core.into(), call_id.into()) {
                return Err(conflict());
            }
        } else {
            tx.execute("UPDATE initial_telephone_operations SET runtime_connection_id=?2,sip_call_id=?3 WHERE operation_record_id=?1 AND signaling_started=0",params![attempt.origin.record,core,call_id]).map_err(|_|Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Persist uncertainty before the first INVITE can be polled. A replay never
    /// returns another signaling owner, including after a missing response.
    pub fn begin_initial_telephone_signaling(
        &self,
        attempt: Arc<InitialTelephoneAttempt>,
    ) -> Result<Option<InitialTelephoneSignaling>> {
        let _guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                attempt.require(self, &c)?;
                scopes(&attempt.origin.source, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        attempt.require(self, &tx)?;
        let started:bool=tx.query_row("SELECT signaling_started FROM initial_telephone_operations WHERE operation_record_id=?1",[&attempt.origin.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if started {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        let (core, call_id) = allocation(&tx, &attempt)?.ok_or_else(Error::forbidden)?;
        let effect = format!("teffect_{}", uuid::Uuid::new_v4().simple());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,provider_ref) VALUES(?1,?2,'sip-initial',?2,'none','submitting',1,1,?3)",params![effect,attempt.origin.record,call_id]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE initial_telephone_operations SET signaling_started=1,effect_id=?2 WHERE operation_record_id=?1 AND signaling_started=0",params![attempt.origin.record,effect]).map_err(|_|Error::storage())?;
        super::super::effects::touch(&tx, &attempt.origin.record, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(InitialTelephoneSignaling {
            attempt,
            core,
            call_id,
        }))
    }
    pub fn initial_telephone_allocation_read(
        &self,
        attempt: &InitialTelephoneAttempt,
    ) -> Result<InitialTelephoneRead> {
        self.initial_telephone_read(attempt, None)
    }
    pub fn initial_telephone_signaling_read(
        &self,
        signaling: &InitialTelephoneSignaling,
    ) -> Result<InitialTelephoneRead> {
        self.initial_telephone_read(
            &signaling.attempt,
            Some((&signaling.core, &signaling.call_id)),
        )
    }
    fn initial_telephone_read(
        &self,
        attempt: &InitialTelephoneAttempt,
        binding: Option<(&str, &str)>,
    ) -> Result<InitialTelephoneRead> {
        let guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                attempt.require(self, &c)?;
                scopes(&attempt.origin.source, false)
            },
        )?;
        let c = self.connection()?;
        attempt.require(self, &c)?;
        let started:bool=c.query_row("SELECT signaling_started FROM initial_telephone_operations WHERE operation_record_id=?1",[&attempt.origin.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        match binding {
            Some((core, call))
                if started
                    && allocation(&c, attempt)?
                        .as_ref()
                        .is_some_and(|v| v.0 == core && v.1 == call) => {}
            None if !started => {}
            _ => return Err(Error::forbidden()),
        }
        let (endpoint_actor, route_actor, expires_at) = attempt.origin.current(self, &c)?;
        let conversation_revision: u64 = c
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                [&attempt.origin.source.request.conversation_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        Ok(InitialTelephoneRead {
            _guard: guard,
            conversation_revision,
            origin: attempt.origin.clone(),
            endpoint_actor,
            route_actor,
            expires_at,
            binding: binding.map(|(a, b)| (a.into(), b.into())),
        })
    }
}
pub(in crate::transitions) fn cancel(
    c: &Connection,
    row: &rows::Saved,
    store: &Store,
) -> Result<&'static str> {
    c.execute(
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        [&row.record.record_id],
    )
    .map_err(|_| Error::storage())?;
    settle_abandoned(c, row, store, true)?;
    Ok(
        if rows::by_record(c, &row.record.record_id)?.record.state == OperationState::Unknown {
            "reconciliation_required"
        } else {
            "cancelled_before_commit"
        },
    )
}
fn settle_abandoned(c: &Connection, row: &rows::Saved, store: &Store, cancel: bool) -> Result<()> {
    let (started,effect):(bool,Option<String>)=c.query_row("SELECT signaling_started,effect_id FROM initial_telephone_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let next = if started {
        OperationState::Unknown
    } else if cancel {
        OperationState::Cancelled
    } else {
        OperationState::Failed
    };
    if started {
        c.execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
            [effect],
        )
        .map_err(|_| Error::storage())?;
        c.execute("UPDATE connections SET state='unknown',revision=revision+1,readiness_json=NULL WHERE id=?1 AND state NOT IN ('retired','failed')",[&row.record.connection_id]).map_err(|_|Error::storage())?;
    } else {
        c.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1 WHERE id=?1 AND state='preparing'",[&row.record.connection_id]).map_err(|_|Error::storage())?;
    }
    rows::change(c, row, next, None, store)?;
    super::super::workers::invalidate(c, &row.record.record_id)
}
pub(in crate::transitions) fn recover_one(
    c: &Connection,
    row: &rows::Saved,
    store: &Store,
) -> Result<bool> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(false);
    }
    let (started,runtime):(bool,Option<String>)=c.query_row("SELECT allocation_started,attempt_runtime FROM initial_telephone_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let expired = row.record.expires_at.is_none_or(|v| v <= crate::now());
    let abandoned = started
        && (runtime.as_deref() != Some(store.runtime_epoch())
            || row.runtime.as_deref() != Some(store.transition_runtime.as_str())
            || row.lease_until.is_none_or(|v| v <= crate::now()));
    if expired || abandoned {
        settle_abandoned(c, row, store, false)?;
        return Ok(true);
    }
    Ok(false)
}
