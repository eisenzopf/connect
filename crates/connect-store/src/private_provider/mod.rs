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
pub(crate) use observations::{recover_one, recovery_disposition};

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
    mutation_hint: Vec<AuthorityScope>,
    owner: Arc<InstallationOwnership>,
    json: String,
    endpoint: Value,
    expires_at: i64,
    provider_ref: Option<String>,
}
pub struct PrivateProviderAttempt {
    context: PrivateProviderContext,
    expires_at: i64,
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
    expires_at: i64,
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
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
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
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
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
#[derive(Clone, Copy)]
enum ReadPhase {
    Planning,
    Creation { until: i64 },
    Export { until: i64 },
}
impl ReadPhase {
    fn until(self) -> i64 {
        match self {
            Self::Planning => i64::MAX,
            Self::Creation { until } | Self::Export { until } => until,
        }
    }
}
// Bounded static diagnostics distinguish file admission, primary SQL and fresh
// policy work without logging credentials, payloads, resource IDs or SQL text.
fn admission_stage<T>(
    phase: &'static str,
    stage: &'static str,
    work: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let started = std::time::Instant::now();
    let result = work();
    tracing::debug!(target: "connect_store::private_provider::admission", phase, stage,
        elapsed_us = started.elapsed().as_micros() as u64,
        code = result.as_ref().err().map_or("ok", |error| error.code.as_str()),
        "Private provider admission stage completed");
    result
}
impl Store {
    fn private_provider_history_sql<'a>(
        &'a self,
        delivery: &str,
        attempt: &str,
        runtime: &str,
    ) -> Result<(
        Arc<AuthorityGuardSet>,
        crate::installation_ownership::ConnectionGuard<'a>,
    )> {
        let needed = {
            let c = self.try_connection()?;
            let root: String = c.query_row("SELECT operation_record_id FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3", params![delivery, attempt, runtime], |row| row.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
            mutation_scopes(&c, &root)?
        };
        let held = self.try_acquire_authority(&needed)?;
        let c = self.try_connection()?;
        c.bind_available_mutation_authority(self, &held)?;
        Ok((held, c))
    }
    fn require_private_provider_history(
        &self,
        c: &Connection,
        held: &AuthorityGuardSet,
        delivery: &str,
        attempt: &str,
        runtime: &str,
    ) -> Result<()> {
        let root: String = c.query_row("SELECT operation_record_id FROM private_provider_contexts WHERE delivery_id=?1 AND attempt_id=?2 AND runtime_epoch=?3", params![delivery, attempt, runtime], |row| row.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        if !self.authority_covers(held, &mutation_scopes(c, &root)?) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
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
        c.bind_available_mutation_authority(self, &held)?;
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
            let c = admission_stage("planning", "read-initial-sql", || self.try_connection())?;
            admission_stage("planning", "read-initial-scopes", || {
                bound_scopes(self, &c, context, false)
            })?
        };
        let held = admission_stage("planning", "read-file-admission", || {
            self.try_acquire_authority(&needed)
        })?;
        let c = admission_stage("planning", "read-sql-reacquire", || self.try_connection())?;
        self.private_provider_read_at(&c, context, held, ReadPhase::Planning)
    }
    #[cfg(test)]
    pub(crate) fn private_provider_counted_read_for_test(
        &self,
        context: &PrivateProviderContext,
        original_repeated: bool,
    ) -> (Result<PrivateProviderRead>, usize) {
        authority::counted_resolves(context, || {
            if original_repeated {
                self.private_provider_original_repeated_read_for_test(context)
            } else {
                self.try_private_provider_read(context)
            }
        })
    }
    // Reconstruct the original outer bound+scopes / bound+scopes+resolve
    // read pipeline for an actual-SQL dependency comparison. Test-only and
    // read-only: the same real current policy, fences and phase validation run.
    #[cfg(test)]
    pub(crate) fn private_provider_original_repeated_read_for_test(
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
        bound(self, &c, context)?;
        if !self.authority_covers(&held, &scopes(self, &c, context, false)?) {
            return Err(Error::forbidden());
        }
        // The current leaf resolves fresh facts and preserves the original
        // current ContextView/phase checks. Prefixes above add back the actual
        // original repeated bound/scopes traversals, not synthetic SQL loops.
        self.private_provider_read_at(&c, context, held, ReadPhase::Planning)
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
            let c = admission_stage("planning", "read-wait-sql", || self.connection())?;
            wait.check()?;
            admission_stage("planning", "read-wait-scopes", || {
                bound_scopes(self, &c, context, false)
            })
        })?;
        let c = admission_stage("planning", "read-wait-final-sql", || self.connection())?;
        wait.check()?;
        let read = self.private_provider_read_at(&c, context, held, ReadPhase::Planning)?;
        wait.check()?;
        Ok(read)
    }
    fn private_provider_read_at(
        &self,
        c: &Connection,
        context: &PrivateProviderContext,
        held: Arc<AuthorityGuardSet>,
        phase: ReadPhase,
    ) -> Result<PrivateProviderRead> {
        self.private_provider_read_at_for(c, context, held, phase, false)
    }
    fn private_provider_read_at_for(
        &self,
        c: &Connection,
        context: &PrivateProviderContext,
        held: Arc<AuthorityGuardSet>,
        phase: ReadPhase,
        mutation: bool,
    ) -> Result<PrivateProviderRead> {
        let label = match phase {
            ReadPhase::Planning => "planning",
            ReadPhase::Creation { .. } => "creation-current",
            ReadPhase::Export { .. } => "export-current",
        };
        let facts = admission_stage(label, "read-current-facts", || resolve(self, c, context))?;
        bound_facts(self, c, context, &facts)?;
        if !self.authority_covers(
            &held,
            &scopes_for_facts(self, c, context, &facts, mutation)?,
        ) {
            return Err(if mutation {
                Error::new("authority-busy", "Private provider mutation scope changed")
            } else {
                Error::forbidden()
            });
        }
        let phase_predicate = match phase {
            ReadPhase::Planning => "p.export_started=0 AND (p.started=0 OR p.created_at IS NOT NULL)",
            ReadPhase::Creation { .. } => "p.started=1 AND p.provider_ref IS NULL AND p.created_at IS NULL AND p.export_started=0 AND p.creation_expires_at=?2 AND p.creation_expires_at>?3",
            ReadPhase::Export { .. } => "p.started=1 AND p.provider_ref IS NOT NULL AND p.created_at IS NOT NULL AND p.export_started=1 AND p.export_uncertain=0 AND p.delivered_at IS NULL AND p.export_expires_at=?2 AND p.export_expires_at>?3",
        };
        let query = format!("SELECT d.selection_json,p.provider_ref FROM context_deliveries d JOIN private_provider_contexts p ON p.delivery_id=d.id WHERE d.id=?1 AND d.state='pending' AND p.uncertain=0 AND p.closed_at IS NULL AND ({phase_predicate})");
        let (encoded, provider_ref): (String, Option<String>) =
            if matches!(phase, ReadPhase::Planning) {
                c.query_row(&query, [&context.delivery], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
            } else {
                c.query_row(
                    &query,
                    params![context.delivery, phase.until(), crate::now()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
            }
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
            .min(crate::now() + 5)
            .min(phase.until());
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        let mutation_hint = scopes_for_facts(self, c, context, &facts, true)?;
        Ok(PrivateProviderRead {
            context: context.clone(),
            mutation_hint,
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
    ) -> Result<PrivateProviderAttempt> {
        self.begin_private_provider_context_before(context, read, i64::MAX)
    }
    pub fn begin_private_provider_context_before(
        &self,
        context: PrivateProviderContext,
        read: PrivateProviderRead,
        original_until: i64,
    ) -> Result<PrivateProviderAttempt> {
        if !read.current()
            || !self.authority_covers(&read._held, &[])
            || read.provider_ref.is_some()
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let until = read.expires_at.min(original_until);
        let encoded = read.json.clone();
        let endpoint = read.endpoint.clone();
        // Scope names only: no grant survives this original reader's release.
        let hint = read.mutation_hint.clone();
        drop(read);
        let (held, mut c) = self.private_provider_reservation_sql(&context, &hint, "creation")?;
        let tx = admission_stage("creation", "begin-immediate", || {
            c.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::storage())
        })?;
        let current = admission_stage("creation", "transaction-current", || {
            self.private_provider_read_at_for(&tx, &context, held, ReadPhase::Planning, true)
        })?;
        if current.provider_ref.is_some()
            || current.json != encoded
            || current.endpoint != endpoint
            || crate::now() >= until
        {
            return Err(Error::forbidden());
        }
        let until = until.min(current.expires_at);
        if tx.execute("UPDATE private_provider_contexts SET started=1,creation_expires_at=?2 WHERE delivery_id=?1 AND started=0 AND uncertain=0 AND delivered_at IS NULL",params![context.delivery,until]).map_err(|_|Error::storage())?!=1{return Err(Error::new("reconciliation-required","Private provider initialization already attempted"));}
        tx.execute(
            "UPDATE operations SET state='preparing' WHERE record_id=?1 AND state='accepted'",
            [context.consultation_record_id()],
        )
        .map_err(|_| Error::storage())?;
        progress(&tx, context.consultation_record_id(), self)?;
        if !current.current() || crate::now() >= until {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(PrivateProviderAttempt {
            context,
            expires_at: until,
        })
    }
    pub fn try_private_provider_creation_read(
        &self,
        attempt: &PrivateProviderAttempt,
    ) -> Result<PrivateProviderRead> {
        self.private_provider_phase_read(
            &attempt.context,
            ReadPhase::Creation {
                until: attempt.expires_at,
            },
        )
    }
    pub fn try_private_provider_export_read(
        &self,
        attempt: &PrivateProviderExport,
    ) -> Result<PrivateProviderRead> {
        let read = self.private_provider_phase_read(
            &attempt.context,
            ReadPhase::Export {
                until: attempt.expires_at,
            },
        )?;
        if read.provider_ref.as_deref() != Some(attempt.provider_ref.as_str()) {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
    fn private_provider_phase_read(
        &self,
        context: &PrivateProviderContext,
        phase: ReadPhase,
    ) -> Result<PrivateProviderRead> {
        if crate::now() >= phase.until() {
            return Err(Error::forbidden());
        }
        let needed = {
            let c = self.try_connection()?;
            bound_scopes(self, &c, context, false)?
        };
        let held = self.try_acquire_authority(&needed)?;
        let c = self.try_connection()?;
        self.private_provider_read_at(&c, context, held, phase)
    }
    fn private_provider_reservation_sql<'a>(
        &'a self,
        context: &PrivateProviderContext,
        hint: &[AuthorityScope],
        phase: &'static str,
    ) -> Result<(
        Arc<AuthorityGuardSet>,
        crate::installation_ownership::ConnectionGuard<'a>,
    )> {
        // The original read validated these names. They route admission only;
        // fresh complete facts/footprint below and inside TX remain mandatory.
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(5));
        let held = admission_stage(phase, "file-admission", || {
            self.acquire_authority(hint, &wait)
        })?;
        let c = admission_stage(phase, "sql-reacquire", || self.try_connection())?;
        let current = admission_stage(phase, "scope-rederive", || {
            bound_scopes(self, &c, context, true)
        })?;
        if !self.authority_covers(&held, &current) {
            return Err(Error::new(
                "authority-busy",
                "Private provider mutation scope changed",
            ));
        }
        c.bind_available_mutation_authority(self, &held)?;
        Ok((held, c))
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
        let (held, mut c) = self.private_provider_history_sql(
            &receipt.delivery,
            &receipt.attempt,
            &receipt.runtime,
        )?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_private_provider_history(
            &tx,
            &held,
            &receipt.delivery,
            &receipt.attempt,
            &receipt.runtime,
        )?;
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
    ) -> Result<PrivateProviderExport> {
        self.begin_private_provider_export_before(context, read, i64::MAX)
    }
    pub fn begin_private_provider_export_before(
        &self,
        context: PrivateProviderContext,
        read: PrivateProviderRead,
        original_until: i64,
    ) -> Result<PrivateProviderExport> {
        if !read.current()
            || !self.authority_covers(&read._held, &[])
            || read.context.delivery != context.delivery
            || read.context.attempt != context.attempt
        {
            return Err(Error::forbidden());
        }
        let provider_ref = read.provider_ref.clone().ok_or_else(Error::forbidden)?;
        let until = read.expires_at.min(original_until);
        let encoded = read.json.clone();
        let endpoint = read.endpoint.clone();
        let hint = read.mutation_hint.clone();
        drop(read);
        let (held, mut c) = self.private_provider_reservation_sql(&context, &hint, "export")?;
        let tx = admission_stage("export", "begin-immediate", || {
            c.transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::storage())
        })?;
        let current = admission_stage("export", "transaction-current", || {
            self.private_provider_read_at_for(&tx, &context, held, ReadPhase::Planning, true)
        })?;
        if current.provider_ref.as_deref() != Some(provider_ref.as_str())
            || current.json != encoded
            || current.endpoint != endpoint
            || crate::now() >= until
        {
            return Err(Error::forbidden());
        }
        let until = until.min(current.expires_at);
        if tx.execute("UPDATE private_provider_contexts SET export_started=1,export_expires_at=?3 WHERE delivery_id=?1 AND started=1 AND provider_ref=?2 AND created_at IS NOT NULL AND export_started=0 AND export_uncertain=0 AND uncertain=0 AND closed_at IS NULL AND delivered_at IS NULL",params![context.delivery,provider_ref,until]).map_err(|_|Error::storage())?!=1{return Err(Error::new("reconciliation-required","Private provider export already attempted"));}
        progress(&tx, context.consultation_record_id(), self)?;
        if !current.current() || crate::now() >= until {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(PrivateProviderExport {
            context,
            provider_ref,
            expires_at: until,
        })
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
        let (held, mut c) = self.private_provider_history_sql(
            &receipt.delivery,
            &receipt.attempt,
            &receipt.runtime,
        )?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_private_provider_history(
            &tx,
            &held,
            &receipt.delivery,
            &receipt.attempt,
            &receipt.runtime,
        )?;
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
