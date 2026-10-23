//! Frozen updates belong to one owned instance; every earlier memory stays live.
use super::*;
const MAX_UPDATES: usize = 16;
#[derive(Clone)]
pub struct AssistantProviderUpdate {
    context: AssistantProviderContext,
    delivery: String,
    attempt: String,
    reference: String,
}
pub struct AssistantProviderUpdateExport {
    update: AssistantProviderUpdate,
    json: String,
}
pub struct AssistantProviderUpdateApplied {
    update: AssistantProviderUpdate,
}
pub struct AssistantProviderUpdateRejected {
    update: AssistantProviderUpdate,
    problem: AssistantContextFailure,
    export: bool,
}
pub struct AssistantProviderUpdateStatus {
    pub state: String,
    pub export_started: bool,
}
redacted!(
    AssistantProviderUpdate,
    AssistantProviderUpdateExport,
    AssistantProviderUpdateApplied,
    AssistantProviderUpdateRejected,
    AssistantProviderUpdateStatus
);
impl AssistantProviderUpdate {
    pub fn instance(&self) -> &AssistantProviderContext {
        &self.context
    }
    pub fn delivery_id(&self) -> &str {
        &self.delivery
    }
    pub fn not_exported(&self) -> AssistantProviderUpdateRejected {
        AssistantProviderUpdateRejected {
            update: self.clone(),
            problem: AssistantContextFailure::Incomplete,
            export: false,
        }
    }
}
impl AssistantProviderUpdateExport {
    pub fn acknowledged(&self, body: &str) -> Result<AssistantProviderUpdateApplied> {
        if body != self.json {
            return Err(Error::new(
                "context-ack-mismatch",
                "Provider did not acknowledge the exact update",
            ));
        }
        Ok(AssistantProviderUpdateApplied {
            update: self.update.clone(),
        })
    }
    pub fn rejected(&self, problem: AssistantContextFailure) -> AssistantProviderUpdateRejected {
        AssistantProviderUpdateRejected {
            update: self.update.clone(),
            problem,
            export: true,
        }
    }
}
fn bind(c: &Connection, update: &AssistantProviderUpdate) -> Result<()> {
    bound(c, &update.context)?;
    let same:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_context_updates u JOIN native_assistant_providers p ON p.delivery_id=u.instance_delivery_id WHERE u.delivery_id=?1 AND u.attempt_id=?2 AND u.instance_delivery_id=?3 AND p.provider_ref=?4)",params![update.delivery,update.attempt,update.context.delivery,update.reference],|r|r.get(0)).map_err(|_|Error::storage())?;
    if same {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
pub(super) fn validate_saved(
    store: &Store,
    c: &Connection,
    f: &Facts,
    context: &AssistantProviderContext,
    json: &str,
) -> Result<()> {
    if json.len() > 65_536 {
        return Err(Error::storage());
    }
    let saved: connect_protocol::ContextView =
        serde_json::from_str(json).map_err(|_| Error::storage())?;
    let current = crate::context::view_for_audience_scope_at(
        store,
        c,
        &f.actor,
        context.origin.conversation_id(),
        1000,
        Some(&f.delegate.identity_id),
        Some(&f.delegate),
        Some(saved.watermark),
        None,
        Some(&f.audience),
    )?;
    if serde_json::to_value(saved).map_err(|_| Error::storage())?
        != serde_json::to_value(current).map_err(|_| Error::storage())?
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn validate_rows(
    store: &Store,
    c: &Connection,
    f: &Facts,
    context: &AssistantProviderContext,
    allow: Option<&str>,
) -> Result<Option<String>> {
    let mut q=c.prepare("SELECT u.delivery_id,d.selection_json,u.export_started,u.delivered_at IS NOT NULL,u.rejected_at IS NOT NULL,u.uncertain FROM native_assistant_context_updates u JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.instance_delivery_id=?1 ORDER BY u.ordinal LIMIT 17").map_err(|_|Error::storage())?;
    let rows = q
        .query_map([&context.delivery], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, bool>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if rows.len() > MAX_UPDATES {
        return Err(Error::storage());
    }
    let mut selected = None;
    for (id, json, started, delivered, rejected, uncertain) in rows {
        let own = allow == Some(id.as_str());
        if started && (rejected || uncertain || !delivered && !own) {
            return Err(Error::new(
                "reconciliation-required",
                "An update may have changed this provider memory",
            ));
        }
        if started || own {
            if own && (rejected || uncertain) {
                return Err(Error::forbidden());
            }
            validate_saved(store, c, f, context, &json)?;
        }
        if own {
            selected = Some(json);
        }
    }
    if allow.is_some() && selected.is_none() {
        return Err(Error::forbidden());
    }
    Ok(selected)
}
impl Store {
    /// Reuse the original complete read interval. Never acquire another reader
    /// behind a queued writer merely to validate later opaque model memory.
    pub(super) fn validate_assistant_updates(
        &self,
        context: &AssistantProviderContext,
        read: &NativeAssistantContextRead,
        allow: Option<&str>,
    ) -> Result<Option<String>> {
        let c = self.try_connection()?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let any:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_context_updates WHERE instance_delivery_id=?1)",[&context.delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !any && allow.is_none() {
            return Ok(None);
        }
        let f = resolve(
            self,
            &c,
            &context.origin,
            &context.credential,
            &context.endpoint,
        )?;
        if !read.current()
            || !self.authority_covers(&read.native._scopes, &f.scopes)
            || crate::provisioning::canonical_json(&read.endpoint)?
                != crate::provisioning::canonical_json(&f.endpoint)?
            || read.audience != f.audience
        {
            return Err(Error::forbidden());
        }
        let selected = validate_rows(self, &c, &f, context, allow)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(selected)
    }
    pub fn prepare_native_assistant_update(
        &self,
        operation_id: &str,
        context: &AssistantProviderContext,
        applied: &AssistantProviderApplied,
        memory: AssistantProviderRead,
    ) -> Result<AssistantProviderUpdate> {
        if operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid assistant update operation ID",
            ));
        }
        if !memory.current()
            || memory.context.delivery != context.delivery
            || memory.context.attempt != context.attempt
            || applied.context.delivery != context.delivery
            || applied.context.attempt != context.attempt
            || memory.provider_ref.as_deref() != Some(applied.reference.as_str())
            || memory.update.is_some()
        {
            return Err(Error::forbidden());
        }
        drop(memory);
        original_runtime(self, context)?;
        let held = self.runtime_conversation_authority(|_| {
            Ok(vec![context.origin.conversation_id().into()])
        })?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        let f = resolve(
            self,
            &tx,
            &context.origin,
            &context.credential,
            &context.endpoint,
        )?;
        if !self.authority_covers(&held, &f.scopes) {
            return Err(Error::forbidden());
        }
        let (owner,json,endpoint,audience):(String,String,String,String)=tx.query_row("SELECT p.owner_id,d.selection_json,p.endpoint_json,p.audience_json FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1 AND p.provider_ref=?2 AND p.delivered_at IS NOT NULL AND p.uncertain=0 AND p.rejected_at IS NULL AND p.end_started=0 AND p.closed_at IS NULL AND d.state='delivered'",params![context.delivery,applied.reference],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::forbidden())?;
        if endpoint != crate::provisioning::canonical_json(&f.endpoint)?
            || audience != serde_json::to_string(&f.audience).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        validate_saved(self, &tx, &f, context, &json)?;
        validate_rows(self, &tx, &f, context, None)?;
        let existing:Option<(String,String,String)>=tx.query_row("SELECT delivery_id,attempt_id,instance_delivery_id FROM native_assistant_context_updates WHERE owner_id=?1 AND operation_id=?2",params![owner,operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if let Some((delivery, attempt, parent)) = existing {
            if parent != context.delivery {
                return Err(Error::new(
                    "conflict",
                    "Update operation belongs to another instance",
                ));
            }
            return Ok(AssistantProviderUpdate {
                context: context.clone(),
                delivery,
                attempt,
                reference: applied.reference.clone(),
            });
        }
        let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_providers WHERE owner_id=?1 AND operation_id=?2) OR EXISTS(SELECT 1 FROM native_assistant_context_updates u JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.instance_delivery_id=?3 AND d.state='pending')",params![owner,operation_id,context.delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
        if conflict {
            return Err(Error::new(
                "conflict",
                "Another context operation is already reserved",
            ));
        }
        let count:u64=tx.query_row("SELECT COUNT(*) FROM native_assistant_context_updates WHERE instance_delivery_id=?1",[&context.delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
        if count >= MAX_UPDATES as u64 {
            return Err(Error::new(
                "capacity-exceeded",
                "This provider memory has reached its update budget",
            ));
        }
        let view = crate::context::view_for_audience_scope_at(
            self,
            &tx,
            &f.actor,
            context.origin.conversation_id(),
            1000,
            Some(&f.delegate.identity_id),
            Some(&f.delegate),
            None,
            None,
            Some(&f.audience),
        )?;
        let json = serde_json::to_string(&view).map_err(|_| Error::storage())?;
        if json.len() > 65_536 {
            return Err(Error::new(
                "capacity-exceeded",
                "Assistant update exceeds its byte budget",
            ));
        }
        let update = AssistantProviderUpdate {
            context: context.clone(),
            delivery: format!("auctx_{}", uuid::Uuid::new_v4()),
            attempt: format!("aua_{}", uuid::Uuid::new_v4()),
            reference: applied.reference.clone(),
        };
        tx.execute("INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,provider_ref,created_at,updated_at,sequence,owner_id) VALUES(?1,?2,?3,'vapi',?4,?5,'pending',?6,?7,?7,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),?8)",params![update.delivery,context.origin.conversation_id(),context.origin.session_id(),view.watermark,json,update.reference,crate::now(),owner]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_assistant_context_updates(delivery_id,instance_delivery_id,owner_id,operation_id,attempt_id,ordinal,prepared_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![update.delivery,context.delivery,owner,operation_id,update.attempt,count+1,f.expires]).map_err(|_|Error::storage())?;
        for (_, pid, epoch) in &f.audience {
            tx.execute("INSERT OR IGNORE INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch) VALUES('context_delivery',?1,?2,?3,?4)",params![update.delivery,context.origin.conversation_id(),pid,epoch]).map_err(|_|Error::storage())?;
        }
        progress(&tx, &update.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(update)
    }
    pub fn native_assistant_update_status(
        &self,
        update: &AssistantProviderUpdate,
    ) -> Result<AssistantProviderUpdateStatus> {
        original_runtime(self, &update.context)?;
        let c = self.try_connection()?;
        bind(&c, update)?;
        c.query_row("SELECT d.state,u.export_started FROM native_assistant_context_updates u JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.delivery_id=?1",[&update.delivery],|r|Ok(AssistantProviderUpdateStatus{state:r.get(0)?,export_started:r.get(1)?})).map_err(|_|Error::storage())
    }
    pub fn try_native_assistant_update_read(
        &self,
        update: &AssistantProviderUpdate,
        applied: &AssistantProviderApplied,
    ) -> Result<AssistantProviderRead> {
        if applied.context.delivery != update.context.delivery
            || applied.context.attempt != update.context.attempt
            || applied.reference != update.reference
        {
            return Err(Error::forbidden());
        }
        {
            let c = self.try_connection()?;
            bind(&c, update)?;
            let fresh:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_context_updates u JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.delivery_id=?1 AND u.export_started=0 AND u.rejected_at IS NULL AND u.uncertain=0 AND d.state='pending')",[&update.delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !fresh {
                return Err(Error::new(
                    "reconciliation-required",
                    "Update export was already admitted or settled",
                ));
            }
        }
        let mut read = self.native_assistant_provider_read(
            &update.context,
            Some(&update.reference),
            Some(&update.delivery),
        )?;
        read.read.json = self
            .validate_assistant_updates(&update.context, &read.read, Some(&update.delivery))?
            .ok_or_else(Error::forbidden)?;
        read.update = Some(update.delivery.clone());
        Ok(read)
    }
    pub fn begin_native_assistant_update(
        &self,
        update: &AssistantProviderUpdate,
        read: AssistantProviderRead,
    ) -> Result<(AssistantProviderUpdateExport, AssistantProviderRead)> {
        if !read.current()
            || read.update.as_deref() != Some(update.delivery.as_str())
            || read.context.delivery != update.context.delivery
            || read.context.attempt != update.context.attempt
            || read.provider_ref.as_deref() != Some(update.reference.as_str())
        {
            return Err(Error::forbidden());
        }
        let json = read.context_json()?.to_owned();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bind(&tx, update)?;
        if tx.execute("UPDATE native_assistant_context_updates SET export_started=1,export_expires_at=?2 WHERE delivery_id=?1 AND export_started=0 AND delivered_at IS NULL AND rejected_at IS NULL AND uncertain=0 AND EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=instance_delivery_id AND p.provider_ref=?3 AND p.delivered_at IS NOT NULL AND p.uncertain=0 AND p.end_started=0 AND p.closed_at IS NULL)",params![update.delivery,read.expires_at(),update.reference]).map_err(|_|Error::storage())?!=1{return Err(Error::new("reconciliation-required","Context update was already attempted"));}
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            AssistantProviderUpdateExport {
                update: update.clone(),
                json,
            },
            read,
        ))
    }
    pub fn record_native_assistant_update_delivered(
        &self,
        applied: &AssistantProviderUpdateApplied,
    ) -> Result<()> {
        let update = &applied.update;
        original_runtime(self, &update.context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bind(&tx, update)?;
        let delivered:Option<i64>=tx.query_row("SELECT delivered_at FROM native_assistant_context_updates WHERE delivery_id=?1 AND export_started=1 AND rejected_at IS NULL",[&update.delivery],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if delivered.is_some() {
            return Ok(());
        }
        tx.execute(
            "UPDATE native_assistant_context_updates SET delivered_at=?2 WHERE delivery_id=?1",
            params![update.delivery, crate::now()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("UPDATE context_deliveries SET state='delivered',updated_at=?2 WHERE id=?1 AND state IN ('pending','uncertain')",params![update.delivery,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &update.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn record_native_assistant_update_rejected(
        &self,
        rejected: &AssistantProviderUpdateRejected,
    ) -> Result<()> {
        let update = &rejected.update;
        original_runtime(self, &update.context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bind(&tx, update)?;
        let (at,problem):(Option<i64>,Option<String>)=tx.query_row("SELECT rejected_at,problem FROM native_assistant_context_updates WHERE delivery_id=?1 AND export_started=?2 AND delivered_at IS NULL",params![update.delivery,rejected.export],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::forbidden())?;
        if at.is_some() {
            return if problem.as_deref() == Some(rejected.problem.as_str()) {
                Ok(())
            } else {
                Err(Error::new("conflict", "Update failure observation changed"))
            };
        }
        tx.execute("UPDATE native_assistant_context_updates SET rejected_at=?2,problem=?3 WHERE delivery_id=?1",params![update.delivery,crate::now(),rejected.problem.as_str()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE context_deliveries SET state='failed',updated_at=?2 WHERE id=?1 AND state IN ('pending','uncertain')",params![update.delivery,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &update.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn native_assistant_update_uncertain(
        &self,
        export: &AssistantProviderUpdateExport,
    ) -> Result<()> {
        self.assistant_update_uncertain_problem(export, "unknown")
    }
    pub fn native_assistant_update_incomplete(
        &self,
        export: &AssistantProviderUpdateExport,
    ) -> Result<()> {
        self.assistant_update_uncertain_problem(export, "incomplete")
    }
    fn assistant_update_uncertain_problem(
        &self,
        export: &AssistantProviderUpdateExport,
        problem: &str,
    ) -> Result<()> {
        let update = &export.update;
        original_runtime(self, &update.context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bind(&tx, update)?;
        let before:bool=tx.query_row("SELECT uncertain FROM native_assistant_context_updates WHERE delivery_id=?1 AND export_started=1 AND delivered_at IS NULL AND rejected_at IS NULL",[&update.delivery],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if before {
            return Ok(());
        }
        tx.execute("UPDATE native_assistant_context_updates SET uncertain=1,problem=?2 WHERE delivery_id=?1",params![update.delivery,problem]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE context_deliveries SET state='uncertain',updated_at=?2 WHERE id=?1 AND state='pending'",params![update.delivery,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &update.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub(super) fn recover_native_assistant_context_updates(&self, limit: usize) -> Result<usize> {
        const TARGETS:&str="SELECT d.conversation_id FROM native_assistant_context_updates u JOIN native_assistant_providers p ON p.delivery_id=u.instance_delivery_id JOIN context_deliveries d ON d.id=u.delivery_id WHERE d.state='pending' AND (p.runtime_epoch!=?2 OR (u.export_started=0 AND u.prepared_expires_at<=?1) OR (u.export_started=1 AND u.export_expires_at<=?1)) ORDER BY d.sequence LIMIT ?3";
        let now = crate::now();
        let limit = limit as u64;
        let held = self.runtime_query_authority(TARGETS, &[&now, &self.runtime_epoch(), &limit])?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_runtime_query(&tx, &held, TARGETS, &[&now, &self.runtime_epoch(), &limit])?;
        let sql = TARGETS.replacen(
            "SELECT d.conversation_id",
            "SELECT u.delivery_id,u.export_started",
            1,
        );
        let rows = tx
            .prepare(&sql)
            .map_err(|_| Error::storage())?
            .query_map(params![now, self.runtime_epoch(), limit], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        for (id, started) in &rows {
            tx.execute("UPDATE native_assistant_context_updates SET uncertain=?2,rejected_at=CASE WHEN export_started=0 THEN ?3 ELSE rejected_at END,problem=?4 WHERE delivery_id=?1",params![id,started,now,if *started{"unknown"}else{"incomplete"}]).map_err(|_|Error::storage())?;
            tx.execute(
                "UPDATE context_deliveries SET state=?2,updated_at=?3 WHERE id=?1",
                params![id, if *started { "uncertain" } else { "failed" }, now],
            )
            .map_err(|_| Error::storage())?;
            progress(&tx, id, self)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(rows.len())
    }
}
