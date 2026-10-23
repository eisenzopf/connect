//! Trusted installed receive custody, immutable recipients and one physical attempt.
use super::*;
use crate::{
    AuthorityScope, NativeDtmfRecipient, NativeMediaOrigin, SipNativeSignalRead, SipSignalOrigin,
    SipSignalRead, TelephoneSignalDirection::FromTelephone,
};
use connect_protocol::{
    native_dtmf::SubmissionDisposition as Disposition, received_dtmf::SourceObservation,
};
use std::{collections::BTreeSet, sync::Weak};
struct Source {
    record: String,
    operation: String,
    effect: String,
    runtime: String,
    expires: i64,
    phone: SipSignalOrigin,
    observation: SourceObservation,
    snapshot: serde_json::Value,
    owner: Weak<crate::InstallationOwnership>,
}
pub struct ReceivedDtmfBatch {
    source: Arc<Source>,
    targets: Vec<ReceivedDtmfTarget>,
}
#[derive(Clone)]
pub struct ReceivedDtmfTarget {
    source: Arc<Source>,
    origin: Arc<NativeMediaOrigin>,
    registration: String,
    snapshot: serde_json::Value,
}
pub struct ReceivedDtmfRead {
    pair: SipNativeSignalRead,
    target: ReceivedDtmfTarget,
}
pub enum ReceivedDtmfPreparation {
    New(ReceivedDtmfBatch),
    Replay(OperationRecord),
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
redacted!(
    ReceivedDtmfBatch,
    ReceivedDtmfTarget,
    ReceivedDtmfRead,
    ReceivedDtmfPreparation
);
impl ReceivedDtmfBatch {
    pub fn targets(&self) -> &[ReceivedDtmfTarget] {
        &self.targets
    }
    pub fn record_id(&self) -> &str {
        &self.source.record
    }
    pub fn operation_id(&self) -> &str {
        &self.source.operation
    }
    pub fn expires_at(&self) -> i64 {
        self.source.expires
    }
    pub fn observation(&self) -> &SourceObservation {
        &self.source.observation
    }
}
impl ReceivedDtmfTarget {
    pub fn origin(&self) -> &Arc<NativeMediaOrigin> {
        &self.origin
    }
    pub fn source(&self) -> &SipSignalOrigin {
        &self.source.phone
    }
    pub fn registration_id(&self) -> &str {
        &self.registration
    }
    pub fn operation_id(&self) -> &str {
        &self.source.operation
    }
    pub fn observation(&self) -> &SourceObservation {
        &self.source.observation
    }
    pub fn expires_at(&self) -> i64 {
        self.source.expires
    }
}
impl ReceivedDtmfRead {
    pub fn pair(&self) -> &SipNativeSignalRead {
        &self.pair
    }
    pub fn current(&self) -> bool {
        self.pair.current() && crate::now() < self.target.source.expires
    }
    pub fn expires_at(&self) -> i64 {
        self.pair.expires_at().min(self.target.source.expires)
    }
}
fn logical_id(phone: &SipSignalOrigin, observation: &SourceObservation) -> String {
    let mut h = Sha256::new();
    h.update(phone.record_id());
    h.update([0]);
    h.update(observation.kind.as_str());
    h.update([0]);
    h.update(&observation.observation_id);
    format!("dtmfr_{:x}", h.finalize())
}
fn historical(c: &Connection, store: &Store, source: &Source) -> Result<rows::Saved> {
    let owner = store.require_runtime_owner()?;
    if source.runtime != store.runtime_epoch()
        || !owner.current()
        || !source
            .owner
            .upgrade()
            .is_some_and(|o| Arc::ptr_eq(&o, &owner))
    {
        return Err(Error::forbidden());
    }
    let row = rows::by_record(c, &source.record)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM received_dtmf_sources s JOIN operation_effects e ON e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.parent_record_id=?2 AND s.runtime_epoch=?3 AND s.observation_json=?4 AND s.source_json=?5 AND s.sealed=1 AND e.id=?6 AND e.provider='received-dtmf' AND s.parent_kind=?7)",params![source.record,source.phone.record_id(),source.runtime,serde_json::to_string(&source.observation).map_err(|_|Error::storage())?,source.snapshot.to_string(),source.effect,source.phone.kind()],|r|r.get(0)).map_err(|_|Error::storage())?;
    let (digest, _) = super::digest(
        &serde_json::json!({"observation":source.observation,"source":source.snapshot}),
    )?;
    if !exact
        || row.record.command_type != "dtmf.send"
        || row.digest != digest
        || row.owner != source.phone.identity_id()
        || row.record.operation_id != source.operation
        || row.record.expires_at != Some(source.expires)
    {
        return Err(Error::forbidden());
    }
    Ok(row)
}
fn current(
    c: &Connection,
    store: &Store,
    target: &ReceivedDtmfTarget,
    read: &SipNativeSignalRead,
) -> Result<()> {
    let row = historical(c, store, &target.source)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM received_dtmf_sources s JOIN received_dtmf_recipients r ON r.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.closed=0 AND r.target_record_id=?2 AND r.registration_id=?3 AND r.target_json=?4 AND r.disposition IS NULL)",params![target.source.record,target.origin.offer_record_id(),target.registration,target.snapshot.to_string()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact
        || row.record.state != OperationState::Preparing
        || row.record.cancel_requested
        || crate::now() >= target.source.expires
        || !read.current()
        || !read.matches(&target.source.phone, FromTelephone)
        || read.snapshot() != target.source.snapshot
        || store.native_dtmf_origin_snapshot(c, &target.origin)? != target.snapshot
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn projection_scopes(c: &Connection, row: &rows::Saved) -> Result<Vec<AuthorityScope>> {
    let mut scopes = Vec::new();
    let mut q=c.prepare("SELECT json_extract(target_json,'$.identity_id') FROM received_dtmf_recipients WHERE operation_record_id=?1 UNION SELECT json_extract(source_json,'$.route_owner_id') FROM received_dtmf_sources WHERE operation_record_id=?1").map_err(|_|Error::storage())?;
    for id in q
        .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
    {
        scopes.push(AuthorityScope::identity(
            &id.map_err(|_| Error::storage())?,
            crate::AuthorityMode::Read,
        )?);
    }
    AuthorityScope::merged(&scopes)
}
// Installed receive replay uses the call's current signal credential. Its
// logical operation owner is the actual participant, not an independent route
// account. Original recipient ceilings are rechecked without rescheduling IO.
fn require_source_replay(c: &Connection, read: &SipSignalRead, row: &rows::Saved) -> Result<()> {
    let source: String = c
        .query_row(
            "SELECT source_json FROM received_dtmf_sources WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let source: serde_json::Value = serde_json::from_str(&source).map_err(|_| Error::storage())?;
    if !read.current() || row.owner != read.member().identity_id || source != read.snapshot() {
        return Err(Error::forbidden());
    }
    let mut q=c.prepare("SELECT DISTINCT json_extract(target_json,'$.identity_id') FROM received_dtmf_recipients WHERE operation_record_id=?1").map_err(|_|Error::storage())?;
    for id in q
        .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
    {
        let id = id.map_err(|_| Error::storage())?;
        if !read.permission_actor().permits_recipient(&id)
            || !read.route_actor().permits_recipient(&id)
        {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
pub(super) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    actor.require("dtmf.send")?;
    let source: String = c
        .query_row(
            "SELECT source_json FROM received_dtmf_sources WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let source: serde_json::Value = serde_json::from_str(&source).map_err(|_| Error::storage())?;
    let cid = row
        .record
        .conversation_id
        .as_deref()
        .ok_or_else(Error::storage)?;
    let member = crate::audience::member(c, &actor.identity_id, cid, true)?;
    if source["identity_id"] != actor.identity_id
        || source["participant_id"] != member.participant_id
        || source["membership_epoch"] != member.membership_epoch
    {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        actor,
        cid,
        connect_protocol::membership::ContentKind::Session,
        row.record
            .session_id
            .as_deref()
            .ok_or_else(Error::storage)?,
    )?;
    let mut q=c.prepare("SELECT DISTINCT json_extract(target_json,'$.identity_id') FROM received_dtmf_recipients WHERE operation_record_id=?1").map_err(|_|Error::storage())?;
    for id in q
        .query_map([&row.record.record_id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
    {
        if !actor.permits_recipient(&id.map_err(|_| Error::storage())?) {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
fn aggregate(c: &Connection, record: &str) -> Result<serde_json::Value> {
    let (source,observation):(String,String)=c.query_row("SELECT source_json,observation_json FROM received_dtmf_sources WHERE operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let source: serde_json::Value = serde_json::from_str(&source).map_err(|_| Error::storage())?;
    let observation: SourceObservation =
        serde_json::from_str(&observation).map_err(|_| Error::storage())?;
    let mut q=c.prepare("SELECT target_json,disposition,frame_id FROM received_dtmf_recipients WHERE operation_record_id=?1 ORDER BY target_record_id").map_err(|_|Error::storage())?;
    let mut recipients = Vec::new();
    for r in q
        .query_map([record], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
    {
        let (target, disposition, frame) = r.map_err(|_| Error::storage())?;
        let target: serde_json::Value =
            serde_json::from_str(&target).map_err(|_| Error::storage())?;
        recipients.push(serde_json::json!({"connection_id":target["connection_id"],"participant_id":target["participant_id"],"membership_epoch":target["membership_epoch"],"generation":target["generation"],"disposition":disposition,"frame_id":frame}));
    }
    Ok(
        serde_json::json!({"connection_id":source["connection_id"],"resources_changed":false,"signal_source":observation.kind.as_str(),"source_observation_id":observation.observation_id,"received_at":observation.received_at,"source_observation":observation,"local_submission_only":true,"local_work_complete":true,"recipients":recipients}),
    )
}
fn outcome(
    c: &Connection,
    store: &Store,
    record: &str,
    target: &str,
    disposition: Disposition,
    recovery: bool,
) -> Result<()> {
    let (attempted,old):(bool,Option<String>)=c.query_row("SELECT attempted,disposition FROM received_dtmf_recipients WHERE operation_record_id=?1 AND target_record_id=?2",params![record,target],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    if let Some(old) = old {
        return if old == disposition.as_str() {
            Ok(())
        } else {
            Err(conflict())
        };
    }
    if !attempted && disposition != Disposition::NotStarted {
        return Err(Error::forbidden());
    }
    let row = rows::by_record(c, record)?;
    if row.record.revision >= 9_007_199_254_740_990 {
        return Err(conflict());
    }
    if !matches!(
        row.record.state,
        OperationState::Preparing | OperationState::Unknown
    ) {
        return Err(conflict());
    }
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Signal outcome reservation unavailable",
        ));
    }
    c.execute("UPDATE received_dtmf_recipients SET disposition=?3,provenance=?4,observed_at=?5,journal_id=?6 WHERE operation_record_id=?1 AND target_record_id=?2 AND disposition IS NULL",params![record,target,disposition.as_str(),if recovery{"runtime_recovery"}else{"local_writer"},crate::now(),format!("evt_{}",uuid::Uuid::new_v4().simple())]).map_err(|_|Error::storage())?;
    c.execute("UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1 AND revision<9007199254740990",params![record,crate::now()]).map_err(|_|Error::storage())?;
    rows::event(c, record, "operation_updated", store)
}
fn finish(c: &Connection, store: &Store, record: &str) -> Result<OperationRecord> {
    let row = rows::by_record(c, record)?;
    let (pending,unknown,submitted,total):(u64,u64,u64,u64)=c.query_row("SELECT COALESCE(sum(disposition IS NULL),0),COALESCE(sum(disposition='outcome_unknown'),0),COALESCE(sum(disposition='local_submitted'),0),count(*) FROM received_dtmf_recipients WHERE operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::storage())?;
    if pending != 0 {
        return Err(conflict());
    }
    let summary = aggregate(c, record)?;
    let desired = if unknown > 0 {
        OperationState::Unknown
    } else if total > 0 && submitted == total {
        OperationState::Completed
    } else if row.record.cancel_requested && submitted == 0 {
        OperationState::Cancelled
    } else {
        OperationState::Failed
    };
    let closed: bool = c
        .query_row(
            "SELECT closed FROM received_dtmf_sources WHERE operation_record_id=?1",
            [record],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if closed {
        return Ok(row.record);
    }
    c.execute("UPDATE operation_effects SET state=?2 WHERE operation_record_id=?1 AND provider='received-dtmf'",params![record,if unknown>0{"unknown"}else if desired==OperationState::Completed{"confirmed"}else{"failed"}]).map_err(|_|Error::storage())?;
    let problem = if desired == OperationState::Completed {
        None
    } else {
        Some(
            serde_json::json!({"code":if unknown>0{"signal-outcome-unknown"}else if total==0{"signal-no-recipients"}else if submitted>0{"partial-submission"}else{"signal-not-submitted"},"message":"Received DTMF was not submitted to every intended recipient","signal":summary}),
        )
    };
    c.execute(
        "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
        params![record, problem.as_ref().map(serde_json::Value::to_string)],
    )
    .map_err(|_| Error::storage())?;
    let mut row = row;
    if desired == OperationState::Completed {
        rows::change(c, &row, OperationState::Committing, None, store)?;
        row = rows::by_record(c, record)?;
    }
    if desired != row.record.state {
        rows::change(
            c,
            &row,
            desired,
            if desired == OperationState::Completed {
                Some(summary)
            } else {
                None
            },
            store,
        )?;
    } else {
        if row.record.revision >= 9_007_199_254_740_990 {
            return Err(conflict());
        }
        c.execute(
            "UPDATE operations SET revision=revision+1 WHERE record_id=?1",
            [record],
        )
        .map_err(|_| Error::storage())?;
        rows::event(c, record, "operation_updated", store)?;
    }
    c.execute(
        "UPDATE received_dtmf_sources SET closed=1 WHERE operation_record_id=?1 AND closed=0",
        [record],
    )
    .map_err(|_| Error::storage())?;
    workers::invalidate(c, record)?;
    Ok(rows::by_record(c, record)?.record)
}
pub(super) fn cancel(c: &Connection, row: &rows::Saved, store: &Store) -> Result<&'static str> {
    if !row.record.cancel_requested {
        if row.record.revision >= 9_007_199_254_740_990 {
            return Err(conflict());
        }
        c.execute("UPDATE operations SET cancel_requested=1,revision=revision+1,updated_at=?2 WHERE record_id=?1 AND cancel_requested=0",params![row.record.record_id,crate::now()]).map_err(|_|Error::storage())?;
        rows::event(c, &row.record.record_id, "operation_updated", store)?;
    }
    Ok("cancel_requested")
}
pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<bool> {
    let (runtime, closed): (String, bool) = c
        .query_row(
            "SELECT runtime_epoch,closed FROM received_dtmf_sources WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    if closed
        || (runtime == store.runtime_epoch()
            && row.record.expires_at.is_some_and(|at| at > crate::now()))
    {
        return Ok(false);
    }
    let targets = {
        let mut q=c.prepare("SELECT target_record_id,attempted FROM received_dtmf_recipients WHERE operation_record_id=?1 AND disposition IS NULL ORDER BY target_record_id").map_err(|_|Error::storage())?;
        let out = q
            .query_map([&row.record.record_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        out
    };
    for (target, attempted) in targets {
        outcome(
            c,
            store,
            &row.record.record_id,
            &target,
            if attempted {
                Disposition::OutcomeUnknown
            } else {
                Disposition::NotStarted
            },
            true,
        )?;
    }
    finish(c, store, &row.record.record_id)?;
    Ok(true)
}
impl Store {
    /// Trusted actual source composition only. Success freezes immutable input
    /// and current exact recipients; no reader or IO grant is queued.
    pub fn prepare_received_dtmf(
        &self,
        phone: SipSignalOrigin,
        observation: SourceObservation,
        candidates: Vec<NativeDtmfRecipient>,
    ) -> Result<ReceivedDtmfPreparation> {
        observation.validate()?;
        if candidates.len() > self.limits().connections.min(512) {
            return Err(invalid());
        }
        let source_read = self.try_sip_signal_read(&phone, FromTelephone)?;
        let operation = logical_id(&phone, &observation);
        let required = {
            let c = self.try_connection()?;
            let mut required = phone.scopes(self, &c)?;
            for target in &candidates {
                match phone.pair_scopes(self, &c, &target.origin) {
                    Ok(scopes) => required.extend(scopes),
                    Err(e) if matches!(e.code.as_str(), "forbidden" | "not-recipient") => (),
                    Err(e) => return Err(e),
                }
            }
            AuthorityScope::merged(&required)?
        };
        drop(source_read);
        let interval = self.try_acquire_authority(&required)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let until = observation
            .received_at
            .timestamp()
            .checked_add(40)
            .ok_or_else(invalid)?;
        if observation.received_at.timestamp() > crate::now() || until <= crate::now() {
            return Err(Error::forbidden());
        }
        let read = phone.read_in(self, &tx, FromTelephone, interval.clone(), until)?;
        let actor = read.permission_actor();
        let tenant = authority::tenant(&tx, actor)?;
        if let Some(row) = rows::find(&tx, &tenant, &read.member().identity_id, &operation)? {
            let (old,parent,kind):(String,String,String)=tx.query_row("SELECT observation_json,parent_record_id,parent_kind FROM received_dtmf_sources WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|conflict())?;
            let old: SourceObservation =
                serde_json::from_str(&old).map_err(|_| Error::storage())?;
            if parent != phone.record_id()
                || kind != phone.kind()
                || !old.same_physical_input(&observation)
            {
                return Err(conflict());
            }
            require_source_replay(&tx, &read, &row)?;
            return Ok(ReceivedDtmfPreparation::Replay(row.record));
        }
        let source_snapshot = read.snapshot();
        let expires = read.expires_at();
        let mut frozen = Vec::new();
        let mut seen = BTreeSet::new();
        let mut registrations = BTreeSet::new();
        for target in candidates {
            id(&target.registration_id)?;
            if !seen.insert(target.origin.offer_record_id().to_owned())
                || !registrations.insert(target.registration_id.clone())
            {
                return Err(conflict());
            }
            let pair = match phone.pair_in(
                self,
                &tx,
                target.origin.clone(),
                FromTelephone,
                interval.clone(),
                expires,
            ) {
                Ok(pair) => pair,
                Err(e) if matches!(e.code.as_str(), "forbidden" | "not-recipient") => continue,
                Err(e) => return Err(e),
            };
            if !pair.current() {
                continue;
            }
            let snapshot = self.native_dtmf_origin_snapshot(&tx, &target.origin)?;
            frozen.push((target, snapshot));
        }
        if !crate::conversations::capacity(&tx, frozen.len() as u64 + 3, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Received signal outcome capacity exhausted",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        let effect = format!("rde_{}", uuid::Uuid::new_v4().simple());
        let intent = serde_json::json!({"observation":observation,"source":source_snapshot});
        let (hash, json) = super::digest(&intent)?;
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,intent_json,state,created_at,updated_at,expires_at,ledger_version,worker_runtime) VALUES(?1,?2,?3,?4,?5,'dtmf.send',?6,?7,?8,?9,'accepted',?10,?10,?11,0,?12)",params![record,tenant,read.member().identity_id,operation,hash,phone.conversation_id(),phone.session_id(),phone.connection_id(),json,crate::now(),expires,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO received_dtmf_sources(operation_record_id,parent_record_id,runtime_epoch,observation_id,source_kind,observation_json,source_json,parent_kind) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![record,phone.record_id(),self.runtime_epoch(),observation.observation_id,observation.kind.as_str(),serde_json::to_string(&observation).map_err(|_|Error::storage())?,source_snapshot.to_string(),phone.kind()]).map_err(|_|Error::storage())?;
        tx.execute(
            "INSERT INTO operation_members VALUES(?1,?2,?3,?4)",
            params![
                record,
                read.member().participant_id,
                read.member().identity_id,
                read.member().membership_epoch
            ],
        )
        .map_err(|_| Error::storage())?;
        for (target, snapshot) in &frozen {
            tx.execute("INSERT INTO received_dtmf_recipients(operation_record_id,target_record_id,registration_id,target_json) VALUES(?1,?2,?3,?4)",params![record,target.origin.offer_record_id(),target.registration_id,snapshot.to_string()]).map_err(|_|Error::storage())?;
        }
        tx.execute(
            "UPDATE received_dtmf_sources SET sealed=1 WHERE operation_record_id=?1",
            [&record],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES(?1,?2,'received-dtmf',?3,'none','submitting',1,1)",params![effect,record,observation.observation_id]).map_err(|_|Error::storage())?;
        let row = rows::by_record(&tx, &record)?;
        rows::change(&tx, &row, OperationState::Preparing, None, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        let source = Arc::new(Source {
            record,
            operation,
            effect,
            runtime: self.runtime_epoch().into(),
            expires,
            phone,
            observation,
            snapshot: source_snapshot,
            owner: Arc::downgrade(&self.require_runtime_owner()?),
        });
        let targets = frozen
            .into_iter()
            .map(|(t, snapshot)| ReceivedDtmfTarget {
                source: source.clone(),
                origin: t.origin,
                registration: t.registration_id,
                snapshot,
            })
            .collect();
        Ok(ReceivedDtmfPreparation::New(ReceivedDtmfBatch {
            source,
            targets,
        }))
    }
    pub fn try_received_dtmf_read(&self, target: &ReceivedDtmfTarget) -> Result<ReceivedDtmfRead> {
        let pair = self.try_sip_native_signal_read(
            &target.source.phone,
            target.origin.clone(),
            FromTelephone,
        )?;
        let c = self.try_connection()?;
        current(&c, self, target, &pair)?;
        Ok(ReceivedDtmfRead {
            pair,
            target: target.clone(),
        })
    }
    pub fn try_start_received_dtmf_write(
        &self,
        read: &ReceivedDtmfRead,
        frame: &str,
    ) -> Result<()> {
        id(frame)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        current(&tx, self, &read.target, &read.pair)?;
        let target = &read.target;
        let old:Option<String>=tx.query_row("SELECT frame_id FROM received_dtmf_recipients WHERE operation_record_id=?1 AND target_record_id=?2",params![target.source.record,target.origin.offer_record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if let Some(old) = old {
            if old != frame {
                return Err(conflict());
            }
        } else {
            tx.execute("UPDATE received_dtmf_recipients SET attempted=1,frame_id=?3 WHERE operation_record_id=?1 AND target_record_id=?2 AND disposition IS NULL AND attempted=0",params![target.source.record,target.origin.offer_record_id(),frame]).map_err(|_|Error::storage())?;
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn try_record_received_dtmf_outcome(
        &self,
        target: &ReceivedDtmfTarget,
        disposition: Disposition,
    ) -> Result<()> {
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        historical(&tx, self, &target.source)?;
        outcome(
            &tx,
            self,
            &target.source.record,
            target.origin.offer_record_id(),
            disposition,
            false,
        )?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn finish_received_dtmf(&self, batch: &ReceivedDtmfBatch) -> Result<OperationRecord> {
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        historical(&tx, self, &batch.source)?;
        let result = finish(&tx, self, &batch.source.record)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    pub fn recover_received_dtmf(&self, limit: u32) -> Result<u32> {
        if !(1..=256).contains(&limit) {
            return Err(invalid());
        }
        let owner = self.require_runtime_owner()?;
        let records = {
            let c = self.connection()?;
            let mut q=c.prepare("SELECT operation_record_id FROM received_dtmf_sources s JOIN operations o ON o.record_id=s.operation_record_id WHERE s.closed=0 AND (s.runtime_epoch!=?1 OR o.expires_at<=?2) ORDER BY operation_record_id LIMIT ?3").map_err(|_|Error::storage())?;
            let out = q
                .query_map(params![self.runtime_epoch(), crate::now(), limit], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            out
        };
        let mut changed = 0;
        for record in records {
            // Do not settle an expired attempt while a physical writer still
            // owns its pair read. Wait without holding the SQL connection.
            let authority = self.stable_authority(
                &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
                || {
                    let c = self.connection()?;
                    admission::row_scopes(&rows::by_record(&c, &record)?)
                },
            )?;
            let mut c = self.connection()?;
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| Error::storage())?;
            let row = rows::by_record(&tx, &record)?;
            admission::require_row(self, &authority, &row)?;
            if recover_one(&tx, &row, self)? {
                changed += 1;
            }
            if !owner.current() {
                return Err(Error::forbidden());
            }
            tx.commit().map_err(|_| Error::storage())?;
        }
        Ok(changed)
    }
}
