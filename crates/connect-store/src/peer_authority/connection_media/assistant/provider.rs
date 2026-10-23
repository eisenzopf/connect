//! Original public native destination, one fresh provider attempt and exact ACK.
use super::*;
use rusqlite::{OptionalExtension, TransactionBehavior};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantProviderTiming {
    AfterCreation,
    Initialization,
}

#[derive(Clone)]
pub struct AssistantProviderContext {
    origin: NativeMediaOrigin,
    credential: String,
    endpoint: String,
    delivery: String,
    attempt: String,
    runtime: String,
    initialization: bool,
    routing: Option<(
        crate::DiscoveryRuntime,
        connect_protocol::routing::ReachabilityPolicy,
    )>,
}
pub struct AssistantProviderRead {
    context: AssistantProviderContext,
    read: NativeAssistantContextRead,
    provider_ref: Option<String>,
    initialization_admitted: bool,
    update: Option<String>,
}
#[derive(Clone)]
pub struct AssistantProviderAttempt {
    context: AssistantProviderContext,
}
pub struct AssistantProviderCreated {
    context: AssistantProviderContext,
    reference: String,
}
pub struct AssistantProviderExport {
    context: AssistantProviderContext,
    reference: String,
    json: String,
}
pub struct AssistantProviderApplied {
    context: AssistantProviderContext,
    reference: String,
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
mod initialization;
mod observations;
mod updates;
pub use initialization::AssistantProviderInitialization;
pub use observations::{
    AssistantContextFailure, AssistantProviderClosed, AssistantProviderEnd,
    AssistantProviderRejected,
};
pub use updates::{
    AssistantProviderUpdate, AssistantProviderUpdateApplied, AssistantProviderUpdateExport,
    AssistantProviderUpdateRejected, AssistantProviderUpdateStatus,
};
redacted!(
    AssistantProviderContext,
    AssistantProviderRead,
    AssistantProviderAttempt,
    AssistantProviderCreated,
    AssistantProviderExport,
    AssistantProviderApplied
);
impl AssistantProviderContext {
    pub fn delivery_id(&self) -> &str {
        &self.delivery
    }
    pub fn connection_id(&self) -> &str {
        self.origin.connection_id()
    }
    pub fn generation(&self) -> u64 {
        self.origin.generation()
    }
    pub fn timing(&self) -> AssistantProviderTiming {
        if self.initialization {
            AssistantProviderTiming::Initialization
        } else {
            AssistantProviderTiming::AfterCreation
        }
    }
}
impl AssistantProviderRead {
    pub(crate) fn observation_interval(
        &self,
        context: &AssistantProviderContext,
    ) -> Result<&Arc<AuthorityGuardSet>> {
        if !self.current()
            || self.context.delivery != context.delivery
            || self.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        Ok(self.read.native.interval())
    }
    pub fn target(&self) -> &PeerConnectionProjection {
        self.read.target()
    }
    pub fn current(&self) -> bool {
        self.read.current()
    }
    pub fn expires_at(&self) -> i64 {
        self.read.expires_at()
    }
    pub fn provider_endpoint(&self) -> Result<&Value> {
        self.read.provider_endpoint()
    }
    pub fn provider_ref(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.provider_ref.as_deref().ok_or_else(|| {
            Error::new(
                "not-ready",
                "Provider creation must be captured before export",
            )
        })
    }
    pub fn context_json(&self) -> Result<&str> {
        if !self.initialization_admitted {
            self.provider_ref()?;
        }
        self.read.context_json()
    }
}
impl AssistantProviderAttempt {
    /// Capture only after the exact admitted creation returns its actual reference.
    pub fn created(&self, reference: String) -> Result<AssistantProviderCreated> {
        if reference.is_empty()
            || reference.len() > 512
            || !reference
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(Error::new("invalid-request", "Invalid provider reference"));
        }
        Ok(AssistantProviderCreated {
            context: self.context.clone(),
            reference,
        })
    }
}
impl AssistantProviderCreated {
    pub fn provider_ref(&self) -> &str {
        &self.reference
    }
}
impl AssistantProviderExport {
    /// The trusted transport must supply the provider's exact observed system
    /// message. A socket write, truncated body or status is not acknowledgment.
    pub fn acknowledged(&self, body: &str) -> Result<AssistantProviderApplied> {
        if body != self.json {
            return Err(Error::new(
                "context-ack-mismatch",
                "Provider did not acknowledge the exact context",
            ));
        }
        Ok(AssistantProviderApplied {
            context: self.context.clone(),
            reference: self.reference.clone(),
        })
    }
}
impl AssistantProviderApplied {
    pub fn delivery_id(&self) -> &str {
        &self.context.delivery
    }
    pub fn provider_ref(&self) -> &str {
        &self.reference
    }
}
fn bound(c: &Connection, context: &AssistantProviderContext) -> Result<()> {
    let same:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_providers p WHERE delivery_id=?1 AND attempt_id=?2 AND context_credential_id=?3 AND connection_id=?4 AND generation=?5 AND endpoint_id=?6 AND runtime_epoch=?7 AND EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=p.delivery_id)=?8)",params![context.delivery,context.attempt,context.credential,context.origin.connection_id(),context.origin.generation(),context.endpoint,context.runtime,context.initialization],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !same {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn progress(c: &Connection, id: &str, store: &Store) -> Result<()> {
    let (cid,raw,state,reference,problem,lifecycle):(String,String,String,Option<String>,Option<String>,String)=c.query_row("SELECT d.conversation_id,p.audience_json,d.state,d.provider_ref,p.problem,json_object('phase',CASE WHEN EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=p.delivery_id) THEN 'initialization' ELSE 'post_creation' END,'parent_delivery_id',NULL,'creation_started',p.creation_started,'created',p.created_at IS NOT NULL,'export_started',(p.export_started=1 OR EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=p.delivery_id AND i.started=1)),'delivered',p.delivered_at IS NOT NULL,'cleanup_started',p.end_started,'closed',p.closed_at IS NOT NULL,'cleanup_unknown',p.end_uncertain) FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1 UNION ALL SELECT d.conversation_id,p.audience_json,d.state,d.provider_ref,u.problem,json_object('phase','update','parent_delivery_id',p.delivery_id,'creation_started',0,'created',p.created_at IS NOT NULL,'export_started',u.export_started,'delivered',u.delivered_at IS NOT NULL,'cleanup_started',p.end_started,'closed',p.closed_at IS NOT NULL,'cleanup_unknown',p.end_uncertain) FROM native_assistant_context_updates u JOIN native_assistant_providers p ON p.delivery_id=u.instance_delivery_id JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.delivery_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|_|Error::storage())?;
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    if !crate::conversations::capacity(c, audience.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Assistant context journal capacity exceeded",
        ));
    }
    for (identity, pid, epoch) in audience {
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,'context_delivery',?4,?5,?6)",params![format!("evt_{}",uuid::Uuid::new_v4()),identity,cid,serde_json::json!({"source_kind":"context_delivery","source_id":id,"state":state,"provider_ref":reference,"problem":problem,"provider_lifecycle":serde_json::from_str::<Value>(&lifecycle).map_err(|_|Error::storage())?}).to_string(),pid,epoch]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
fn original_runtime(store: &Store, context: &AssistantProviderContext) -> Result<()> {
    let owner = store.require_runtime_owner()?;
    if context.runtime != store.runtime_epoch()
        || !owner.current()
        || !context
            .origin
            .owner
            .upgrade()
            .is_some_and(|old| Arc::ptr_eq(&old, &owner))
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn memory_deadline(c: &Connection, context: &AssistantProviderContext) -> Result<Option<i64>> {
    let mut q=c.prepare("SELECT d.selection_json FROM context_deliveries d WHERE d.id=?1 UNION ALL SELECT d.selection_json FROM native_assistant_context_updates u JOIN context_deliveries d ON d.id=u.delivery_id WHERE u.instance_delivery_id=?1 AND u.export_started=1").map_err(|_|Error::storage())?;
    let rows = q
        .query_map([&context.delivery], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut deadline = None;
    for row in rows {
        if let Some(until) = crate::records::context::encoded_deadline(&row)? {
            deadline = Some(deadline.map_or(until, |old: i64| old.min(until)));
        }
    }
    Ok(deadline)
}
impl Store {
    /// Resolve captured provider memory in the caller's current SQL snapshot.
    /// This acquires no second reader behind a queued writer. The observation
    /// layer acquires/reuses the complete returned scopes before publication.
    pub(crate) fn native_assistant_observation_facts(
        &self,
        c: &Connection,
        context: &AssistantProviderContext,
        applied: &AssistantProviderApplied,
    ) -> Result<crate::events::observations::memory::Facts> {
        original_runtime(self, context)?;
        bound(c, context)?;
        if applied.context.delivery != context.delivery
            || applied.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let (json, endpoint, audience): (String, String, String) = c.query_row(
            "SELECT d.selection_json,p.endpoint_json,p.audience_json FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1 AND p.provider_ref=?2 AND p.uncertain=0 AND p.rejected_at IS NULL AND p.end_started=0 AND p.closed_at IS NULL AND p.delivered_at IS NOT NULL AND d.state='delivered' AND d.provider_ref=p.provider_ref",
            params![context.delivery, applied.reference],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).map_err(|_| Error::forbidden())?;
        let facts = resolve(
            self,
            c,
            &context.origin,
            &context.credential,
            &context.endpoint,
        )?;
        if crate::now() >= facts.expires
            || endpoint != crate::provisioning::canonical_json(&facts.endpoint)?
            || audience != serde_json::to_string(&facts.audience).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        updates::validate_saved(self, c, &facts, context, &json)?;
        updates::validate_rows(self, c, &facts, context, None)?;
        if crate::now() >= facts.expires {
            return Err(Error::new(
                "authority-busy",
                "Observation memory interval expired during validation",
            ));
        }
        let expires =
            memory_deadline(c, context)?.map_or(facts.expires, |until| facts.expires.min(until));
        if crate::now() >= expires {
            return Err(Error::forbidden());
        }
        let target = facts.view.connection;
        Ok(crate::events::observations::memory::Facts {
            cid: target.conversation_id,
            sid: target.session_id,
            connection: target.connection_id,
            pid: target.participant_id,
            epoch: target.membership_epoch,
            generation: target.generation,
            audience: facts
                .audience
                .into_iter()
                .map(|(_, pid, epoch)| (pid, epoch))
                .collect(),
            scopes: facts.scopes,
            expires_at: expires,
        })
    }
    /// Reserve before any provider IO. Drop the selection's read interval before
    /// taking metadata writers, then resolve again under the complete writer.
    pub fn prepare_native_assistant_provider(
        &self,
        operation_id: &str,
        read: NativeAssistantContextRead,
    ) -> Result<AssistantProviderContext> {
        self.prepare_native_assistant_provider_timed(
            operation_id,
            read,
            AssistantProviderTiming::AfterCreation,
        )
    }
    pub fn prepare_native_assistant_provider_timed(
        &self,
        operation_id: &str,
        read: NativeAssistantContextRead,
        timing: AssistantProviderTiming,
    ) -> Result<AssistantProviderContext> {
        if operation_id.is_empty()
            || operation_id.len() > 128
            || !operation_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid assistant provider operation ID",
            ));
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        let origin = read.origin.clone();
        let credential = read.credential.clone();
        let endpoint = read.endpoint_id.clone();
        let routing = read.routing.clone();
        drop(read);
        let held =
            self.runtime_conversation_authority(|_| Ok(vec![origin.conversation_id().into()]))?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let facts = resolve(self, &tx, &origin, &credential, &endpoint)?;
        if let Some(routing) = &routing {
            check_route(
                self,
                &tx,
                &origin,
                &facts.actor,
                &facts.endpoint,
                &endpoint,
                routing,
            )?;
        }
        if !self.authority_covers(&held, &facts.scopes) {
            return Err(Error::forbidden());
        }
        let existing:Option<(String,String,String,u64,String,String,String)>=tx.query_row("SELECT delivery_id,attempt_id,connection_id,generation,endpoint_id,runtime_epoch,context_credential_id FROM native_assistant_providers WHERE owner_id=?1 AND operation_id=?2",params![facts.actor.identity_id,operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|_|Error::storage())?;
        let initialization = timing == AssistantProviderTiming::Initialization;
        if let Some((
            delivery,
            attempt,
            connection,
            generation,
            saved_endpoint,
            runtime,
            saved_credential,
        )) = existing
        {
            let saved_initialization:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_initializations WHERE delivery_id=?1)",[&delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
            if saved_credential != credential
                || connection != origin.connection_id()
                || generation != origin.generation()
                || saved_endpoint != endpoint
                || runtime != self.runtime_epoch()
                || saved_initialization != initialization
            {
                return Err(Error::new(
                    "conflict",
                    "Logical assistant request is already bound to another destination",
                ));
            }
            return Ok(AssistantProviderContext {
                origin,
                credential,
                endpoint,
                delivery,
                attempt,
                runtime,
                initialization,
                routing,
            });
        }
        let used_update:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_assistant_context_updates WHERE owner_id=?1 AND operation_id=?2)",params![facts.actor.identity_id,operation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if used_update {
            return Err(Error::new(
                "conflict",
                "Logical operation is already a context update",
            ));
        }
        let view = crate::context::view_for_audience_scope_at(
            self,
            &tx,
            &facts.actor,
            origin.conversation_id(),
            1000,
            Some(&facts.delegate.identity_id),
            Some(&facts.delegate),
            None,
            None,
            Some(&facts.audience),
        )?;
        let encoded = serde_json::to_string(&view).map_err(|_| Error::storage())?;
        if encoded.len() > 65_536 {
            return Err(Error::new(
                "capacity-exceeded",
                "Assistant context exceeds its byte budget",
            ));
        }
        let context = AssistantProviderContext {
            origin,
            credential,
            endpoint,
            delivery: format!("actx_{}", uuid::Uuid::new_v4()),
            attempt: format!("aca_{}", uuid::Uuid::new_v4()),
            runtime: self.runtime_epoch().into(),
            initialization,
            routing,
        };
        tx.execute("INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence,owner_id) VALUES(?1,?2,?3,'vapi',?4,?5,'pending',?6,?6,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),?7)",params![context.delivery,context.origin.conversation_id(),context.origin.session_id(),view.watermark,encoded,crate::now(),facts.actor.identity_id]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_assistant_providers(delivery_id,operation_id,context_credential_id,connection_id,generation,runtime_epoch,endpoint_id,endpoint_json,audience_json,attempt_id,owner_id,prepared_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![context.delivery,operation_id,context.credential,context.origin.connection_id(),context.origin.generation(),self.runtime_epoch(),context.endpoint,crate::provisioning::canonical_json(&facts.endpoint)?,serde_json::to_string(&facts.audience).map_err(|_|Error::storage())?,context.attempt,facts.actor.identity_id,facts.expires]).map_err(|_|Error::storage())?;
        if initialization {
            tx.execute(
                "INSERT INTO native_assistant_initializations(delivery_id) VALUES(?1)",
                [&context.delivery],
            )
            .map_err(|_| Error::storage())?;
        }
        for (_, pid, epoch) in &facts.audience {
            tx.execute("INSERT OR IGNORE INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch) VALUES('context_delivery',?1,?2,?3,?4)",params![context.delivery,context.origin.conversation_id(),pid,epoch]).map_err(|_|Error::storage())?;
        }
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(context)
    }
    pub fn try_native_assistant_provider_read(
        &self,
        context: &AssistantProviderContext,
    ) -> Result<AssistantProviderRead> {
        self.native_assistant_provider_read(context, None, None)
    }
    pub fn try_native_assistant_memory_read(
        &self,
        context: &AssistantProviderContext,
        receipt: &AssistantProviderApplied,
    ) -> Result<AssistantProviderRead> {
        if receipt.context.delivery != context.delivery
            || receipt.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        self.native_assistant_provider_read(context, Some(&receipt.reference), None)
    }
    fn native_assistant_provider_read(
        &self,
        context: &AssistantProviderContext,
        memory: Option<&str>,
        allow_update: Option<&str>,
    ) -> Result<AssistantProviderRead> {
        original_runtime(self, context)?;
        let c = self.try_connection()?;
        bound(&c, context)?;
        let (json,endpoint,audience,reference):(String,String,String,Option<String>)=c.query_row("SELECT d.selection_json,p.endpoint_json,p.audience_json,p.provider_ref FROM native_assistant_providers p JOIN context_deliveries d ON d.id=p.delivery_id WHERE p.delivery_id=?1 AND p.uncertain=0 AND p.rejected_at IS NULL AND p.end_started=0 AND p.closed_at IS NULL AND ((?2 IS NULL AND d.state='pending' AND p.export_started=0 AND (p.creation_started=0 OR p.created_at IS NOT NULL)) OR (?2=p.provider_ref AND p.delivered_at IS NOT NULL AND d.state='delivered'))",params![context.delivery,memory],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::forbidden())?;
        drop(c);
        let saved: connect_protocol::ContextView =
            serde_json::from_str(&json).map_err(|_| Error::storage())?;
        let mut read = self.try_native_assistant_context_read_at(
            &context.origin,
            &context.credential,
            &context.endpoint,
            Some(saved.watermark),
        )?;
        // Repeat initial routing policy under the exact current native read
        // before possible creation. Availability hints do not erase an owned call.
        if memory.is_none() && reference.is_none() {
            if let Some(routing) = &context.routing {
                let c = self.try_connection()?;
                let actor = crate::identity::actor_from_connection(&c, &context.credential)?;
                check_route(
                    self,
                    &c,
                    &context.origin,
                    &actor,
                    &read.endpoint,
                    &context.endpoint,
                    routing,
                )?;
            }
        }
        if endpoint != crate::provisioning::canonical_json(&read.endpoint)?
            || audience != serde_json::to_string(&read.audience).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        if serde_json::from_str::<Value>(&read.json).map_err(|_| Error::storage())?
            != serde_json::to_value(saved).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        read.json = json;
        if memory.is_some() {
            self.validate_assistant_updates(&context, &read, allow_update)?;
        }
        let c = self.try_connection()?;
        if let Some(until) = memory_deadline(&c, context)? {
            read.native.expires_at = read.native.expires_at.min(until);
        }
        if let Some(until) = crate::records::context::encoded_deadline(&read.json)? {
            read.native.expires_at = read.native.expires_at.min(until);
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(AssistantProviderRead {
            context: context.clone(),
            read,
            provider_ref: reference,
            initialization_admitted: false,
            update: None,
        })
    }
    pub fn begin_native_assistant_provider(
        &self,
        context: AssistantProviderContext,
        read: AssistantProviderRead,
    ) -> Result<(AssistantProviderAttempt, AssistantProviderRead)> {
        if context.initialization
            || read.update.is_some()
            || !read.current()
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
            || read.provider_ref.is_some()
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, &context)?;
        if tx.execute("UPDATE native_assistant_providers SET creation_started=1,creation_expires_at=?2 WHERE delivery_id=?1 AND creation_started=0 AND uncertain=0 AND closed_at IS NULL",params![context.delivery,read.expires_at()]).map_err(|_|Error::storage())?!=1 {return Err(Error::new("reconciliation-required","Provider creation was already attempted"));}
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((AssistantProviderAttempt { context }, read))
    }
    pub fn record_native_assistant_provider_created(
        &self,
        receipt: &AssistantProviderCreated,
    ) -> Result<()> {
        let context = &receipt.context;
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        let reference:Option<String>=tx.query_row("SELECT provider_ref FROM native_assistant_providers WHERE delivery_id=?1 AND creation_started=1",[&context.delivery],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if let Some(saved) = reference {
            if saved == receipt.reference {
                return Ok(());
            }
            return Err(Error::new("conflict", "Provider reference changed"));
        }
        crate::context::require_fresh_provider_reference(
            &tx,
            &receipt.reference,
            &context.delivery,
        )?;
        tx.execute("UPDATE native_assistant_providers SET provider_ref=?2,created_at=?3,export_started=CASE WHEN EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=?1 AND i.started=1) THEN 1 ELSE export_started END,export_expires_at=CASE WHEN EXISTS(SELECT 1 FROM native_assistant_initializations i WHERE i.delivery_id=?1 AND i.started=1) THEN (SELECT expires_at FROM native_assistant_initializations WHERE delivery_id=?1) ELSE export_expires_at END WHERE delivery_id=?1",params![context.delivery,receipt.reference,crate::now()]).map_err(|_|Error::storage())?;
        tx.execute(
            "UPDATE context_deliveries SET provider_ref=?2,updated_at=?3 WHERE id=?1",
            params![context.delivery, receipt.reference, crate::now()],
        )
        .map_err(|_| Error::storage())?;
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn begin_native_assistant_export(
        &self,
        context: AssistantProviderContext,
        read: AssistantProviderRead,
    ) -> Result<(AssistantProviderExport, AssistantProviderRead)> {
        if context.initialization
            || read.update.is_some()
            || !read.current()
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let reference = read.provider_ref()?.to_owned();
        let json = read.context_json()?.to_owned();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, &context)?;
        if tx.execute("UPDATE native_assistant_providers SET export_started=1,export_expires_at=?2 WHERE delivery_id=?1 AND export_started=0 AND uncertain=0 AND provider_ref=?3 AND end_started=0 AND closed_at IS NULL",params![context.delivery,read.expires_at(),reference]).map_err(|_|Error::storage())?!=1 {return Err(Error::new("reconciliation-required","Context export was already attempted"));}
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            AssistantProviderExport {
                context,
                reference,
                json,
            },
            read,
        ))
    }
    pub fn record_native_assistant_delivered(
        &self,
        receipt: &AssistantProviderApplied,
    ) -> Result<()> {
        let context = &receipt.context;
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        let delivered:Option<i64>=tx.query_row("SELECT delivered_at FROM native_assistant_providers WHERE delivery_id=?1 AND provider_ref=?2 AND export_started=1 AND rejected_at IS NULL",params![context.delivery,receipt.reference],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if delivered.is_some() {
            return Ok(());
        }
        if tx.execute("UPDATE native_assistant_providers SET delivered_at=COALESCE(delivered_at,?2) WHERE delivery_id=?1 AND provider_ref=?3 AND export_started=1",params![context.delivery,crate::now(),receipt.reference]).map_err(|_|Error::storage())?!=1{return Err(Error::forbidden());}
        tx.execute("UPDATE context_deliveries SET state='delivered',provider_ref=?2,updated_at=?3 WHERE id=?1 AND state IN ('pending','uncertain')",params![context.delivery,receipt.reference,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn native_assistant_creation_uncertain(
        &self,
        attempt: &AssistantProviderAttempt,
    ) -> Result<()> {
        self.assistant_uncertain(&attempt.context)
    }
    pub fn native_assistant_export_uncertain(
        &self,
        export: &AssistantProviderExport,
    ) -> Result<()> {
        self.assistant_uncertain(&export.context)
    }
    pub fn native_assistant_export_incomplete(
        &self,
        export: &AssistantProviderExport,
    ) -> Result<()> {
        self.assistant_uncertain_problem(&export.context, "incomplete")
    }
    fn assistant_uncertain(&self, context: &AssistantProviderContext) -> Result<()> {
        self.assistant_uncertain_problem(context, "unknown")
    }
    fn assistant_uncertain_problem(
        &self,
        context: &AssistantProviderContext,
        problem: &str,
    ) -> Result<()> {
        original_runtime(self, context)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(&tx, context)?;
        if tx.execute("UPDATE native_assistant_providers SET uncertain=1,problem=?2 WHERE delivery_id=?1 AND creation_started=1 AND delivered_at IS NULL",params![context.delivery,problem]).map_err(|_|Error::storage())?!=1{return Err(Error::forbidden());}
        tx.execute("UPDATE context_deliveries SET state='uncertain',updated_at=?2 WHERE id=?1 AND state='pending'",params![context.delivery,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &context.delivery, self)?;
        tx.commit().map_err(|_| Error::storage())
    }
}
