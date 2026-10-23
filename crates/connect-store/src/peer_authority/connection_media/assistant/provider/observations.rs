use super::*;
#[derive(Clone, Copy, Debug)]
pub enum AssistantContextFailure {
    Rejected,
    Truncated,
    Incomplete,
}
impl AssistantContextFailure {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Truncated => "truncated",
            Self::Incomplete => "incomplete",
        }
    }
}
pub struct AssistantProviderRejected {
    context: AssistantProviderContext,
    reference: Option<String>,
    problem: AssistantContextFailure,
    export: bool,
}
pub struct AssistantProviderEnd {
    context: AssistantProviderContext,
    reference: String,
}
pub struct AssistantProviderClosed {
    context: AssistantProviderContext,
    reference: String,
}
redacted!(
    AssistantProviderRejected,
    AssistantProviderEnd,
    AssistantProviderClosed
);
impl AssistantProviderAttempt {
    pub fn not_submitted(&self) -> AssistantProviderRejected {
        AssistantProviderRejected {
            context: self.context.clone(),
            reference: None,
            problem: AssistantContextFailure::Incomplete,
            export: false,
        }
    }
    pub fn rejected(&self) -> AssistantProviderRejected {
        AssistantProviderRejected {
            context: self.context.clone(),
            reference: None,
            problem: AssistantContextFailure::Rejected,
            export: false,
        }
    }
}
impl AssistantProviderExport {
    pub fn rejected(&self, problem: AssistantContextFailure) -> AssistantProviderRejected {
        AssistantProviderRejected {
            context: self.context.clone(),
            reference: Some(self.reference.clone()),
            problem,
            export: true,
        }
    }
}
impl AssistantProviderCreated {
    /// The admitted creation was captured, but no export was submitted.
    pub fn not_exported(&self) -> AssistantProviderRejected {
        AssistantProviderRejected {
            context: self.context.clone(),
            reference: Some(self.reference.clone()),
            problem: AssistantContextFailure::Incomplete,
            export: false,
        }
    }
}
impl AssistantProviderEnd {
    /// Only after actual terminal provider status or completion of the admitted
    /// end exchange, never merely because the local socket was dropped.
    pub fn closed(self) -> AssistantProviderClosed {
        AssistantProviderClosed {
            context: self.context,
            reference: self.reference,
        }
    }
}
impl Store {
    pub fn record_native_assistant_rejected(
        &self,
        receipt: &AssistantProviderRejected,
    ) -> Result<()> {
        let context = &receipt.context;
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        let (rejected,problem):(Option<i64>,Option<String>)=tx.query_row("SELECT rejected_at,problem FROM native_assistant_providers WHERE delivery_id=?1 AND creation_started=1 AND provider_ref IS ?2 AND delivered_at IS NULL AND export_started=?3",params![context.delivery,receipt.reference,receipt.export],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::forbidden())?;
        if rejected.is_some() {
            if problem.as_deref() == Some(receipt.problem.as_str()) {
                return Ok(());
            }
            return Err(Error::new(
                "conflict",
                "Provider failure observation changed",
            ));
        }
        tx.execute(
            "UPDATE native_assistant_providers SET rejected_at=?2,problem=?3 WHERE delivery_id=?1",
            params![context.delivery, crate::now(), receipt.problem.as_str()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("UPDATE context_deliveries SET state='failed',updated_at=?2 WHERE id=?1 AND state IN ('pending','uncertain')",params![context.delivery,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Cleanup is pinned to the original captured call. It cannot export data,
    /// create another call, change target or repeat an uncertain end command.
    pub fn begin_native_assistant_end(
        &self,
        created: &AssistantProviderCreated,
    ) -> Result<AssistantProviderEnd> {
        let context = &created.context;
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        if tx.execute("UPDATE native_assistant_providers SET end_started=1,end_expires_at=?3 WHERE delivery_id=?1 AND provider_ref=?2 AND end_started=0 AND closed_at IS NULL",params![context.delivery,created.reference,crate::now()+5]).map_err(|_|Error::storage())?!=1 {return Err(Error::new("reconciliation-required","Provider cleanup already attempted"));}
        tx.commit().map_err(|_| Error::storage())?;
        Ok(AssistantProviderEnd {
            context: context.clone(),
            reference: created.reference.clone(),
        })
    }
    pub fn native_assistant_end_uncertain(&self, end: &AssistantProviderEnd) -> Result<()> {
        let context = &end.context;
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        if tx.execute("UPDATE native_assistant_providers SET end_uncertain=1 WHERE delivery_id=?1 AND provider_ref=?2 AND end_started=1 AND closed_at IS NULL",params![context.delivery,end.reference]).map_err(|_|Error::storage())?!=1{return Err(Error::forbidden());}
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn record_native_assistant_closed(&self, closed: &AssistantProviderClosed) -> Result<()> {
        let context = &closed.context;
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        let before:Option<i64>=tx.query_row("SELECT closed_at FROM native_assistant_providers WHERE delivery_id=?1 AND provider_ref=?2 AND end_started=1",params![context.delivery,closed.reference],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if before.is_some() {
            return Ok(());
        }
        tx.execute(
            "UPDATE native_assistant_providers SET closed_at=?2 WHERE delivery_id=?1",
            params![context.delivery, crate::now()],
        )
        .map_err(|_| Error::storage())?;
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Bounded metadata recovery. A possible external creation/export is kept
    /// uncertain; this never sends IO or reconstructs/adopts an old capability.
    pub fn recover_native_assistant_providers(&self, limit: usize) -> Result<usize> {
        if limit == 0 || limit > 1000 {
            return Err(Error::new("invalid-request", "Invalid recovery batch size"));
        }
        let recovered_updates = self.recover_native_assistant_context_updates(limit)?;
        if recovered_updates == limit {
            return Ok(recovered_updates);
        }
        const TARGETS:&str="SELECT d.conversation_id FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.closed_at IS NULL AND p.end_uncertain=0 AND ((p.runtime_epoch!=?2) OR (d.state='pending' AND ((p.creation_started=0 AND p.prepared_expires_at<=?1) OR (p.creation_started=1 AND p.provider_ref IS NULL AND p.creation_expires_at<=?1) OR (p.created_at IS NOT NULL AND p.export_started=0 AND p.created_at+5<=?1) OR (p.export_started=1 AND p.delivered_at IS NULL AND p.export_expires_at<=?1))) OR (p.end_started=1 AND p.end_expires_at<=?1)) ORDER BY d.sequence LIMIT ?3";
        let now = crate::now();
        let batch = (limit - recovered_updates) as u64;
        let held = self.runtime_query_authority(TARGETS, &[&now, &self.runtime_epoch(), &batch])?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(&tx, &held, TARGETS, &[&now, &self.runtime_epoch(), &batch])?;
        let sql = TARGETS.replacen(
            "SELECT d.conversation_id",
            "SELECT p.delivery_id,p.creation_started,p.delivered_at IS NOT NULL",
            1,
        );
        let rows = {
            let mut statement = tx.prepare(&sql).map_err(|_| Error::storage())?;
            let rows = statement
                .query_map(params![now, self.runtime_epoch(), batch], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, bool>(2)?,
                    ))
                })
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            rows
        };
        for (id, started, delivered) in &rows {
            let state = if *started { "uncertain" } else { "failed" };
            tx.execute("UPDATE native_assistant_providers SET end_uncertain=1,uncertain=?2,problem=?3,rejected_at=CASE WHEN creation_started=0 THEN ?4 ELSE rejected_at END WHERE delivery_id=?1",params![id,started,if *started{"unknown"}else{"incomplete"},now]).map_err(|_|Error::storage())?;
            if !delivered {
                tx.execute("UPDATE context_deliveries SET state=?2,updated_at=?3 WHERE id=?1 AND state='pending'",params![id,state,now]).map_err(|_|Error::storage())?;
            }
            progress(&tx, id, self)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(rows.len() + recovered_updates)
    }
}
