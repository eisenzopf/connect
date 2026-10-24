//! Single-attempt audience custody and separately verified native human lifetime.
use super::*;
use crate::{NativeConsultationRead, NativeConsultationSelection};
use std::sync::Weak;
mod preparation;
pub(crate) use preparation::settle_after_provider_closed;
pub use preparation::NativeConsultationPreparation;

pub struct NativeConsultationLease {
    selection: NativeConsultationSelection,
    attempt: String,
    effect: String,
    plan: String,
    selection_json: String,
    hash: String,
    expires_at: i64,
}
pub struct NativeConsultationAttempt {
    lease: NativeConsultationLease,
}
pub struct NativeConsultationApplied {
    owner: Weak<crate::InstallationOwnership>,
    runtime: String,
    record: String,
    attempt: String,
    effect: String,
    hash: String,
    observation: String,
}
impl std::fmt::Debug for NativeConsultationLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationLease(redacted)")
    }
}
impl std::fmt::Debug for NativeConsultationAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationAttempt(redacted)")
    }
}
impl std::fmt::Debug for NativeConsultationApplied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationApplied(redacted)")
    }
}
impl NativeConsultationLease {
    /// Recording-only custody. It cannot authorize or repeat native IO.
    pub fn preparation(&self) -> NativeConsultationPreparation {
        preparation::capture(self)
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn record_id(&self) -> &str {
        self.selection.origin.record_id()
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.plan).map_err(|_| Error::storage())
    }
}
impl NativeConsultationAttempt {
    pub fn expires_at(&self) -> i64 {
        self.lease.expires_at
    }
    pub fn record_id(&self) -> &str {
        self.lease.record_id()
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        self.lease.plan()
    }
    /// Trusted runtime boundary only after actual Core audience application
    /// returned this observation. It is not decoded consulting media readiness.
    pub fn applied(self, observation: serde_json::Value) -> Result<NativeConsultationApplied> {
        let plan = self.lease.plan()?;
        let revisions = plan["expected_receiver_revisions"]
            .as_object()
            .ok_or_else(invalid)?;
        let observed = observation["receiver_revisions"]
            .as_object()
            .ok_or_else(invalid)?;
        if observation["operation_id"] != plan["operation_id"]
            || observation["audience_revision"].as_u64()
                != plan["audience_revision"]
                    .as_u64()
                    .and_then(|r| r.checked_add(1))
            || observation["restored"] != serde_json::json!(false)
            || !observation["departed_connections"]
                .as_array()
                .is_some_and(Vec::is_empty)
            || !observation["participant_handoff"].is_null()
            || !observation["participant_replacement"].is_null()
            || observed.len() != revisions.len()
            || revisions.iter().any(|(id, previous)| {
                observed.get(id).and_then(|r| r.as_u64())
                    != previous.as_u64().and_then(|r| r.checked_add(1))
            })
        {
            return Err(conflict());
        }
        let encoded = crate::provisioning::canonical_json(&observation)?;
        if encoded.len() > 32768 {
            return Err(invalid());
        }
        let origin = &self.lease.selection.origin;
        Ok(NativeConsultationApplied {
            owner: origin.owner.clone(),
            runtime: origin.runtime.clone(),
            record: origin.record.clone(),
            attempt: self.lease.attempt,
            effect: self.lease.effect,
            hash: self.lease.hash,
            observation: encoded,
        })
    }
}
fn bound(c: &Connection, store: &Store, lease: &NativeConsultationLease) -> Result<rows::Saved> {
    let origin = &lease.selection.origin;
    origin.require(store, c)?;
    let matches:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_attempts n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.runtime_epoch=?3 AND n.credential_id=?4 AND n.effect_id=?5 AND n.plan_json=?6 AND n.selection_json=?7 AND n.plan_digest=?8 AND o.lease_owner=?2 AND o.lease_expires_at=?9)",params![origin.record,lease.attempt,origin.runtime,origin.credential,lease.effect,lease.plan,lease.selection_json,lease.hash,lease.expires_at],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !matches || crate::now() >= lease.expires_at {
        return Err(Error::forbidden());
    }
    rows::by_record(c, &origin.record)
}
fn event(c: &Connection, row: &rows::Saved, store: &Store) -> Result<()> {
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable operation journal capacity exceeded",
        ));
    }
    let (pid,epoch):(String,u64)=c.query_row("SELECT participant_id,membership_epoch FROM operation_members WHERE operation_record_id=?1 AND identity_id=?2",params![row.record.record_id,row.owner],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let data = serde_json::json!({"source_kind":"session","source_id":row.record.session_id,"operation_id":row.record.operation_id,"state":row.record.state.as_str()});
    c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,'operation_updated',?4,?5,?6,?7,?8)",params![uuid::Uuid::new_v4().to_string(),row.owner,row.record.conversation_id,row.record.record_id,row.record.revision,data.to_string(),pid,epoch]).map_err(|_|Error::storage())?;
    Ok(())
}
fn touch(c: &Connection, record: &str, store: &Store) -> Result<()> {
    let changed=c.execute("UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1 AND revision<9007199254740990",params![record,crate::now()]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(conflict());
    }
    event(c, &rows::by_record(c, record)?, store)
}
pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<bool> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(false);
    }
    let (started,runtime):(bool,String)=c.query_row("SELECT started,runtime_epoch FROM native_consultation_attempts WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let expired = row.record.expires_at.is_none_or(|t| t <= crate::now());
    let stale = row.lease_until.is_none_or(|t| t <= crate::now())
        || (store.recover_foreign_runtime && runtime != store.runtime_epoch());
    if !expired && !stale {
        return Ok(false);
    }
    c.execute("UPDATE operation_effects SET state=?2 WHERE operation_record_id=?1 AND state IN ('prepared','submitting')",params![row.record.record_id,if started {"unknown"}else{"cancelled"}]).map_err(|_|Error::storage())?;
    let next = if started || rows::unsettled(c, &row.record.record_id)? {
        OperationState::Unknown
    } else {
        OperationState::Failed
    };
    c.execute(
        "UPDATE operations SET state=?2 WHERE record_id=?1",
        params![row.record.record_id, next.as_str()],
    )
    .map_err(|_| Error::storage())?;
    touch(c, &row.record.record_id, store)?;
    workers::invalidate(c, &row.record.record_id)?;
    Ok(true)
}
impl Store {
    /// Complete only after captured decoded media and exact private preparation.
    /// AI additionally requires independently captured fresh provider context;
    /// Participant/Endpoint labels cannot manufacture initialization readiness.
    pub fn complete_native_consultation(
        &self,
        receipt: &NativeConsultationApplied,
    ) -> Result<Option<OperationRecord>> {
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &receipt.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                if row.record.state != OperationState::Completed {
                    scopes.extend(crate::authority_admission::conversation_scopes(
                        &c,
                        row.record
                            .conversation_id
                            .as_deref()
                            .ok_or_else(Error::storage)?,
                        crate::AuthorityMode::Write,
                    )?);
                }
                crate::AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation progress has expired"));
        }
        admission::require_row(self, &held, &row)?;
        let data:Option<(String,String)>=tx.query_row(
            "SELECT n.selection_json,l.audience_json FROM native_consultation_attempts n
             JOIN native_consultation_legs l ON l.operation_record_id=n.operation_record_id
             JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
             WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.effect_id=?3 AND n.plan_digest=?4
             AND n.runtime_epoch=?5 AND n.started=1 AND n.applied_json=?6 AND n.decoded_audio_observed=1 AND n.media_failure IS NULL",
            params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime,receipt.observation],
            |r|Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(|_|Error::storage())?;
        let (selection, audience) = data.ok_or_else(Error::forbidden)?;
        if row.record.state == OperationState::Completed {
            let established:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_private_lifetimes WHERE operation_record_id=?1 AND runtime_epoch=?2 AND selection_json=?3 AND audience_json=?4)",params![receipt.record,receipt.runtime,selection,audience],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !established {
                return Err(Error::forbidden());
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(Some(row.record));
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let restored:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.consultation_record_id=?1 AND r.applied_json IS NOT NULL)",[&receipt.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if restored {
            return Err(conflict());
        }
        let context = crate::connection_control::private_consultation::active::validate(
            self,
            &tx,
            &receipt.record,
            &selection,
            &audience,
        )?;
        let human:bool=tx.query_row("SELECT p.kind='human' AND i.kind='human' FROM connections v JOIN participants p ON p.participant_id=v.participant_id JOIN identities i ON i.id=v.identity_id WHERE v.id=?1",[&context.candidate],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !human
            && !crate::private_provider::ready(&tx, self, &receipt.record, &selection, &audience)?
        {
            return Ok(None);
        }
        let prepared:bool=tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes life JOIN operations o ON o.record_id=life.operation_record_id WHERE life.connection_id=?1 AND o.state='completed'
             AND json_extract(o.result_json,'$.private_consultation.profile')='connect-private-audio-v1'
             AND json_extract(o.result_json,'$.private_consultation.operation_record_id')=?2
             AND json_extract(o.result_json,'$.private_consultation.prepared_muted')=1)",
            params![context.candidate,receipt.record],|r|r.get(0),
        ).map_err(|_|Error::storage())?;
        if !prepared {
            return Err(Error::forbidden());
        }
        let request = rows::request(&row)?;
        let base_revision = super::consultation_source::effective_revision(&tx, &row)?;
        let session_revision = base_revision
            .checked_add(1)
            .filter(|n| *n < 9_007_199_254_740_991)
            .ok_or_else(conflict)?;
        let changed=tx.execute("UPDATE sessions SET topology_revision=?2,updated_at=?3 WHERE id=?1 AND topology_revision=?4",params![context.session,session_revision,crate::now(),base_revision]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        if row.record.state == OperationState::Preparing {
            tx.execute(
                "UPDATE operations SET state='committing' WHERE record_id=?1",
                [&receipt.record],
            )
            .map_err(|_| Error::storage())?;
            touch(&tx, &receipt.record, self)?;
        }
        tx.execute("INSERT INTO native_private_lifetimes(operation_record_id,runtime_epoch,selection_json,audience_json,established_at) VALUES(?1,?2,?3,?4,?5)",params![receipt.record,receipt.runtime,selection,audience,crate::now()]).map_err(|_|Error::storage())?;
        let mut retained = vec![context.primary, context.remote];
        retained.sort();
        let completion = TransitionCompletion {
            retained_connection_ids: retained,
            admitted_connection_ids: vec![context.candidate],
            retired_connection_ids: vec![],
            session_revision,
            conversation_revision: None,
        };
        tx.execute(
            "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
            params![
                receipt.record,
                serde_json::to_string(&completion).map_err(|_| Error::storage())?
            ],
        )
        .map_err(|_| Error::storage())?;
        touch(&tx, &receipt.record, self)?;
        workers::invalidate(&tx, &receipt.record)?;
        let session =
            crate::session_control::view(&tx, &request.conversation_id, &request.session_id)?;
        crate::session_control::emit_transition(
            &tx,
            &row.owner,
            &row.record.operation_id,
            &request.conversation_id,
            &request.session_id,
            "session.consult",
            &session,
            self,
        )?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &receipt.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(result))
    }
    /// Actual sealed-handle failure after audience application preserves known
    /// effect confirmation and fences the unfinished engagement as unknown.
    pub fn record_native_consultation_interrupted(
        &self,
        receipt: &NativeConsultationApplied,
        reason: &str,
    ) -> Result<()> {
        if !matches!(
            reason,
            "timeout"
                | "cancelled"
                | "primary-ended"
                | "remote-ended"
                | "consultant-ended"
                | "already-restored"
                | "native-confirmation-lost"
        ) {
            return Err(invalid());
        }
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &receipt.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation progress has expired"));
        }
        let existing:Option<Option<String>>=tx.query_row("SELECT media_failure FROM native_consultation_attempts WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND plan_digest=?4 AND runtime_epoch=?5 AND started=1 AND applied_json=?6",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime,receipt.observation],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let existing = existing.ok_or_else(Error::forbidden)?;
        if let Some(existing) = existing {
            if existing != reason {
                return Err(conflict());
            }
        } else {
            if !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            ) {
                return Err(conflict());
            }
            tx.execute("UPDATE native_consultation_attempts SET media_failure=?2 WHERE operation_record_id=?1",params![receipt.record,reason]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE operations SET state='unknown',problem_json=COALESCE(problem_json,?2) WHERE record_id=?1",params![receipt.record,serde_json::json!({"code":"native-consultation-interrupted","reason":reason}).to_string()]).map_err(|_|Error::storage())?;
            touch(&tx, &receipt.record, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Trusted runtime calls only after the actual sealed Core consultation
    /// handle reports decoded candidate audio. This is historical progress,
    /// not provider readiness or permission to complete/execute a transition.
    pub fn record_native_consultation_decoded(
        &self,
        receipt: &NativeConsultationApplied,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &receipt.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation progress has expired"));
        }
        admission::require_row(self, &held, &row)?;
        let existing:Option<bool>=tx.query_row("SELECT decoded_audio_observed FROM native_consultation_attempts WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND plan_digest=?4 AND runtime_epoch=?5 AND started=1 AND applied_json=?6",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime,receipt.observation],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let existing = existing.ok_or_else(Error::forbidden)?;
        if !existing {
            tx.execute("UPDATE native_consultation_attempts SET decoded_audio_observed=1 WHERE operation_record_id=?1",[&receipt.record]).map_err(|_|Error::storage())?;
            touch(&tx, &receipt.record, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Drop planning readers before acquiring metadata writer authority. Plan
    /// and selected incarnations are durable before any execution permit exists.
    pub fn prepare_native_consultation(
        &self,
        read: NativeConsultationRead,
        plan: serde_json::Value,
    ) -> Result<Option<NativeConsultationLease>> {
        let selection = read.into_selection()?;
        selection.validate_plan(&plan)?;
        let (hash, encoded) = digest(&plan)?;
        let selected = crate::provisioning::canonical_json(&selection.snapshot())?;
        if encoded.len() > 32768 || selected.len() > 32768 {
            return Err(invalid());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                selection.scopes(self, &c, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !self.authority_covers(&held, &selection.scopes(self, &tx, true)?) {
            return Err(Error::forbidden());
        }
        let origin = &selection.origin;
        let row = rows::by_record(&tx, &origin.record)?;
        crate::private_provider::require_for_native(&tx, self, &origin.record, &selected)?;
        if let Some((attempt,effect,old_plan,old_selection,old_hash,started,runtime,credential,expires))=tx.query_row("SELECT n.attempt_id,n.effect_id,n.plan_json,n.selection_json,n.plan_digest,n.started,n.runtime_epoch,n.credential_id,o.lease_expires_at FROM native_consultation_attempts n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1",[&origin.record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,i64>(8)?))).optional().map_err(|_|Error::storage())? {
            if old_plan!=encoded || old_selection!=selected || old_hash!=hash || credential!=origin.credential{return Err(conflict());}
            if runtime!=self.runtime_epoch() || started || row.record.state!=OperationState::Preparing || expires<=crate::now(){return Ok(None);}
            tx.commit().map_err(|_|Error::storage())?;
            return Ok(Some(NativeConsultationLease{selection,attempt,effect,plan:encoded,selection_json:selected,hash,expires_at:expires}));
        }
        if !(row.record.state == OperationState::Accepted
            || (row.record.state == OperationState::Preparing
                && row.native_provider
                && row.worker.is_none()))
            || !row.record.effects.is_empty()
            || row.generation >= 9_007_199_254_740_990
        {
            return Err(conflict());
        }
        let attempt = format!("nca_{}", uuid::Uuid::new_v4());
        let effect = format!("nce_{}", uuid::Uuid::new_v4());
        let expires = (crate::now() + 60).min(origin.request.expires_at);
        tx.execute("UPDATE operations SET state='preparing',lease_owner=?2,lease_expires_at=?3,worker_generation=worker_generation+1,worker_runtime=?4,worker_cleanup=0 WHERE record_id=?1",params![origin.record,attempt,expires,self.transition_runtime]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-consultation',?2,'none','prepared')",params![effect,origin.record]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_consultation_attempts(operation_record_id,attempt_id,runtime_epoch,credential_id,effect_id,source_expires_at,selection_json,plan_json,plan_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![origin.record,attempt,self.runtime_epoch(),origin.credential,effect,origin.request.expires_at,selected,encoded,hash]).map_err(|_|Error::storage())?;
        touch(&tx, &origin.record, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(NativeConsultationLease {
            selection,
            attempt,
            effect,
            plan: encoded,
            selection_json: selected,
            hash,
            expires_at: expires,
        }))
    }
    pub fn native_consultation_attempt_read(
        &self,
        lease: &NativeConsultationLease,
    ) -> Result<NativeConsultationRead> {
        {
            let c = self.try_connection()?;
            bound(&c, self, lease)?;
        }
        let mut read = lease.selection.read(self)?;
        read.limit(lease.expires_at)?;
        Ok(read)
    }
    /// A fresh complete selection remains held through the bounded Core call.
    /// SQL permits exactly one transition from prepared to possibly submitted.
    pub fn begin_native_consultation(
        &self,
        lease: NativeConsultationLease,
        mut read: NativeConsultationRead,
    ) -> Result<(NativeConsultationAttempt, NativeConsultationRead)> {
        read.limit(lease.expires_at)?;
        if !read.current()
            || read.selection().snapshot() != lease.selection.snapshot()
            || read.origin().record_id() != lease.record_id()
            || read.origin().credential != lease.selection.origin.credential
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = bound(&tx, self, &lease)?;
        lease.selection.validate_revisions(&tx, &lease.plan()?)?;
        crate::private_provider::require_for_native(
            &tx,
            self,
            lease.record_id(),
            &lease.selection_json,
        )?;
        if row.record.state != OperationState::Preparing {
            return Err(conflict());
        }
        let changed=tx.execute("UPDATE native_consultation_attempts SET started=1 WHERE operation_record_id=?1 AND started=0",[lease.record_id()]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(Error::new(
                "reconciliation-required",
                "Native consultation already attempted",
            ));
        }
        let changed=tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1 AND state='prepared' AND attempt=0",[&lease.effect]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        touch(&tx, lease.record_id(), self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((NativeConsultationAttempt { lease }, read))
    }
    /// An engine error after attempt admission is uncertainty, never permission
    /// to call Core again. The caller drops its bounded IO readers first.
    pub fn native_consultation_uncertain(&self, attempt: NativeConsultationAttempt) -> Result<()> {
        let lease = attempt.lease;
        let owner = self.require_runtime_owner()?;
        let origin = &lease.selection.origin;
        if origin.runtime != self.runtime_epoch()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &origin.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &origin.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_attempts WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND runtime_epoch=?4 AND plan_digest=?5 AND started=1)",params![origin.record,lease.attempt,lease.effect,origin.runtime,lease.hash],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact
            || !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            )
        {
            return Err(Error::forbidden());
        }
        tx.execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
            [&lease.effect],
        )
        .map_err(|_| Error::storage())?;
        if row.record.state != OperationState::Unknown {
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1",
                [&origin.record],
            )
            .map_err(|_| Error::storage())?;
            touch(&tx, &origin.record, self)?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Recording-only historical custody after actual native application. No
    /// original grant, reader, selected native origin or command is retained.
    pub fn record_native_consultation_applied(
        &self,
        receipt: &NativeConsultationApplied,
    ) -> Result<OperationRecord> {
        let owner = self.require_runtime_owner()?;
        if receipt.runtime != self.runtime_epoch()
            || !receipt
                .owner
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&owner, &p))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &receipt.record)?;
                admission::row_scopes(&row)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        let existing:Option<Option<String>>=tx.query_row("SELECT applied_json FROM native_consultation_attempts WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND plan_digest=?4 AND runtime_epoch=?5 AND started=1",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let existing = existing.ok_or_else(Error::forbidden)?;
        if let Some(existing) = existing {
            if existing != receipt.observation {
                return Err(conflict());
            }
        } else {
            if !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            ) {
                return Err(conflict());
            }
            tx.execute("UPDATE native_consultation_attempts SET applied_json=?2 WHERE operation_record_id=?1",params![receipt.record,receipt.observation]).map_err(|_|Error::storage())?;
            let changed=tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&receipt.effect]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
            let (plan,selected):(String,String)=tx.query_row("SELECT plan_json,selection_json FROM native_consultation_attempts WHERE operation_record_id=?1",[&receipt.record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
            let plan: serde_json::Value =
                serde_json::from_str(&plan).map_err(|_| Error::storage())?;
            let selected: serde_json::Value =
                serde_json::from_str(&selected).map_err(|_| Error::storage())?;
            let observed: serde_json::Value =
                serde_json::from_str(&receipt.observation).map_err(|_| Error::storage())?;
            for binding in std::iter::once(&selected["remote"])
                .chain(
                    selected["private"]
                        .as_array()
                        .ok_or_else(Error::storage)?
                        .iter(),
                )
                .chain(
                    selected["outside"]
                        .as_array()
                        .ok_or_else(Error::storage)?
                        .iter(),
                )
            {
                let id = binding["connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)?;
                let runtime = binding["runtime_connection_id"]
                    .as_str()
                    .ok_or_else(Error::storage)?;
                let generation = binding["generation"].as_u64().ok_or_else(Error::storage)?;
                let previous = plan["expected_receiver_revisions"][runtime]
                    .as_u64()
                    .ok_or_else(Error::storage)?;
                let revision = observed["receiver_revisions"][runtime]
                    .as_u64()
                    .ok_or_else(Error::storage)?;
                if binding.get("source_kind").is_some() {
                    self.record_sip_audience_revision(
                        &tx,
                        binding,
                        &receipt.runtime,
                        previous,
                        revision,
                    )?;
                    continue;
                }
                // Historical metadata never revives a retired/replaced generation
                // or rewrites a later native observation.
                tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0)",params![id,generation,receipt.runtime,revision,previous]).map_err(|_|Error::storage())?;
            }
            if let Some(sips) = selected.get("outside_sip") {
                for binding in sips.as_array().ok_or_else(Error::storage)? {
                    let runtime_id = binding["runtime_connection_id"]
                        .as_str()
                        .ok_or_else(Error::storage)?;
                    let previous = plan["expected_receiver_revisions"][runtime_id]
                        .as_u64()
                        .ok_or_else(Error::storage)?;
                    let revision = observed["receiver_revisions"][runtime_id]
                        .as_u64()
                        .ok_or_else(Error::storage)?;
                    self.record_sip_audience_revision(
                        &tx,
                        binding,
                        &receipt.runtime,
                        previous,
                        revision,
                    )?;
                }
            }
            touch(&tx, &receipt.record, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &receipt.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
