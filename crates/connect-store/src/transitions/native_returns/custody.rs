//! Exactly one native restoration attempt; captured results only authorize recording.
use super::*;
use crate::{NativeReturnRead, NativeReturnSelection};
/// Only Deferred is a definite pre-reservation admission miss. After any
/// transaction DML/storage error the caller must retain ordinary uncertainty;
/// it may not replay planning/prepare as if nothing happened.
#[derive(Debug)]
pub enum NativeConsultationReturnPreparation {
    Deferred(Error),
    Reserved(NativeConsultationReturnLease),
    NoWork,
}
pub struct NativeConsultationReturnLease {
    selection: NativeReturnSelection,
    attempt: String,
    effect: String,
    plan: String,
    selection_json: String,
    hash: String,
    expires_at: i64,
}
pub struct NativeConsultationReturnAttempt {
    lease: Arc<NativeConsultationReturnLease>,
}
pub struct NativeConsultationReturnApplied {
    owner: Weak<crate::InstallationOwnership>,
    runtime: String,
    record: String,
    attempt: String,
    effect: String,
    hash: String,
    observation: String,
    metadata: connect_protocol::native_audience::AudienceMetadata,
}
impl std::fmt::Debug for NativeConsultationReturnLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnLease(redacted)")
    }
}
impl std::fmt::Debug for NativeConsultationReturnAttempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnAttempt(redacted)")
    }
}
impl std::fmt::Debug for NativeConsultationReturnApplied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeConsultationReturnApplied(redacted)")
    }
}
impl NativeConsultationReturnApplied {
    pub fn record_id(&self) -> &str {
        &self.record
    }
}
impl NativeConsultationReturnLease {
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
impl NativeConsultationReturnAttempt {
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
    pub fn applied(
        self,
        observation: serde_json::Value,
        metadata: connect_protocol::native_audience::AudienceMetadata,
    ) -> Result<NativeConsultationReturnApplied> {
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
            || observation["restored"] != serde_json::json!(true)
            || observation["departed_connections"] != plan["known_departed_connections"]
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
        connect_protocol::native_audience::validate(
            &metadata,
            observation["receiver_revisions"]
                .as_object()
                .ok_or_else(invalid)?,
        )?;
        let encoded = crate::provisioning::canonical_json(&observation)?;
        if encoded.len() > 32768 {
            return Err(invalid());
        }
        let origin = &self.lease.selection.origin;
        Ok(NativeConsultationReturnApplied {
            owner: origin.owner.clone(),
            runtime: origin.runtime.clone(),
            record: origin.record.clone(),
            attempt: self.lease.attempt.clone(),
            effect: self.lease.effect.clone(),
            hash: self.lease.hash.clone(),
            observation: encoded,
            metadata,
        })
    }
}
fn bound(
    c: &Connection,
    store: &Store,
    lease: &NativeConsultationReturnLease,
) -> Result<rows::Saved> {
    let origin = &lease.selection.origin;
    origin.require(store, c)?;
    let matches:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE n.operation_record_id=?1 AND n.attempt_id=?2 AND n.runtime_epoch=?3 AND n.credential_id=?4 AND n.effect_id=?5 AND n.plan_json=?6 AND n.selection_json=?7 AND n.plan_digest=?8 AND o.lease_owner=?2 AND o.lease_expires_at=?9)",params![origin.record,lease.attempt,origin.runtime,origin.credential,lease.effect,lease.plan,lease.selection_json,lease.hash,lease.expires_at],|r|r.get(0)).map_err(|_|Error::storage())?;
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
pub(super) fn touch(c: &Connection, record: &str, store: &Store) -> Result<()> {
    let changed=c.execute("UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1 AND revision<9007199254740990",params![record,crate::now()]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(conflict());
    }
    event(c, &rows::by_record(c, record)?, store)
}
fn return_begin_refused(stage: &'static str, error: &Error) {
    tracing::debug!(target: "connect_store::native_return::admission", stage, code=%error.code,
        "Original native return begin deferred before effect");
}
impl Store {
    /// Exact SQL footprint for one original return. Identity policy facts keep
    /// Read; only affected Conversation metadata and enrolled dependents gain
    /// Write. This is never a reusable permission to restore media.
    pub(crate) fn native_return_mutation_scopes(
        &self,
        c: &Connection,
        record: &str,
        include_parent: bool,
    ) -> Result<Vec<crate::AuthorityScope>> {
        let row = rows::by_record(c, record)?;
        if !row.native_return || row.record.command_type != "session.consult.return" {
            return Err(Error::forbidden());
        }
        let cid = row
            .record
            .conversation_id
            .as_deref()
            .ok_or_else(Error::forbidden)?;
        let parent: String = c.query_row(
            "SELECT consultation_record_id FROM native_consultation_returns WHERE operation_record_id=?1",
            [record], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        let original = rows::by_record(c, &parent)?;
        if original.record.conversation_id.as_deref() != Some(cid) {
            return Err(Error::forbidden());
        }
        let mut scopes = vec![
            crate::AuthorityScope::identity(&row.owner, crate::AuthorityMode::Read)?,
            crate::AuthorityScope::conversation(cid, crate::AuthorityMode::Write)?,
        ];
        // The persistent operations trigger also guards an enrollment subject
        // conditionally dependent on either operation actually changed here.
        let mut q = c.prepare_cached(
            "SELECT e.identity_id FROM client_enrollments e JOIN operations o ON o.tenant_id=e.tenant_id AND o.owner_id=e.issuer_identity_id AND o.operation_id=e.complete_operation_id WHERE o.record_id=?1 OR (?3 AND o.record_id=?2)"
        ).map_err(|_| Error::storage())?;
        for subject in q
            .query_map(params![record, parent, include_parent], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| Error::storage())?
        {
            scopes.push(crate::AuthorityScope::identity(
                &subject.map_err(|_| Error::storage())?,
                crate::AuthorityMode::Write,
            )?);
        }
        crate::AuthorityScope::merged(&scopes)
    }
    fn native_return_recording_sql<'a>(
        &'a self,
        record: &str,
    ) -> Result<(
        Arc<crate::AuthorityGuardSet>,
        crate::installation_ownership::ConnectionGuard<'a>,
    )> {
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(5));
        // Historical recording owns no IO reader: wait for SQL first, then
        // announce/try exact Writes immediately. Never park a Writer on SQL.
        let c = self.try_connection_with_wait(&wait)?;
        let scopes = self.native_return_mutation_scopes(&c, record, false)?;
        let held = self.try_acquire_authority(&scopes)?;
        if !self.authority_covers(
            &held,
            &self.native_return_mutation_scopes(&c, record, false)?,
        ) {
            return Err(Error::forbidden());
        }
        c.bind_available_mutation_authority(self, &held)?;
        Ok((held, c))
    }
}
impl Store {
    /// Drop planning readers before acquiring metadata writer authority. Plan
    /// and selected incarnations are durable before any execution permit exists.
    pub fn prepare_native_consultation_return(
        &self,
        read: NativeReturnRead,
        plan: serde_json::Value,
    ) -> Result<Option<NativeConsultationReturnLease>> {
        match self.prepare_native_return_admission(read, plan, None)? {
            NativeConsultationReturnPreparation::Deferred(error) => Err(error),
            NativeConsultationReturnPreparation::Reserved(lease) => Ok(Some(lease)),
            NativeConsultationReturnPreparation::NoWork => Ok(None),
        }
    }
    /// Consume all planning Reads before bounded primary SQL admission. Only
    /// immediate authority admission is allowed under that SQL mutex.
    pub fn prepare_native_consultation_return_with_wait(
        &self,
        mut read: NativeReturnRead,
        plan: serde_json::Value,
        original_until: i64,
        wait: &crate::AuthorityWait,
    ) -> Result<NativeConsultationReturnPreparation> {
        read.limit(original_until)?;
        let until = read.expires_at();
        let wait = wait.bounded(std::time::Duration::from_secs(
            until.saturating_sub(crate::now()).max(0) as u64,
        ));
        self.prepare_native_return_admission(read, plan, Some(&wait))
    }
    fn prepare_native_return_admission(
        &self,
        read: NativeReturnRead,
        plan: serde_json::Value,
        control_wait: Option<&crate::AuthorityWait>,
    ) -> Result<NativeConsultationReturnPreparation> {
        let original_until = read.expires_at();
        let selection = read.into_selection()?;
        let (hash, encoded) = digest(&plan)?;
        let selected = crate::provisioning::canonical_json(&selection.snapshot())?;
        if encoded.len() > 32768 || selected.len() > 32768 {
            return Err(invalid());
        }
        // Never exclude current private packet readers while waiting for SQL.
        // Couple only immediate admissions, then validate every current fact
        // again in the transaction. The service retries fresh planning reads;
        // no reservation or physical execution has happened on refusal.
        let mut c = match (match control_wait {
            Some(wait) => self.try_connection_with_wait(wait),
            None => self.try_connection(),
        }) {
            Ok(connection) => connection,
            Err(error) if error.code == "authority-busy" => {
                return_begin_refused("prepare-sql", &error);
                return Ok(NativeConsultationReturnPreparation::Deferred(error));
            }
            Err(error) => return Err(error),
        };
        let needed = selection.scopes(self, &c, true)?;
        if let Some(wait) = control_wait {
            wait.check()?;
        }
        let held = match self.try_acquire_authority(&needed) {
            Ok(held) => held,
            Err(error) if error.code == "authority-busy" => {
                return_begin_refused("prepare-file-immediate", &error);
                return Ok(NativeConsultationReturnPreparation::Deferred(error));
            }
            Err(error) => return Err(error),
        };
        c.bind_available_mutation_authority(self, &held)?;
        let tx = match c.transaction_with_behavior(TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                let error = Error::new(
                    "authority-busy",
                    "Native return reservation metadata is busy",
                );
                return_begin_refused("prepare-begin-immediate", &error);
                return Ok(NativeConsultationReturnPreparation::Deferred(error));
            }
            Err(_) => return Err(Error::storage()),
        };
        if !self.authority_covers(&held, &selection.scopes(self, &tx, true)?) {
            return Err(Error::forbidden());
        }
        selection.validate_plan(&tx, &plan)?;
        let origin = &selection.origin;
        let row = rows::by_record(&tx, &origin.record)?;
        if let Some((attempt,effect,old_plan,old_selection,old_hash,started,runtime,credential,expires))=tx.query_row("SELECT n.attempt_id,n.effect_id,n.plan_json,n.selection_json,n.plan_digest,n.started,n.runtime_epoch,n.credential_id,o.lease_expires_at FROM native_consultation_returns n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.attempt_id IS NOT NULL",[&origin.record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?,r.get::<_,String>(6)?,r.get::<_,String>(7)?,r.get::<_,i64>(8)?))).optional().map_err(|_|Error::storage())? {
            if old_plan!=encoded || old_selection!=selected || old_hash!=hash || credential!=origin.credential{return Err(conflict());}
            if runtime!=self.runtime_epoch() || started || row.record.state!=OperationState::Preparing || expires<=crate::now(){return Ok(NativeConsultationReturnPreparation::NoWork);}
            tx.commit().map_err(|_|Error::storage())?;
            return Ok(NativeConsultationReturnPreparation::Reserved(NativeConsultationReturnLease{selection,attempt,effect,plan:encoded,selection_json:selected,hash,expires_at:expires}));
        }
        if row.record.state != OperationState::Accepted
            || !row.record.effects.is_empty()
            || row.generation >= 9_007_199_254_740_990
        {
            return Err(conflict());
        }
        if let Some(wait) = control_wait {
            wait.check()?;
        }
        if crate::now() >= original_until {
            return Err(Error::forbidden());
        }
        let attempt = format!("nra_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        let expires = (crate::now() + 60).min(origin.request.expires_at);
        tx.execute("UPDATE operations SET state='preparing',lease_owner=?2,lease_expires_at=?3,worker_generation=worker_generation+1,worker_runtime=?4,worker_cleanup=0 WHERE record_id=?1",params![origin.record,attempt,expires,self.transition_runtime]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-consultation-return',?2,'none','prepared')",params![effect,origin.record]).map_err(|_|Error::storage())?;
        let changed=tx.execute("UPDATE native_consultation_returns SET attempt_id=?2,effect_id=?3,selection_json=?4,plan_json=?5,plan_digest=?6 WHERE operation_record_id=?1 AND attempt_id IS NULL",params![origin.record,attempt,effect,selected,encoded,hash]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        touch(&tx, &origin.record, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(NativeConsultationReturnPreparation::Reserved(
            NativeConsultationReturnLease {
                selection,
                attempt,
                effect,
                plan: encoded,
                selection_json: selected,
                hash,
                expires_at: expires,
            },
        ))
    }
    pub fn native_consultation_return_attempt_read(
        &self,
        lease: &NativeConsultationReturnLease,
    ) -> Result<NativeReturnRead> {
        self.native_consultation_return_attempt_read_before(lease, i64::MAX)
    }
    /// Fresh current authority for this exact original reservation only. A
    /// pre-begin retry may narrow, never renew, its first actual read cutoff.
    pub fn native_consultation_return_attempt_read_before(
        &self,
        lease: &NativeConsultationReturnLease,
        original_until: i64,
    ) -> Result<NativeReturnRead> {
        {
            let c = self.try_connection()?;
            bound(&c, self, lease)?;
        }
        let mut read = lease.selection.read(self)?;
        read.limit(lease.expires_at.min(original_until))?;
        Ok(read)
    }
    /// Owned control admission for the same reserved return. No SQL guard spans
    /// file admission; original_until decreases even on later refusal.
    pub fn native_consultation_return_attempt_read_with_wait(
        &self,
        lease: &NativeConsultationReturnLease,
        original_until: &mut i64,
        wait: &crate::AuthorityWait,
    ) -> Result<NativeReturnRead> {
        *original_until = (*original_until).min(lease.expires_at);
        lease
            .selection
            .read_with_wait(self, original_until, wait, |c| {
                bound(c, self, lease).map(|_| ())
            })
    }
    /// A fresh complete selection remains held through the bounded Core call.
    /// SQL permits exactly one transition from prepared to possibly submitted.
    pub fn begin_native_consultation_return(
        &self,
        lease: NativeConsultationReturnLease,
        read: NativeReturnRead,
    ) -> Result<(NativeConsultationReturnAttempt, NativeReturnRead)> {
        self.try_begin_native_consultation_return_before(Arc::new(lease), read, i64::MAX)?
            .ok_or_else(|| Error::new("authority-busy", "Native return begin admission is busy"))
    }
    /// None means definite pre-effect admission failure only. The caller retains
    /// the SAME Arc lease and may obtain a fresh current read, never re-prepare
    /// or repeat a possibly started mutation. Every other error is terminal.
    pub fn try_begin_native_consultation_return_before(
        &self,
        lease: Arc<NativeConsultationReturnLease>,
        read: NativeReturnRead,
        original_until: i64,
    ) -> Result<Option<(NativeConsultationReturnAttempt, NativeReturnRead)>> {
        self.begin_native_consultation_return_admission(lease, read, original_until, None)
    }
    /// Wait for primary SQL before obtaining any mutation authority, then use
    /// only immediate file admission under SQL. No Writer ever parks on SQL.
    pub fn try_begin_native_consultation_return_with_wait(
        &self,
        lease: Arc<NativeConsultationReturnLease>,
        read: NativeReturnRead,
        original_until: i64,
        wait: &crate::AuthorityWait,
    ) -> Result<Option<(NativeConsultationReturnAttempt, NativeReturnRead)>> {
        self.begin_native_consultation_return_admission(lease, read, original_until, Some(wait))
    }
    fn begin_native_consultation_return_admission(
        &self,
        lease: Arc<NativeConsultationReturnLease>,
        mut read: NativeReturnRead,
        original_until: i64,
        control_wait: Option<&crate::AuthorityWait>,
    ) -> Result<Option<(NativeConsultationReturnAttempt, NativeReturnRead)>> {
        read.limit(lease.expires_at.min(original_until))?;
        if !read.current()
            || read.selection().snapshot() != lease.selection.snapshot()
            || read.origin().record_id() != lease.record_id()
            || read.origin().credential != lease.selection.origin.credential
        {
            return Err(Error::forbidden());
        }
        let (selection, routing, until) = read.into_mutation_selection(self)?;
        let (held, mut c) = if let Some(wait) = control_wait {
            let wait = wait.bounded(std::time::Duration::from_secs(
                until.saturating_sub(crate::now()).max(0) as u64,
            ));
            // No Read/Write survives into this wait: into_mutation_selection
            // consumed the original current read above.
            let c = self.try_connection_with_wait(&wait).map_err(|error| {
                return_begin_refused("control-begin-sql", &error);
                error
            })?;
            wait.check()?;
            let held = match self.try_acquire_authority(&routing) {
                Ok(held) => held,
                Err(error) if error.code == "authority-busy" => {
                    return_begin_refused("control-begin-file-immediate", &error);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            wait.check()?;
            (held, c)
        } else {
            let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(5));
            let held = match self.acquire_authority(&routing, &wait) {
                Ok(held) => held,
                Err(error) if error.code == "authority-busy" => {
                    return_begin_refused("file-admission", &error);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            let c = match self.try_connection() {
                Ok(c) => c,
                Err(error) if error.code == "authority-busy" => {
                    return_begin_refused("primary-sql", &error);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            (held, c)
        };
        if !self.authority_covers(&held, &selection.scopes(self, &c, true)?) {
            return Err(Error::new(
                "authority-busy",
                "Native return mutation scope changed",
            ));
        }
        c.bind_available_mutation_authority(self, &held)?;
        let tx = match c.transaction_with_behavior(TransactionBehavior::Immediate) {
            Ok(tx) => tx,
            Err(error)
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
                ) =>
            {
                return_begin_refused(
                    "begin-immediate",
                    &Error::new("authority-busy", "Native return begin metadata is busy"),
                );
                return Ok(None);
            }
            Err(_) => return Err(Error::storage()),
        };
        if !self.authority_covers(&held, &selection.scopes(self, &tx, true)?) {
            return Err(Error::forbidden());
        }
        // Construct the exact held control view BEFORE marking the original
        // effect started. No post-reservation fresh Read can lose custody.
        let read = self.native_return_read_in(&tx, selection, held, until)?;
        let row = bound(&tx, self, &lease)?;
        lease.selection.validate_plan(&tx, &lease.plan()?)?;
        if row.record.state != OperationState::Preparing {
            return Err(conflict());
        }
        // Last effect-free boundary: cancellation during fresh validation must
        // not mark the original reservation started. After DML, retain custody.
        if let Some(wait) = control_wait {
            wait.check()?;
        }
        let changed=tx.execute("UPDATE native_consultation_returns SET started=1 WHERE operation_record_id=?1 AND started=0",[lease.record_id()]).map_err(|_|Error::storage())?;
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
        Ok(Some((NativeConsultationReturnAttempt { lease }, read)))
    }
    /// An engine error after attempt admission is uncertainty, never permission
    /// to call Core again. The caller drops its bounded IO readers first.
    pub fn native_consultation_return_uncertain(
        &self,
        attempt: &NativeConsultationReturnAttempt,
    ) -> Result<()> {
        let lease = &attempt.lease;
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
        let (held, mut c) = self.native_return_recording_sql(&origin.record)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &origin.record)?;
        if !self.authority_covers(
            &held,
            &self.native_return_mutation_scopes(&tx, &row.record.record_id, false)?,
        ) {
            return Err(Error::forbidden());
        }
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND runtime_epoch=?4 AND plan_digest=?5 AND started=1)",params![origin.record,lease.attempt,lease.effect,origin.runtime,lease.hash],|r|r.get(0)).map_err(|_|Error::storage())?;
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
    pub fn record_native_consultation_return_applied(
        &self,
        receipt: &NativeConsultationReturnApplied,
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
        let (held, mut c) = self.native_return_recording_sql(&receipt.record)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        if !self.authority_covers(
            &held,
            &self.native_return_mutation_scopes(&tx, &row.record.record_id, false)?,
        ) {
            return Err(Error::forbidden());
        }
        admission::require_row(self, &held, &row)?;
        let existing:Option<Option<String>>=tx.query_row("SELECT applied_json FROM native_consultation_returns WHERE operation_record_id=?1 AND attempt_id=?2 AND effect_id=?3 AND plan_digest=?4 AND runtime_epoch=?5 AND started=1",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
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
            tx.execute("UPDATE native_consultation_returns SET applied_json=?2 WHERE operation_record_id=?1",params![receipt.record,receipt.observation]).map_err(|_|Error::storage())?;
            let changed=tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&receipt.effect]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
            let (plan,selected):(String,String)=tx.query_row("SELECT plan_json,selection_json FROM native_consultation_returns WHERE operation_record_id=?1",[&receipt.record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
            let plan: serde_json::Value =
                serde_json::from_str(&plan).map_err(|_| Error::storage())?;
            let selected: serde_json::Value =
                serde_json::from_str(&selected).map_err(|_| Error::storage())?;
            let observed: serde_json::Value =
                serde_json::from_str(&receipt.observation).map_err(|_| Error::storage())?;
            for binding in selected.as_array().ok_or_else(Error::storage)? {
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
                let Some(revision) = observed["receiver_revisions"][runtime].as_u64() else {
                    continue;
                };
                if matches!(
                    binding["source_kind"].as_str(),
                    Some("initial-sip" | "private-initial-sip" | "established-sip")
                ) {
                    if self.record_sip_audience_revision(
                        &tx,
                        binding,
                        &receipt.runtime,
                        previous,
                        revision,
                    )? {
                        crate::native_stream_projection::audience(
                            &tx,
                            id,
                            generation,
                            previous,
                            revision,
                            receipt.metadata.get(runtime).ok_or_else(invalid)?,
                        )?;
                    }
                    continue;
                }
                if binding.get("source_kind").is_some() {
                    return Err(Error::storage());
                }
                // Historical metadata never revives a retired/replaced generation
                // or rewrites a later native observation.
                let changed=tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0)",params![id,generation,receipt.runtime,revision,previous]).map_err(|_|Error::storage())?;
                if changed == 1 {
                    crate::native_stream_projection::audience(
                        &tx,
                        id,
                        generation,
                        previous,
                        revision,
                        receipt.metadata.get(runtime).ok_or_else(invalid)?,
                    )?;
                }
            }
            tx.execute("UPDATE native_private_lifetimes SET restored_at=?2 WHERE operation_record_id=(SELECT consultation_record_id FROM native_consultation_returns WHERE operation_record_id=?1) AND restored_at IS NULL",params![receipt.record,crate::now()]).map_err(|_|Error::storage())?;
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

impl Store {
    /// Completion waits for captured retirement of the original typed candidate. It does not
    /// infer a continuing remote call from successful audience restoration.
    pub fn complete_native_consultation_return(
        &self,
        receipt: &NativeConsultationReturnApplied,
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
        let wait = crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        let mut c = self.try_connection_with_wait(&wait)?;
        let completion_scopes = |c: &Connection| -> Result<Vec<crate::AuthorityScope>> {
            let row = rows::by_record(&c, &receipt.record)?;
            let mut scopes = admission::row_scopes(&row)?;
            if row.record.state != OperationState::Completed {
                scopes.extend(crate::authority_admission::conversation_scopes(
                    &c,
                    row.record
                        .conversation_id
                        .as_deref()
                        .ok_or_else(Error::storage)?,
                    crate::AuthorityMode::Read,
                )?);
                scopes.extend(self.native_return_mutation_scopes(&c, &receipt.record, true)?);
                let context: String = c.query_row("SELECT context_selection_json FROM native_consultation_returns WHERE operation_record_id=?1",[&receipt.record],|r|r.get(0)).map_err(|_|Error::storage())?;
                let context: serde_json::Value =
                    serde_json::from_str(&context).map_err(|_| Error::storage())?;
                for port in context["private"]
                    .as_array()
                    .ok_or_else(Error::storage)?
                    .iter()
                    .chain(std::iter::once(&context["remote"]))
                {
                    if port.get("source_kind").is_some() {
                        scopes.extend(Store::sip_completion_scopes(port)?);
                    }
                }
            }
            crate::AuthorityScope::merged(&scopes)
        };
        let scopes = completion_scopes(&c)?;
        let held = self.try_acquire_authority(&scopes)?;
        c.bind_available_mutation_authority(self, &held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        // Preserve the complete post-admission footprint check, including
        // current membership and typed SIP completion dependencies.
        if !self.authority_covers(&held, &completion_scopes(&tx)?) {
            return Err(Error::forbidden());
        }
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        if row.tombstoned {
            return Err(Error::new("gone", "Operation progress has expired"));
        }
        let original:Option<(String,String,String)>=tx.query_row("SELECT r.consultation_record_id,r.context_selection_json,l.connection_id FROM native_consultation_returns r JOIN consultation_candidate_bindings l ON l.operation_record_id=r.consultation_record_id JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.operation_record_id=?1 AND r.attempt_id=?2 AND r.effect_id=?3 AND r.plan_digest=?4 AND r.runtime_epoch=?5 AND r.started=1 AND r.applied_json=?6",params![receipt.record,receipt.attempt,receipt.effect,receipt.hash,receipt.runtime,receipt.observation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let (parent, context, candidate) = original.ok_or_else(Error::forbidden)?;
        if row.record.state == OperationState::Completed {
            return Ok(Some(row.record));
        }
        if !self.authority_covers(
            &held,
            &self.native_return_mutation_scopes(&tx, &receipt.record, true)?,
        ) {
            return Err(Error::forbidden());
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let context: serde_json::Value =
            serde_json::from_str(&context).map_err(|_| Error::storage())?;
        let original_candidate = context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .find(|p| p["connection_id"] == candidate)
            .ok_or_else(Error::storage)?;
        let generation = original_candidate["generation"]
            .as_u64()
            .ok_or_else(Error::storage)?;
        let retired = if original_candidate.get("source_kind").is_some() {
            Store::recorded_sip_departure(&tx, original_candidate, &receipt.runtime)?
        } else {
            tx.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN connections v ON v.id=l.connection_id WHERE l.connection_id=?1 AND l.generation=?2 AND l.runtime_epoch=?3 AND l.retired=1 AND v.state IN ('retired','failed'))",params![candidate,generation,receipt.runtime],|r|r.get(0)).map_err(|_|Error::storage())?
        };
        if !retired {
            return Ok(None);
        }
        let request = rows::request(&row)?;
        let mut current_actors = vec![];
        let mut currently_authorized = true;
        let mut retained = Vec::<String>::new();
        let mut retired = vec![candidate.clone()];
        for port in context["private"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .chain(std::iter::once(&context["remote"]))
        {
            let id = port["connection_id"].as_str().ok_or_else(Error::storage)?;
            if id == candidate {
                continue;
            }
            if port.get("source_kind").is_some() {
                if Store::recorded_sip_live(&tx, port, &receipt.runtime)? {
                    retained.push(id.into());
                    match self.sip_completion_actors(
                        &tx,
                        port,
                        &request.conversation_id,
                        &request.session_id,
                        &held,
                    )? {
                        Some(actors) => current_actors.push((
                            port["identity_id"]
                                .as_str()
                                .ok_or_else(Error::storage)?
                                .to_owned(),
                            actors,
                        )),
                        None => currently_authorized = false,
                    }
                } else {
                    retired.push(id.into());
                }
                continue;
            }
            let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN connection_offers m ON m.connection_id=v.id JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 WHERE v.id=?1 AND v.generation=?2 AND v.participant_id=?3 AND v.membership_epoch=?4 AND v.observed_runtime_id=?5 AND m.runtime_connection_id=?6 AND v.state IN ('ready','active'))",params![id,port["generation"].as_u64().ok_or_else(Error::storage)?,port["participant_id"].as_str().ok_or_else(Error::storage)?,port["membership_epoch"].as_u64().ok_or_else(Error::storage)?,receipt.runtime,port["runtime_connection_id"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
            if live {
                let credential = port["credential_id"].as_str().ok_or_else(Error::storage)?;
                let actor = crate::identity::actor_from_connection(&tx, credential);
                let key_current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM credential_signing_keys binding JOIN peer_signing_keys k ON k.key_id=binding.key_id JOIN devices d ON d.id=k.device_id AND d.revoked=0 JOIN endpoints ep ON ep.device_id=d.id AND ep.identity_id=k.identity_id AND ep.verification_state='verified' WHERE binding.credential_id=?1 AND k.key_id=?2 AND k.revoked=0 AND k.not_before<=?3 AND k.expires_at>?3 AND ep.id=?4 AND d.id=?5)",params![credential,port["key_id"].as_str().ok_or_else(Error::storage)?,crate::now(),port["endpoint_id"].as_str().ok_or_else(Error::storage)?,port["device_id"].as_str().ok_or_else(Error::storage)?],|r|r.get(0)).map_err(|_|Error::storage())?;
                match actor {
                    Ok(actor)
                        if key_current
                            && actor.permits("connection.offer")
                            && actor.permits("stream.subscribe") =>
                    {
                        current_actors.push((actor.identity_id.clone(), vec![actor]))
                    }
                    _ => currently_authorized = false,
                }
                retained.push(id.into());
            } else {
                retired.push(id.into());
            }
        }
        retained.sort();
        retired.sort();
        let continuing = retained.len() == 2
            && currently_authorized
            && current_actors.len() == 2
            && current_actors.iter().all(|(_, actors)| {
                current_actors.iter().all(|(identity, _)| {
                    actors.iter().all(|actor| actor.permits_recipient(identity))
                })
            });
        let revision = request
            .expected_revision
            .checked_add(1)
            .filter(|r| *r < 9_007_199_254_740_991)
            .ok_or_else(conflict)?;
        if tx.execute("UPDATE sessions SET topology_revision=?2,updated_at=?3 WHERE id=?1 AND topology_revision=?4",params![request.session_id,revision,crate::now(),request.expected_revision]).map_err(|_|Error::storage())?!=1{return Err(conflict());}
        let result = serde_json::json!({"consultation_record_id":parent,"retained_connection_ids":retained,"admitted_connection_ids":[],"retired_connection_ids":retired,"session_revision":revision,"conversation_revision":null,"remote_continuing":continuing});
        if row.record.state == OperationState::Preparing {
            tx.execute(
                "UPDATE operations SET state='committing' WHERE record_id=?1",
                [&receipt.record],
            )
            .map_err(|_| Error::storage())?;
            touch(&tx, &receipt.record, self)?;
        }
        tx.execute(
            "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
            params![
                receipt.record,
                crate::provisioning::canonical_json(&result)?
            ],
        )
        .map_err(|_| Error::storage())?;
        touch(&tx, &receipt.record, self)?;
        workers::invalidate(&tx, &receipt.record)?;
        // A pending engagement cannot subsequently establish another lifetime
        // after a separately authorized, confirmed return.
        tx.execute("UPDATE operations SET state='cancelled' WHERE record_id=?1 AND state IN ('preparing','unknown','interrupted')",[&parent]).map_err(|_|Error::storage())?;
        if tx.changes() > 0 {
            touch(&tx, &parent, self)?;
            workers::invalidate(&tx, &parent)?;
        }
        let session =
            crate::session_control::view(&tx, &request.conversation_id, &request.session_id)?;
        crate::session_control::emit_transition(
            &tx,
            &row.owner,
            &row.record.operation_id,
            &request.conversation_id,
            &request.session_id,
            "session.consult.return",
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
}
