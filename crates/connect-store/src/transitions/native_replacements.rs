//! Durable managed-preparation custody. This API cannot allocate or dial a peer.
use super::*;
use crate::{NativeReplacementPreparationRead, NativeReplacementSelection};
pub use allocation::{
    NativeReplacementAllocated, NativeReplacementAllocationAttempt,
    NativeReplacementAllocationRead, NativeReplacementAudienceCapture,
    NativeReplacementCommitAttempt, NativeReplacementCommitNotSubmitted,
    NativeReplacementCommitUncertain, NativeReplacementCommitted, NativeReplacementMediaAttempt,
    NativeReplacementMediaCapture, NativeReplacementMediaOrigin, NativeReplacementMediaPhase,
    NativeReplacementMediaRead, NativeReplacementSceneRead, NativeReplacementStartAttempt,
    NativeReplacementStartOrigin, NativeReplacementStartRead, NativeReplacementStartUncertain,
    NativeReplacementStarted,
};
use serde::{Deserialize, Serialize};
use std::sync::Weak;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ReplacementConfirmation {
    MediaOnly,
    AnsweredDtmf { digits: String },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacementPreparationRequest {
    pub candidate_connection_id: String,
    pub participant_revision: u64,
    pub timeout_seconds: u32,
    pub confirmation: ReplacementConfirmation,
    pub telephone_policy: Option<TelephoneReplacementPolicy>,
}
pub struct NativeReplacementPreparationLease {
    selection: NativeReplacementSelection,
    reservation: String,
    attempt: String,
    effect: String,
    plan: String,
    selected: String,
    hash: String,
    expires: i64,
}
pub struct NativeReplacementPreparationAttempt {
    lease: NativeReplacementPreparationLease,
}
#[derive(Clone)]
pub struct NativeReplacementPrepared {
    selection: NativeReplacementSelection,
    owner: Weak<crate::InstallationOwnership>,
    runtime: String,
    record: String,
    reservation: String,
    attempt: String,
    effect: String,
    hash: String,
    observation: String,
}
macro_rules! redacted {($($name:ident),*)=>{$(impl std::fmt::Debug for $name {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($name),"(redacted)"))}})*}}
pub(super) mod candidate;
pub use candidate::{NativeReplacementCandidate, NativeReplacementCandidateRetirement};
mod allocation;
mod telephone;
pub use telephone::{
    TelephoneAudioResources, TelephoneConnectionRuntimeObservation, TelephoneConnectionSnapshot,
    TelephoneMediaOrigin, TelephoneMediaRead, TelephoneNativeMediaRead, TelephoneNativeSignalRead,
    TelephoneReplacementAllocated, TelephoneReplacementAttempt,
    TelephoneReplacementAudienceCapture, TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
    TelephoneReplacementMediaOrigin, TelephoneReplacementMediaRead, TelephoneReplacementPhase,
    TelephoneReplacementPhaseAttempt, TelephoneReplacementPhaseCapture, TelephoneReplacementPolicy,
    TelephoneReplacementRead, TelephoneReplacementResourcesAttempt,
    TelephoneReplacementResourcesCapture, TelephoneReplacementSceneRead,
    TelephoneReplacementSubmitted, TelephoneReplacementUncertain, TelephoneReplacementWriteRead,
    TelephoneSignalDirection, TelephoneSignalRead,
};
redacted!(
    NativeReplacementPreparationLease,
    NativeReplacementPreparationAttempt,
    NativeReplacementPrepared
);
impl NativeReplacementPreparationLease {
    fn snapshot(&self) -> Self {
        Self {
            selection: self.selection.clone(),
            reservation: self.reservation.clone(),
            attempt: self.attempt.clone(),
            effect: self.effect.clone(),
            hash: self.hash.clone(),
            selected: self.selected.clone(),
            plan: self.plan.clone(),
            expires: self.expires,
        }
    }
    pub fn record_id(&self) -> &str {
        self.selection.origin.record_id()
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.plan).map_err(|_| Error::storage())
    }
}
impl NativeReplacementPreparationAttempt {
    pub fn record_id(&self) -> &str {
        self.lease.record_id()
    }
    pub fn plan(&self) -> Result<serde_json::Value> {
        self.lease.plan()
    }
    /// Trusted runtime capture of the real owned Core preparation ticket, not a
    /// wire observation or resource-ready assertion. No media is ready here.
    pub fn prepared(self, observation: serde_json::Value) -> Result<NativeReplacementPrepared> {
        let plan = self.lease.plan()?;
        if observation != plan["expected_preparation"] {
            return Err(conflict());
        }
        let origin = &self.lease.selection.origin;
        Ok(NativeReplacementPrepared {
            selection: self.lease.selection.clone(),
            owner: origin.owner.clone(),
            runtime: origin.runtime.clone(),
            record: origin.record.clone(),
            reservation: self.lease.reservation,
            attempt: self.lease.attempt,
            effect: self.lease.effect,
            hash: self.lease.hash,
            observation: crate::provisioning::canonical_json(&observation)?,
        })
    }
}
fn validate_request(request: &ReplacementPreparationRequest) -> Result<()> {
    id(&request.candidate_connection_id)?;
    if !(1..=60).contains(&request.timeout_seconds)
        || request.participant_revision >= 9_007_199_254_740_990
    {
        return Err(invalid());
    }
    if let ReplacementConfirmation::AnsweredDtmf { digits } = &request.confirmation {
        if digits.is_empty()
            || digits.len() > 16
            || !digits
                .bytes()
                .all(|b| b.is_ascii_digit() || b"*#ABCD".contains(&b))
        {
            return Err(invalid());
        }
    }
    Ok(())
}
fn plan(
    store: &Store,
    c: &Connection,
    selection: &NativeReplacementSelection,
    request: &ReplacementPreparationRequest,
    runtime: &str,
) -> Result<serde_json::Value> {
    validate_request(request)?;
    let ctx = selection.context(store, c)?;
    plan_with_packet_context(store, c, selection, request, runtime, &ctx)
}
// Context is borrowed only during an exact-version packet capture. This is
// pure plan construction, never an authority proof or reusable media grant.
fn plan_with_packet_context(
    store: &Store,
    c: &Connection,
    selection: &NativeReplacementSelection,
    request: &ReplacementPreparationRequest,
    runtime: &str,
    ctx: &crate::NativeReplacementContext,
) -> Result<serde_json::Value> {
    validate_request(request)?;
    let candidate = &request.candidate_connection_id;
    if candidate == &ctx.source_connection_id
        || candidate == &ctx.retained_connection_id
        || ctx
            .candidate_connection_id
            .as_ref()
            .is_some_and(|id| id != candidate)
    {
        return Err(conflict());
    }
    let telephone_route = telephone::route(store, c, selection, &ctx, request)?;
    let generation = ctx
        .source_generation
        .checked_add(1)
        .filter(|g| *g < 9_007_199_254_740_991)
        .ok_or_else(conflict)?;
    let expected = serde_json::json!({"operation_id":ctx.operation_record_id,"session":ctx.session_id,"participant":ctx.participant_id,"membership_epoch":ctx.membership_epoch,"source":ctx.source_runtime_connection_id,"retained":ctx.retained_runtime_connection_id,"candidate":runtime,"endpoint":ctx.target_endpoint_id,"device":ctx.target_device_id,"generation":generation,"participant_revision":request.participant_revision});
    let mut result = serde_json::json!({"operation_id":ctx.operation_record_id,"candidate_connection_id":candidate,"candidate_runtime_id":runtime,"generation":generation,"source_generation":ctx.source_generation,"source_role":ctx.role,"target_endpoint_revision":ctx.target_endpoint_revision,"target_device_kind":ctx.target_device_kind,"target_device_platform":ctx.target_device_platform,"confirmation":request.confirmation,"timeout_seconds":request.timeout_seconds,"expected_preparation":expected});
    if let TransitionIntent::Replace {
        video: Some(video), ..
    } = &selection.origin.request().intent
    {
        video.validate()?;
        if telephone_route.is_some()
            || matches!(
                selection.source(),
                crate::NativeReplacementSourceOrigin::Telephone(_)
            )
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "Video replacement requires native source and target resources",
            ));
        }
        result["video"] = serde_json::to_value(video).map_err(|_| Error::storage())?;
    }
    if let Some(route) = telephone_route {
        result["telephone_policy"] =
            serde_json::to_value(&request.telephone_policy).map_err(|_| Error::storage())?;
        result["telephone_route"] = route;
    }
    Ok(result)
}
fn local_owner(
    store: &Store,
    weak: &Weak<crate::InstallationOwnership>,
    runtime: &str,
) -> Result<()> {
    let owner = store.require_runtime_owner()?;
    if runtime != store.runtime_epoch()
        || !owner.current()
        || !weak.upgrade().is_some_and(|v| Arc::ptr_eq(&owner, &v))
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn bound(
    c: &Connection,
    store: &Store,
    lease: &NativeReplacementPreparationLease,
) -> Result<rows::Saved> {
    let origin = &lease.selection.origin;
    local_owner(store, &origin.owner, &origin.runtime)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.reservation_id=?2 AND s.attempt_id=?3 AND s.effect_id=?4 AND n.runtime_epoch=?5 AND n.credential_id=?6 AND n.plan_digest=?7 AND n.plan_json=?8 AND n.selection_json=?9 AND n.expires_at=?10 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at)",params![origin.record,lease.reservation,lease.attempt,lease.effect,origin.runtime,origin.credential,lease.hash,lease.plan,lease.selected,lease.expires],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact || crate::now() >= lease.expires {
        return Err(Error::forbidden());
    }
    rows::by_record(c, &origin.record)
}
fn preparation_refused(stage: &'static str, error: Error) -> Error {
    tracing::debug!(target:"connect_store::native_replacement_preparation",stage,code=%error.code,"Replacement preparation metadata refused");
    error
}
impl Store {
    /// Exact persisted ledger footprint, not a current execution/IO grant.
    /// Callers retain/rederive their separate current selection Read facts.
    /// No identity/key/member row changes with these operation-local writes.
    pub(crate) fn native_replacement_bookkeeping_scopes(
        &self,
        c: &Connection,
        record: &str,
    ) -> Result<(String, String, Vec<crate::AuthorityScope>)> {
        let row = rows::by_record(c, record)?;
        let request = rows::request(&row)?;
        if !row.ledger
            || row.record.command_type != "connection.replace"
            || !matches!(request.intent, TransitionIntent::Replace { .. })
            || row.record.conversation_id.as_deref() != Some(request.conversation_id.as_str())
        {
            return Err(Error::forbidden());
        }
        let cid = request.conversation_id;
        let mut scopes = vec![
            crate::AuthorityScope::identity(&row.owner, crate::AuthorityMode::Read)?,
            crate::AuthorityScope::conversation(&cid, crate::AuthorityMode::Write)?,
        ];
        // Same transitive footprint as the persistent operations trigger. A
        // matching enrollment can depend on this exact operation even though
        // its subject is not a member of this Conversation. Usually empty for
        // replacement, but never silently omit a present dependent subject.
        let mut query = c.prepare_cached(
            "SELECT e.identity_id FROM client_enrollments e JOIN operations o ON o.tenant_id=e.tenant_id AND o.owner_id=e.issuer_identity_id AND o.operation_id=e.complete_operation_id WHERE o.record_id=?1"
        ).map_err(|_| Error::storage())?;
        let subjects = query
            .query_map([record], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?;
        for subject in subjects {
            scopes.push(crate::AuthorityScope::identity(
                &subject.map_err(|_| Error::storage())?,
                crate::AuthorityMode::Write,
            )?);
        }
        Ok((cid, row.owner, crate::AuthorityScope::merged(&scopes)?))
    }
    fn native_replacement_reservation_scopes(
        &self,
        c: &Connection,
        selection: &NativeReplacementSelection,
    ) -> Result<Vec<crate::AuthorityScope>> {
        // resolve remains the exact current actor/key/endpoint/source/retained
        // check, with all original Identity Reads. Only actual ledger changes
        // gain Write; no conversation-member Identity is blindly promoted.
        let mut scopes = selection.scopes(self, c, false)?;
        let (cid, _, mutation) =
            self.native_replacement_bookkeeping_scopes(c, selection.origin.record_id())?;
        if cid != selection.origin.request.conversation_id {
            return Err(Error::forbidden());
        }
        scopes.extend(mutation);
        crate::AuthorityScope::merged(&scopes)
    }
    /// Wait for primary SQL without holding file authority. If the mutex is
    /// taken again after file admission, release the complete proof before
    /// waiting for the same original request. This cannot retry a mutation.
    fn native_replacement_preparation_sql<'a>(
        &'a self,
        wait: &crate::AuthorityWait,
        mutation: bool,
        authority_stage: &'static str,
        sql_stage: &'static str,
        scopes: impl Fn(&Connection) -> Result<Vec<crate::AuthorityScope>>,
    ) -> Result<(
        Arc<crate::AuthorityGuardSet>,
        crate::installation_ownership::ConnectionGuard<'a>,
    )> {
        loop {
            let scopes = {
                let c = self
                    .try_connection_with_wait(wait)
                    .map_err(|error| preparation_refused(sql_stage, error))?;
                scopes(&c)?
            };
            let held = self
                .acquire_authority(&scopes, wait)
                .map_err(|error| preparation_refused(authority_stage, error))?;
            #[cfg(test)]
            crate::authority_admission::observe_passive_admission(true);
            let c = match self.try_connection() {
                Err(error)
                    if error.code == "authority-busy"
                        && error.message == "Current state is busy" =>
                {
                    drop(held);
                    continue;
                }
                result => result.map_err(|error| preparation_refused(sql_stage, error))?,
            };
            wait.check()?;
            if mutation {
                c.bind_available_mutation_authority(self, &held)?;
            }
            return Ok((held, c));
        }
    }

    /// Reserve the original managed preparation before Core is called. An
    /// unused generated target can be born under current selection Reads;
    /// existing or explicitly named candidate lineages still need Write.
    pub fn prepare_native_replacement(
        &self,
        read: NativeReplacementPreparationRead,
        request: ReplacementPreparationRequest,
    ) -> Result<Option<NativeReplacementPreparationLease>> {
        validate_request(&request)?;
        let fresh_target = read.context().candidate_connection_id.is_none()
            && request.candidate_connection_id
                == format!("replacement_{}", read.selection().origin.record_id());
        let selection = read.into_selection()?;
        // Consume input Reads before rederiving current file scopes. Existing
        // candidate linkage participates in private-floor facts before commit.
        let scopes = |c: &Connection| {
            if fresh_target {
                selection.scopes(self, c, false)
            } else {
                self.native_replacement_reservation_scopes(c, &selection)
            }
        };
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(250));
        let (held, mut c) = self.native_replacement_preparation_sql(
            &wait,
            !fresh_target,
            "reservation-authority",
            "reservation-sql",
            &scopes,
        )?;
        let replay = fresh_target
            && c.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_replacements WHERE operation_record_id=?1)",
                [selection.origin.record_id()],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| Error::storage())?;
        let behavior = if replay {
            TransactionBehavior::Deferred
        } else {
            TransactionBehavior::Immediate
        };
        let tx = c
            .transaction_with_behavior(behavior)
            .map_err(|error| {
                tracing::debug!(target:"connect_store::native_candidate",stage="reservation-transaction",sqlite_code=?error.sqlite_error_code(),"Replacement metadata deferred");
                Error::new("authority-busy", "Replacement reservation metadata is busy")
            })?;
        if !self.authority_covers(&held, &scopes(&tx)?) {
            return Err(Error::forbidden());
        }
        let origin = &selection.origin;
        let row = rows::by_record(&tx, &origin.record)?;
        let existing:Option<(String,String,String,String,String,String,String,i64,bool)>=tx.query_row("SELECT n.reservation_id,n.candidate_runtime_id,n.plan_json,n.selection_json,n.plan_digest,s.attempt_id,s.effect_id,n.expires_at,s.started FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' WHERE n.operation_record_id=?1",[&origin.record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|_|Error::storage())?;
        let selected = crate::provisioning::canonical_json(&selection.snapshot())?;
        if let Some((
            reservation,
            runtime,
            old_plan,
            old_selected,
            hash,
            attempt,
            effect,
            expires,
            started,
        )) = existing
        {
            let (_, encoded) = digest(&plan(self, &tx, &selection, &request, &runtime)?)?;
            if old_plan != encoded || old_selected != selected {
                return Err(conflict());
            }
            if started || row.record.state != OperationState::Preparing || expires <= crate::now() {
                return Ok(None);
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(Some(NativeReplacementPreparationLease {
                selection,
                reservation,
                attempt,
                effect,
                plan: encoded,
                selected,
                hash,
                expires,
            }));
        }
        if replay {
            return Err(conflict());
        }
        let claimed = candidate::prepared(&tx, origin)?;
        if (!(row.record.state == OperationState::Accepted && row.record.effects.is_empty())
            && !(claimed && row.record.state == OperationState::Preparing))
            || row.generation >= 9_007_199_254_740_990
        {
            return Err(conflict());
        }
        let context = selection.context(self, &tx)?;
        if context.candidate_connection_id.is_some() {
            if !claimed {
                return Err(conflict());
            }
            let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_candidates q JOIN connections v ON v.id=q.candidate_connection_id JOIN connection_offers m ON m.connection_id=v.id WHERE q.operation_record_id=?1 AND v.id=?2 AND v.state='retired' AND v.generation=json_extract(q.claim_json,'$.generation')+1 AND v.observed_runtime_id IS NULL AND v.readiness_json IS NULL AND m.runtime_connection_id=json_extract(q.claim_json,'$.core') AND ?3+1>=v.generation)",params![origin.record,request.candidate_connection_id,context.source_generation],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !valid {
                return Err(conflict());
            }
        } else {
            let used:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1) OR EXISTS(SELECT 1 FROM native_replacements WHERE candidate_connection_id=?1)",[&request.candidate_connection_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if used {
                return Err(conflict());
            }
        }
        let runtime = format!("cre_{}", uuid::Uuid::new_v4());
        let value = plan(self, &tx, &selection, &request, &runtime)?;
        let (hash, encoded) = digest(&value)?;
        if encoded.len() > 32768 || selected.len() > 32768 {
            return Err(invalid());
        }
        let actor = origin.require(self, &tx)?;
        let expires = (crate::now() + i64::from(request.timeout_seconds))
            .min(origin.request.expires_at)
            .min(actor.expires_at)
            .min(selection.physical_expiry());
        let reservation = format!("nrr_{}", uuid::Uuid::new_v4());
        let attempt = format!("nrp_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        let generation = value["generation"].as_u64().ok_or_else(invalid)?;
        let permit = if fresh_target {
            use crate::authority_writer_boundary::phase_ledger::reservation::Reservation;
            Some(
                self.authority_writer_boundary
                    .native_fresh_reservation_ledger(
                        self,
                        &tx,
                        &held,
                        Reservation {
                            record: &origin.record,
                            credential: &origin.credential,
                            runtime: &origin.runtime,
                            original_digest: &origin.digest,
                            reservation: &reservation,
                            candidate: &request.candidate_connection_id,
                            candidate_runtime: &runtime,
                            generation,
                            plan: &encoded,
                            selection: &selected,
                            plan_digest: &hash,
                            expires,
                            attempt: &attempt,
                            effect: &effect,
                        },
                    )?,
            )
        } else {
            None
        };
        tx.execute("UPDATE operations SET state='preparing',lease_owner=?2,lease_expires_at=?3,worker_generation=worker_generation+1,worker_runtime=?4,worker_cleanup=0 WHERE record_id=?1",params![origin.record,reservation,expires,self.transition_runtime]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacements(operation_record_id,credential_id,runtime_epoch,original_digest,reservation_id,candidate_connection_id,candidate_runtime_id,generation,plan_json,selection_json,plan_digest,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![origin.record,origin.credential,origin.runtime,origin.digest,reservation,request.candidate_connection_id,runtime,generation,encoded,selected,hash,expires]).map_err(|e|if matches!(e,rusqlite::Error::SqliteFailure(ref v,_) if v.code==rusqlite::ErrorCode::ConstraintViolation){conflict()}else{Error::storage()})?;
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state) VALUES(?1,?2,'native-replacement-prepare',?2,'none','prepared')",params![effect,origin.record]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'prepare',?2,?3)",params![origin.record,attempt,effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &origin.record, self)?;
        drop(permit);
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(NativeReplacementPreparationLease {
            selection,
            reservation,
            attempt,
            effect,
            plan: encoded,
            selected,
            hash,
            expires,
        }))
    }
    pub fn native_replacement_preparation_attempt_read(
        &self,
        lease: &NativeReplacementPreparationLease,
    ) -> Result<NativeReplacementPreparationRead> {
        {
            let c = self
                .try_connection()
                .map_err(|e| preparation_refused("read-sql", e))?;
            bound(&c, self, lease).map_err(|e| preparation_refused("read-reservation", e))?;
        }
        let mut read = lease
            .selection
            .read(self)
            .map_err(|e| preparation_refused("read-selection", e))?;
        read.limit(lease.expires)
            .map_err(|e| preparation_refused("read-interval", e))?;
        Ok(read)
    }
    /// Owned control-only resolution for the SAME original reservation. The
    /// caller supplies its original cutoff/cancellation wait and joins this
    /// work before beginning. `original_until` only decreases, including when
    /// first facts resolve but a later admission refuses. This never retries
    /// SQL statements or any effect.
    pub fn native_replacement_preparation_attempt_read_with_wait(
        &self,
        lease: &NativeReplacementPreparationLease,
        original_until: &mut i64,
        wait: &crate::AuthorityWait,
    ) -> Result<NativeReplacementPreparationRead> {
        *original_until = (*original_until).min(lease.expires);
        lease
            .selection
            .read_with_wait(self, wait, original_until, |c| {
                bound(c, self, lease)
                    .map(|_| ())
                    .map_err(|e| preparation_refused("control-reservation", e))
            })
    }
    pub fn begin_native_replacement_preparation(
        &self,
        lease: &NativeReplacementPreparationLease,
        read: NativeReplacementPreparationRead,
    ) -> Result<(
        NativeReplacementPreparationAttempt,
        NativeReplacementPreparationRead,
    )> {
        let until = read.expires_at();
        self.begin_native_replacement_preparation_before(lease, read, until)
    }
    /// Retry admission for this SAME reserved, not-yet-begun attempt. The
    /// original caller's interval can only shorten the fresh read; a committed
    /// begun phase still refuses replay before Core may be invoked again.
    pub fn begin_native_replacement_preparation_before(
        &self,
        lease: &NativeReplacementPreparationLease,
        mut read: NativeReplacementPreparationRead,
        original_until: i64,
    ) -> Result<(
        NativeReplacementPreparationAttempt,
        NativeReplacementPreparationRead,
    )> {
        read.limit(lease.expires.min(original_until))
            .map_err(|e| preparation_refused("begin-interval", e))?;
        if read.selection().snapshot() != lease.selection.snapshot()
            || read.selection().origin.record != lease.selection.origin.record
            || read.selection().origin.credential != lease.selection.origin.credential
        {
            return Err(preparation_refused("begin-selection", Error::forbidden()));
        }
        let expires = read.expires_at();
        // Release the old Read before primary SQL wait. Begin only submits
        // the exact preparation effect; fresh complete source/selection Reads
        // remain valid through synchronous Core capacity preparation.
        let selection = read.into_selection()?;
        let remaining =
            std::time::Duration::from_secs(expires.saturating_sub(crate::now()).max(0) as u64);
        let wait = crate::AuthorityWait::new(remaining.min(std::time::Duration::from_millis(250)));
        let (held, mut c) = self.native_replacement_preparation_sql(
            &wait,
            false,
            "begin-authority",
            "begin-sql",
            |c| selection.scopes(self, c, false),
        )?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| preparation_refused("begin-transaction", Error::storage()))?;
        // Revalidate the complete current selection and exact original
        // attempt in this transaction before the once-only ledger mutation.
        let read = selection
            .read_with_connection(self, &tx, held.clone(), expires)
            .map_err(|e| preparation_refused("begin-current-selection", e))?;
        let row =
            bound(&tx, self, &lease).map_err(|e| preparation_refused("begin-reservation", e))?;
        lease
            .selection
            .origin
            .require(self, &tx)
            .map_err(|e| preparation_refused("begin-original", e))?;
        let saved = lease.plan()?;
        let request = ReplacementPreparationRequest {
            candidate_connection_id: saved["candidate_connection_id"]
                .as_str()
                .ok_or_else(invalid)?
                .into(),
            participant_revision: saved["expected_preparation"]["participant_revision"]
                .as_u64()
                .ok_or_else(invalid)?,
            timeout_seconds: saved["timeout_seconds"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(invalid)?,
            telephone_policy: saved
                .get("telephone_policy")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::storage())?,
            confirmation: serde_json::from_value(saved["confirmation"].clone())
                .map_err(|_| invalid())?,
        };
        let runtime = saved["candidate_runtime_id"].as_str().ok_or_else(invalid)?;
        if plan(self, &tx, &lease.selection, &request, runtime)
            .map_err(|e| preparation_refused("begin-plan", e))?
            != saved
        {
            return Err(conflict());
        }

        if row.record.state != OperationState::Preparing {
            return Err(conflict());
        }
        let started: bool = tx.query_row(
            "SELECT started FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='prepare'",
            [lease.record_id()], |row| row.get(0),
        ).map_err(|_|Error::storage())?;
        if started {
            return Err(Error::new(
                "reconciliation-required",
                "Native replacement preparation already attempted",
            ));
        }
        use crate::authority_writer_boundary::phase_ledger;
        let permit = self.authority_writer_boundary.native_phase_ledger(
            self,
            &tx,
            &held,
            phase_ledger::Key {
                record: lease.record_id().to_owned(),
                phase: "prepare",
                attempt: lease.attempt.clone(),
                effect: lease.effect.clone(),
                expected: String::new(),
                runtime: lease.selection.origin.runtime.clone(),
                reservation: lease.reservation.clone(),
                digest: lease.hash.clone(),
                stage: phase_ledger::Stage::Begin,
            },
        )?;
        let changed=tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='prepare' AND started=0",[lease.record_id()]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(Error::new(
                "reconciliation-required",
                "Native replacement preparation already attempted",
            ));
        }
        let changed=tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1 AND state='prepared' AND attempt=0",[&lease.effect]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        native_handoffs::touch(&tx, lease.record_id(), self)?;
        if !read.current() {
            return Err(preparation_refused("begin-current", Error::forbidden()));
        }
        drop(permit);
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            NativeReplacementPreparationAttempt {
                lease: lease.snapshot(),
            },
            read,
        ))
    }
    /// Historical definite refusal before any Core preparation can be called.
    /// An exact unstarted reservation is mandatory; a begun/unknown attempt is
    /// never reclassified as zero effect or lent to another worker.
    pub fn abort_native_replacement_preparation(
        &self,
        lease: &NativeReplacementPreparationLease,
        code: &str,
    ) -> Result<OperationRecord> {
        if !matches!(
            code,
            "unsupported-media-profile" | "invalid-request" | "forbidden" | "conflict"
        ) {
            return Err(invalid());
        }
        local_owner(
            self,
            &lease.selection.origin.owner,
            &lease.selection.origin.runtime,
        )?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::mutation_row_scopes(&rows::by_record(&c, lease.record_id())?)
            },
        )?;
        let mut c = self.connection_with_authority(&held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, lease.record_id())?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND n.plan_json=?5 AND n.selection_json=?6 AND s.phase='prepare' AND s.attempt_id=?7 AND s.effect_id=?8 AND s.started=0 AND e.attempt=0 AND e.state IN ('prepared','failed'))",params![lease.record_id(),lease.selection.origin.runtime,lease.reservation,lease.hash,lease.plan,lease.selected,lease.attempt,lease.effect],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::new(
                "reconciliation-required",
                "Core preparation was already begun or reservation changed",
            ));
        }
        if row.record.state == OperationState::Failed {
            return Ok(row.record);
        }
        tx.execute("UPDATE operation_effects SET state='failed' WHERE id=?1 AND state='prepared' AND attempt=0",[&lease.effect]).map_err(|_|Error::storage())?;
        if row.record.state == OperationState::Preparing && !row.record.cancel_requested {
            tx.execute("UPDATE operations SET problem_json=?2,lease_owner=NULL,lease_expires_at=NULL WHERE record_id=?1",params![lease.record_id(),serde_json::json!({"code":code,"source":"native-replacement-preparation"}).to_string()]).map_err(|_|Error::storage())?;
            rows::change(&tx, &row, OperationState::Failed, None, self)?;
        }
        native_handoffs::touch(&tx, lease.record_id(), self)?;
        let record = rows::by_record(&tx, lease.record_id())?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
    pub fn native_replacement_preparation_uncertain(
        &self,
        attempt: NativeReplacementPreparationAttempt,
    ) -> Result<()> {
        let lease = attempt.lease;
        let origin = &lease.selection.origin;
        local_owner(self, &origin.owner, &origin.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::mutation_row_scopes(&rows::by_record(&c, &origin.record)?)
            },
        )?;
        let mut c = self.connection_with_authority(&held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &origin.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.reservation_id=?2 AND n.plan_digest=?3 AND s.phase='prepare' AND s.attempt_id=?4 AND s.effect_id=?5 AND s.started=1)",params![origin.record,lease.reservation,lease.hash,lease.attempt,lease.effect],|r|r.get(0)).map_err(|_|Error::storage())?;
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
        tx.execute(
            "UPDATE operations SET state='unknown' WHERE record_id=?1",
            [&origin.record],
        )
        .map_err(|_| Error::storage())?;
        native_handoffs::touch(&tx, &origin.record, self)?;
        workers::invalidate(&tx, &origin.record)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Record an already captured owned Core ticket after cancellation/revocation.
    /// This cannot assert candidate media, advance roles or supply future IO.
    pub fn record_native_replacement_prepared(
        &self,
        receipt: &NativeReplacementPrepared,
    ) -> Result<OperationRecord> {
        local_owner(self, &receipt.owner, &receipt.runtime)?;
        let wait = crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        {
            let (held, mut c) = self.native_replacement_preparation_sql(
                &wait,
                false,
                "capture-read-authority",
                "capture-read-sql",
                |c| admission::row_scopes(&rows::by_record(c, &receipt.record)?),
            )?;
            if prepared_recording_state(&c, receipt)?.0.is_some() {
                // An identical receipt is an observation. Revalidate it inside
                // a read transaction without taking a SQLite writer reservation.
                let tx = c
                    .transaction_with_behavior(TransactionBehavior::Deferred)
                    .map_err(|_| Error::storage())?;
                let row = rows::by_record(&tx, &receipt.record)?;
                admission::require_row(self, &held, &row)?;
                let old = prepared_recording_state(&tx, receipt)?
                    .0
                    .ok_or_else(conflict)?;
                if old != receipt.observation {
                    return Err(conflict());
                }
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(row.record);
            }
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::storage())?;
            if let Some(result) = self.record_replacement_prepared_in(&tx, &held, receipt, true)? {
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(result);
            }
            // Late or uncertain first captures retain their existing Writer.
            // Drop this transaction and all readers before that admission.
        }
        let (held, mut c) = self.native_replacement_preparation_sql(
            &wait,
            true,
            "capture-write-authority",
            "capture-write-sql",
            |c| admission::mutation_row_scopes(&rows::by_record(c, &receipt.record)?),
        )?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let result = self
            .record_replacement_prepared_in(&tx, &held, receipt, false)?
            .ok_or_else(Error::forbidden)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }

    fn record_replacement_prepared_in(
        &self,
        tx: &Connection,
        held: &Arc<crate::AuthorityGuardSet>,
        receipt: &NativeReplacementPrepared,
        positive_only: bool,
    ) -> Result<Option<OperationRecord>> {
        let row = rows::by_record(tx, &receipt.record)?;
        admission::require_row(self, held, &row)?;
        let (old, effect_state, positive) = prepared_recording_state(tx, receipt)?;
        if let Some(old) = old {
            if old != receipt.observation {
                return Err(conflict());
            }
            return Ok(Some(row.record));
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let permit = if positive_only {
            if !positive
                || row.record.state != OperationState::Preparing
                || row.record.cancel_requested
                || row.tombstoned
                || effect_state != "submitting"
            {
                return Ok(None);
            }
            use crate::authority_writer_boundary::phase_ledger;
            Some(
                self.authority_writer_boundary
                    .native_resource_capture_ledger(
                        self,
                        tx,
                        held,
                        phase_ledger::Key {
                            record: receipt.record.clone(),
                            phase: "prepare",
                            attempt: receipt.attempt.clone(),
                            effect: receipt.effect.clone(),
                            expected: receipt.observation.clone(),
                            runtime: receipt.runtime.clone(),
                            reservation: receipt.reservation.clone(),
                            digest: receipt.hash.clone(),
                            stage: phase_ledger::Stage::Confirm,
                        },
                        None,
                    )?,
            )
        } else {
            if !self.authority_covers(held, &admission::mutation_row_scopes(&row)?) {
                return Err(Error::forbidden());
            }
            None
        };
        tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='prepare'",params![receipt.record,receipt.observation]).map_err(|_|Error::storage())?;
        let changed=tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&receipt.effect]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        let fact = serde_json::json!({"operation_id":row.record.operation_id,"native_replacement_prepared":true,"media_ready":false});
        let (hash, encoded) = digest(&fact)?;
        tx.execute("INSERT INTO operation_observations(id,operation_record_id,source,source_event_id,digest,actor_id,data_json,observed_at) VALUES(?1,?2,'native-replacement-prepare',?3,?4,?5,?6,?7)",params![format!("nro_{}",uuid::Uuid::new_v4()),receipt.record,receipt.attempt,hash,row.owner,encoded,crate::now()]).map_err(|_|Error::storage())?;
        native_handoffs::touch(tx, &receipt.record, self)?;
        drop(permit);
        Ok(Some(rows::by_record(tx, &receipt.record)?.record))
    }
}
fn prepared_recording_state(
    c: &Connection,
    receipt: &NativeReplacementPrepared,
) -> Result<(Option<String>, String, bool)> {
    let saved:Option<(Option<String>,String,bool)>=c.query_row("SELECT s.observation_json,e.state,COALESCE((n.expires_at>?7 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND NOT EXISTS(SELECT 1 FROM native_replacement_steps later WHERE later.operation_record_id=n.operation_record_id AND later.phase!='prepare')),0) FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.phase='prepare' AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1",params![receipt.record,receipt.runtime,receipt.reservation,receipt.hash,receipt.attempt,receipt.effect,crate::now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    saved.ok_or_else(Error::forbidden)
}
// The caller may inspect this before acquiring authority and again under its
// SQL transaction. A due result never authorizes DML without the exact Writer.
pub(super) fn recovery_disposition(
    c: &Connection,
    row: &rows::Saved,
    store: &Store,
) -> Result<Option<bool>> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(None);
    }
    let (runtime,expires,started):(String,i64,bool)=c.query_row("SELECT n.runtime_epoch,n.expires_at,EXISTS(SELECT 1 FROM native_replacement_steps s WHERE s.operation_record_id=n.operation_record_id AND s.started=1) FROM native_replacements n WHERE n.operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    let now = crate::now();
    Ok((expires <= now
        || row.record.expires_at.is_none_or(|t| t <= now)
        || (store.recover_foreign_runtime && runtime != store.runtime_epoch()))
    .then_some(started))
}
pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<bool> {
    if let Some(started) = recovery_disposition(c, row, store)? {
        c.execute("UPDATE operation_effects SET state=?2 WHERE operation_record_id=?1 AND state IN ('prepared','submitting')",params![row.record.record_id,if started{"unknown"}else{"cancelled"}]).map_err(|_|Error::storage())?;
        c.execute(
            "UPDATE operations SET state=?2 WHERE record_id=?1",
            params![
                row.record.record_id,
                if started { "unknown" } else { "failed" }
            ],
        )
        .map_err(|_| Error::storage())?;
        native_handoffs::touch(c, &row.record.record_id, store)?;
        workers::invalidate(c, &row.record.record_id)?;
        return Ok(true);
    }
    Ok(false)
}

impl Store {
    /// Definitive refusal before any native reservation or attempt exists.
    pub fn fail_native_replacement_preflight(
        &self,
        credential: &str,
        mut command: TransitionCommand,
        code: &str,
    ) -> Result<bool> {
        command.normalize()?;
        if !matches!(command.intent, TransitionIntent::Replace { .. })
            || !matches!(
                code,
                "unsupported-media-profile" | "invalid-request" | "forbidden" | "conflict"
            )
        {
            return Err(invalid());
        }
        self.require_runtime_owner()?;
        if let Some(settled) = self.fail_candidate_before_preparation(credential, &command)? {
            return Ok(settled);
        }
        let held = self.transition_operation_mutation_authority(
            credential,
            &command.operation_id,
            &["connection.replace"],
            None,
        )?;
        let mut c = self.connection_with_authority(&held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        actor.require("connection.replace")?;
        let row = rows::find(
            &tx,
            &authority::tenant(&tx, &actor)?,
            &actor.identity_id,
            &command.operation_id,
        )?
        .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &held, &row)?;
        if rows::request(&row)?
            != command.with_expiry(row.record.expires_at.ok_or_else(Error::storage)?)
        {
            return Err(conflict());
        }
        if row.native_replacement
            || row.record.state != OperationState::Accepted
            || row.worker.is_some()
            || !row.record.effects.is_empty()
        {
            return Ok(false);
        }
        tx.execute(
            "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
            params![
                row.record.record_id,
                serde_json::json!({"code":code,"source":"native-replacement"}).to_string()
            ],
        )
        .map_err(|_| Error::storage())?;
        rows::change(&tx, &row, OperationState::Failed, None, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
    /// Trusted capture after the owned uncommitted Core ticket was cancelled.
    /// No allocation/start/commit may have begun. This is local preparation
    /// cleanup, never a claim that a possibly dialed callback was undone.
    pub fn record_native_replacement_preparation_stopped(
        &self,
        receipt: &NativeReplacementPrepared,
    ) -> Result<OperationRecord> {
        local_owner(self, &receipt.owner, &receipt.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::mutation_row_scopes(&rows::by_record(&c, &receipt.record)?)
            },
        )?;
        let mut c = self.connection_with_authority(&held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &receipt.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.observation_json=?7 AND s.started=1) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase!='prepare' AND started=1)",params![receipt.record,receipt.runtime,receipt.reservation,receipt.hash,receipt.attempt,receipt.effect,receipt.observation],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement may have progressed beyond local preparation",
            ));
        }
        let cleanup = format!("nrc_{}", receipt.attempt);
        let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE id=?1 AND operation_record_id=?2 AND compensates_effect_id=?3 AND state='confirmed')",params![cleanup,receipt.record,receipt.effect],|r|r.get(0)).map_err(|_|Error::storage())?;
        if already {
            return Ok(row.record);
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id) VALUES(?1,?2,'native-replacement-prepare',?1,'none','confirmed',1,1,?3)",params![cleanup,receipt.record,receipt.effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operations SET state=?2,problem_json=?3 WHERE record_id=?1",params![receipt.record,if row.record.cancel_requested{"cancelled"}else{"failed"},if row.record.cancel_requested{None}else{Some(serde_json::json!({"code":"preparation-stopped","source":"native-replacement"}).to_string())}]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &receipt.record, self)?;
        workers::invalidate(&tx, &receipt.record)?;
        let result = rows::by_record(&tx, &receipt.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
