//! Connection-bound provider initialization and original private provenance.
//! These are trusted runtime APIs; no caller-selected provider memory is reused.
use crate::{
    Actor, AuthorityGuardSet, AuthorityMode, AuthorityScope, InstallationOwnership,
    NativeConsultationRead, NativeConsultationSelection, Store,
};
use connect_protocol::{ContextView, Error, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::sync::{Arc, Weak};
mod authority;
use authority::*;
pub(crate) use authority::{ready, require_for_native};
mod cache;
pub(crate) use cache::{ready_until, Cache};
mod media;
pub use media::PrivateProviderMediaRead;
mod observations;
pub(crate) use observations::recover_one;

#[derive(Clone)]
pub struct PrivateProviderContext {
    selection: NativeConsultationSelection,
    context_credential: String,
    endpoint: String,
    delivery: String,
    attempt: String,
}
pub struct PrivateProviderRead {
    context: PrivateProviderContext,
    _held: Arc<AuthorityGuardSet>,
    owner: Arc<InstallationOwnership>,
    json: String,
    endpoint: Value,
    expires_at: i64,
    provider_ref: Option<String>,
}
pub struct PrivateProviderAttempt {
    context: PrivateProviderContext,
}
pub struct PrivateProviderCreated {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    delivery: String,
    attempt: String,
    provider_ref: String,
}
pub struct PrivateProviderExport {
    context: PrivateProviderContext,
    provider_ref: String,
}
pub struct PrivateProviderApplied {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    delivery: String,
    attempt: String,
    provider_ref: String,
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
redacted!(
    PrivateProviderContext,
    PrivateProviderRead,
    PrivateProviderAttempt,
    PrivateProviderCreated,
    PrivateProviderExport,
    PrivateProviderApplied
);
impl PrivateProviderApplied {
    pub fn provider_ref(&self) -> &str {
        &self.provider_ref
    }
}
impl PrivateProviderContext {
    pub fn delivery_id(&self) -> &str {
        &self.delivery
    }
    pub fn consultation_record_id(&self) -> &str {
        self.selection.origin.record_id()
    }
}
impl PrivateProviderRead {
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires_at
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn json(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        if self.provider_ref.is_none() {
            return Err(Error::new(
                "not-ready",
                "Provider creation must be recorded before private context export",
            ));
        }
        Ok(&self.json)
    }
    pub fn provider_ref(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.provider_ref
            .as_deref()
            .ok_or_else(|| Error::new("not-ready", "Provider creation is not recorded"))
    }
    pub fn assistant_id(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.endpoint["address"].as_str().ok_or_else(Error::storage)
    }
    pub fn secret_ref(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.endpoint["secret_ref"]
            .as_str()
            .ok_or_else(Error::storage)
    }
}
impl PrivateProviderAttempt {
    /// Only after actual fresh provider creation, before any private export.
    pub fn created(self, provider_ref: String) -> Result<PrivateProviderCreated> {
        if provider_ref.is_empty()
            || provider_ref.len() > 512
            || provider_ref.chars().any(char::is_control)
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid private provider reference",
            ));
        }
        let origin = &self.context.selection.origin;
        Ok(PrivateProviderCreated {
            owner: origin.owner.clone(),
            runtime: origin.runtime.clone(),
            delivery: self.context.delivery,
            attempt: self.context.attempt,
            provider_ref,
        })
    }
}
impl PrivateProviderExport {
    /// Actual successful export to the already captured fresh provider instance.
    /// No caller-selected reference can redirect this observation.
    pub fn delivered(self) -> PrivateProviderApplied {
        let origin = &self.context.selection.origin;
        PrivateProviderApplied {
            owner: origin.owner.clone(),
            runtime: origin.runtime.clone(),
            delivery: self.context.delivery,
            attempt: self.context.attempt,
            provider_ref: self.provider_ref,
        }
    }
}
impl Store {
    /// Persist the exact private selection and configured verified Vapi target
    /// before provider IO. The consumed media planning reader is dropped before
    /// metadata writers are acquired. This operation does not contact a provider.
    pub fn prepare_private_provider_context(
        &self,
        read: NativeConsultationRead,
        context_credential: &str,
        endpoint: &str,
    ) -> Result<PrivateProviderContext> {
        let selection = read.into_selection()?;
        let context = PrivateProviderContext {
            selection,
            context_credential: context_credential.into(),
            endpoint: endpoint.into(),
            delivery: format!("pctx_{}", uuid::Uuid::new_v4()),
            attempt: format!("pca_{}", uuid::Uuid::new_v4()),
        };
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                scopes(self, &c, &context, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !self.authority_covers(&held, &scopes(self, &tx, &context, true)?) {
            return Err(Error::forbidden());
        }
        let facts = resolve(self, &tx, &context)?;
        let existing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts WHERE operation_record_id=?1)",[context.consultation_record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if existing {
            return Err(Error::new(
                "conflict",
                "Private provider context is already reserved",
            ));
        }
        let view = crate::context::view_for_private_scope_at(
            self,
            &tx,
            &facts.actor,
            &facts.conversation,
            1000,
            Some(&facts.delegate.identity_id),
            Some(&facts.delegate),
            None,
            Some(context.consultation_record_id()),
        )?;
        let encoded = serde_json::to_string(&view).map_err(|_| Error::storage())?;
        if encoded.len() > 65536 {
            return Err(Error::new(
                "capacity-exceeded",
                "Private provider context exceeds its byte budget",
            ));
        }
        tx.execute("INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence,owner_id) VALUES(?1,?2,?3,'vapi',?4,?5,'pending',?6,?6,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),?7)",params![context.delivery,facts.conversation,context.selection.origin.request.session_id,view.watermark,encoded,crate::now(),facts.actor.identity_id]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO private_provider_contexts(delivery_id,operation_record_id,connection_id,runtime_epoch,source_digest,selection_json,audience_json,context_credential_id,delegate_credential_id,endpoint_id,endpoint_json,attempt_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![context.delivery,context.consultation_record_id(),facts.candidate,self.runtime_epoch(),context.selection.origin.digest,crate::provisioning::canonical_json(&context.selection.snapshot())?,facts.audience_json,context.context_credential,facts.delegate.credential_id,context.endpoint,crate::provisioning::canonical_json(&facts.endpoint)?,context.attempt]).map_err(|_|Error::storage())?;
        for (_, pid, epoch) in &facts.audience {
            tx.execute("INSERT OR IGNORE INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch) VALUES('context_delivery',?1,?2,?3,?4)",params![context.delivery,facts.conversation,pid,epoch]).map_err(|_|Error::storage())?;
        }
        tx.execute("INSERT INTO private_content_sources(kind,source_id,conversation_id,operation_record_id,connection_id) VALUES('context_delivery',?1,?2,?3,?4)",params![context.delivery,facts.conversation,context.consultation_record_id(),facts.candidate]).map_err(|_|Error::storage())?;
        progress(&tx, context.consultation_record_id(), self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(context)
    }
    pub fn try_private_provider_read(
        &self,
        context: &PrivateProviderContext,
    ) -> Result<PrivateProviderRead> {
        let needed = {
            let c = self.try_connection()?;
            bound(self, &c, context)?;
            scopes(self, &c, context, false)?
        };
        let held = self.try_acquire_authority(&needed)?;
        let c = self.try_connection()?;
        self.private_provider_read_at(&c, context, held)
    }
    /// Bounded metadata admission on a blocking host worker. Unlike packet
    /// policy this queues for SQL and authority, then repeats every current
    /// selection/audience check. It never starts provider creation or export.
    pub fn private_provider_read(
        &self,
        context: &PrivateProviderContext,
        wait: &crate::AuthorityWait,
    ) -> Result<PrivateProviderRead> {
        wait.check()?;
        let held = self.stable_authority(wait, || {
            let c = self.connection()?;
            wait.check()?;
            bound(self, &c, context)?;
            scopes(self, &c, context, false)
        })?;
        let c = self.connection()?;
        wait.check()?;
        let read = self.private_provider_read_at(&c, context, held)?;
        wait.check()?;
        Ok(read)
    }
    fn private_provider_read_at(
        &self,
        c: &Connection,
        context: &PrivateProviderContext,
        held: Arc<AuthorityGuardSet>,
    ) -> Result<PrivateProviderRead> {
        bound(self, c, context)?;
        if !self.authority_covers(&held, &scopes(self, c, context, false)?) {
            return Err(Error::forbidden());
        }
        let facts = resolve(self, c, context)?;
        let (encoded,provider_ref):(String,Option<String>) = c
            .query_row(
                "SELECT d.selection_json,p.provider_ref FROM context_deliveries d JOIN private_provider_contexts p ON p.delivery_id=d.id WHERE d.id=?1 AND d.state='pending' AND p.uncertain=0 AND p.export_started=0 AND p.closed_at IS NULL AND (p.started=0 OR p.created_at IS NOT NULL)",
                [&context.delivery],
                |r| Ok((r.get(0)?,r.get(1)?)),
            )
            .map_err(|_| Error::forbidden())?;
        let view: ContextView = serde_json::from_str(&encoded).map_err(|_| Error::storage())?;
        let current = crate::context::view_for_private_scope_at(
            self,
            c,
            &facts.actor,
            &facts.conversation,
            1000,
            Some(&facts.delegate.identity_id),
            Some(&facts.delegate),
            Some(view.watermark),
            Some(context.consultation_record_id()),
        )?;
        if serde_json::to_value(&current).map_err(|_| Error::storage())?
            != serde_json::to_value(&view).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        for item in &view.items {
            crate::context::authorize_private_export_event(
                c,
                &facts.actor,
                &facts.delegate.identity_id,
                Some(&facts.delegate),
                &facts.conversation,
                item.sequence,
                Some(context.consultation_record_id()),
            )?;
        }
        let owner = self.require_runtime_owner()?;
        let expires_at = crate::records::context::deadline(&view)
            .map_or(facts.expires_at, |until| facts.expires_at.min(until))
            .min(crate::now() + 5);
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        Ok(PrivateProviderRead {
            context: context.clone(),
            _held: held,
            owner,
            json: encoded,
            endpoint: facts.endpoint,
            expires_at,
            provider_ref,
        })
    }
    pub fn begin_private_provider_context(
        &self,
        context: PrivateProviderContext,
        read: PrivateProviderRead,
    ) -> Result<(PrivateProviderAttempt, PrivateProviderRead)> {
        if !read.current()
            || read.provider_ref.is_some()
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(self, &tx, &context)?;
        resolve(self, &tx, &context)?;
        if tx.execute("UPDATE private_provider_contexts SET started=1,creation_expires_at=?2 WHERE delivery_id=?1 AND started=0 AND uncertain=0 AND delivered_at IS NULL",params![context.delivery,read.expires_at]).map_err(|_|Error::storage())?!=1{return Err(Error::new("reconciliation-required","Private provider initialization already attempted"));}
        tx.execute(
            "UPDATE operations SET state='preparing' WHERE record_id=?1 AND state='accepted'",
            [context.consultation_record_id()],
        )
        .map_err(|_| Error::storage())?;
        progress(&tx, context.consultation_record_id(), self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((PrivateProviderAttempt { context }, read))
    }
    /// A fresh provider reference is durably reserved before private JSON can
    /// be exported. Metadata recording never creates or resumes a provider call.
    pub fn record_private_provider_created(&self, receipt: &PrivateProviderCreated) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|o| Arc::ptr_eq(&o, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,Option<String>)>=tx.query_row("SELECT operation_record_id,provider_ref FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND started=1",params![receipt.delivery,receipt.attempt,receipt.runtime],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (root, existing) = row.ok_or_else(Error::forbidden)?;
        if let Some(existing) = existing {
            if existing != receipt.provider_ref {
                return Err(Error::new(
                    "conflict",
                    "Private provider creation observation changed",
                ));
            }
            return Ok(());
        }
        crate::context::require_fresh_provider_reference(
            &tx,
            &receipt.provider_ref,
            &receipt.delivery,
        )?;
        tx.execute("UPDATE private_provider_contexts SET provider_ref=?2,created_at=?3 WHERE delivery_id=?1 AND provider_ref IS NULL",params![receipt.delivery,receipt.provider_ref,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &root, self)?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn begin_private_provider_export(
        &self,
        context: PrivateProviderContext,
        read: PrivateProviderRead,
    ) -> Result<(PrivateProviderExport, PrivateProviderRead)> {
        if !read.current()
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let provider_ref = read.provider_ref.clone().ok_or_else(Error::forbidden)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        bound(self, &tx, &context)?;
        resolve(self, &tx, &context)?;
        if tx.execute("UPDATE private_provider_contexts SET export_started=1,export_expires_at=?3 WHERE delivery_id=?1 AND started=1 AND provider_ref=?2 AND created_at IS NOT NULL AND export_started=0 AND export_uncertain=0 AND uncertain=0 AND closed_at IS NULL AND delivered_at IS NULL",params![context.delivery,provider_ref,read.expires_at]).map_err(|_|Error::storage())?!=1{return Err(Error::new("reconciliation-required","Private provider export already attempted"));}
        progress(&tx, context.consultation_record_id(), self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            PrivateProviderExport {
                context,
                provider_ref,
            },
            read,
        ))
    }
    /// Record an actual context ACK after dropping export readers. This token
    /// cannot attach media, export a body or change its captured provider target.
    pub fn record_private_provider_delivered(
        &self,
        receipt: &PrivateProviderApplied,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|o| Arc::ptr_eq(&o, &owner))
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row:Option<(String,Option<i64>)>=tx.query_row("SELECT operation_record_id,delivered_at FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3 AND started=1 AND export_started=1 AND provider_ref=?4 AND created_at IS NOT NULL",params![receipt.delivery,receipt.attempt,receipt.runtime,receipt.provider_ref],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (root, delivered) = row.ok_or_else(Error::forbidden)?;
        if delivered.is_some() {
            return Ok(());
        }
        tx.execute("UPDATE private_provider_contexts SET delivered_at=?2 WHERE delivery_id=?1 AND delivered_at IS NULL",params![receipt.delivery,crate::now()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE context_deliveries SET state='delivered',provider_ref=?2,updated_at=?3 WHERE id=?1 AND state IN ('pending','uncertain')",params![receipt.delivery,receipt.provider_ref,crate::now()]).map_err(|_|Error::storage())?;
        progress(&tx, &root, self)?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
}
