//! One original signal, frozen recipients and append-only local writer facts.
use super::*;
use crate::{NativeMediaOrigin, NativeSignalRead};
use connect_protocol::native_dtmf::{SourceObservation, SubmissionDisposition as Disposition};
use std::{
    collections::BTreeSet,
    sync::{Arc, Weak},
};

/// Actual host registration metadata. It contains no signal permission.
pub struct NativeDtmfRecipient {
    pub origin: Arc<NativeMediaOrigin>,
    pub registration_id: String,
}
struct Source {
    record: String,
    effect: String,
    runtime: String,
    origin: Arc<NativeMediaOrigin>,
    wire: Intent,
    observation: SourceObservation,
    owner: Weak<crate::InstallationOwnership>,
    expires: i64,
}
pub struct NativeDtmfTarget {
    source: Arc<Source>,
    origin: Arc<NativeMediaOrigin>,
    registration: String,
    snapshot: serde_json::Value,
}
pub struct NativeDtmfBatch {
    source: Arc<Source>,
    targets: Vec<NativeDtmfTarget>,
}
impl NativeDtmfBatch {
    pub fn targets(&self) -> &[NativeDtmfTarget] {
        &self.targets
    }
    pub fn observation(&self) -> &SourceObservation {
        &self.source.observation
    }
    pub fn intent(&self) -> &Intent {
        &self.source.wire
    }
}
impl NativeDtmfTarget {
    pub fn origin(&self) -> &Arc<NativeMediaOrigin> {
        &self.origin
    }
    pub fn registration_id(&self) -> &str {
        &self.registration
    }
    pub fn observation(&self) -> &SourceObservation {
        &self.source.observation
    }
    pub fn intent(&self) -> &Intent {
        &self.source.wire
    }
}
pub struct NativeDtmfRead {
    media: NativeSignalRead,
    source: Arc<Source>,
    target: Arc<NativeMediaOrigin>,
    registration: String,
    snapshot: serde_json::Value,
    expires: i64,
}
impl NativeDtmfRead {
    pub fn media(&self) -> &NativeSignalRead {
        &self.media
    }
    pub fn current(&self) -> bool {
        self.media.current() && crate::now() < self.expires
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
}
macro_rules! redacted {
    ($($t:ty),*) => { $(impl std::fmt::Debug for $t {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.write_str(concat!(stringify!($t),"(redacted)")) }
    })* }
}
redacted!(
    NativeDtmfRecipient,
    NativeDtmfBatch,
    NativeDtmfTarget,
    NativeDtmfRead
);

pub(super) fn recipient_scopes(
    c: &Connection,
    actor: &Actor,
    record: &str,
) -> Result<Vec<AuthorityScope>> {
    let mut q=c.prepare("SELECT DISTINCT json_extract(target_json,'$.identity_id') FROM native_dtmf_recipients WHERE operation_record_id=?1 ORDER BY 1").map_err(|_|Error::storage())?;
    let ids = q
        .query_map([record], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    ids.iter()
        .map(|id| {
            if !actor.permits_recipient(id) {
                return Err(Error::forbidden());
            }
            AuthorityScope::identity(id, crate::AuthorityMode::Read)
        })
        .collect()
}

fn historical(c: &Connection, store: &Store, source: &Source) -> Result<rows::Saved> {
    let owner = store.require_runtime_owner()?;
    if source.runtime != store.runtime_epoch()
        || !owner.current()
        || !source
            .owner
            .upgrade()
            .is_some_and(|old| Arc::ptr_eq(&old, &owner))
    {
        return Err(Error::forbidden());
    }
    let row = rows::by_record(c, &source.record)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN native_dtmf_sources d ON d.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.parent_record_id=?2 AND n.effect_id=?3 AND n.attempt_runtime=?4 AND n.generation=?5 AND d.runtime_epoch=n.attempt_runtime AND d.sealed=1 AND d.observation_json=?6 AND e.provider='native-stream')",params![source.record,source.origin.offer_record_id(),source.effect,source.runtime,source.origin.generation(),serde_json::to_string(&source.observation).map_err(|_|Error::storage())?],|r|r.get(0)).map_err(|_|Error::storage())?;
    let (digest, _) =
        super::digest(&serde_json::to_value(&source.wire).map_err(|_| Error::storage())?)?;
    if !exact
        || row.record.command_type != "dtmf.send"
        || row.digest != digest
        || row.record.expires_at != Some(source.expires)
    {
        return Err(Error::forbidden());
    }
    Ok(row)
}
fn current(
    c: &Connection,
    store: &Store,
    source: &Source,
    target: &NativeMediaOrigin,
    registration: &str,
    snapshot: &serde_json::Value,
) -> Result<()> {
    let row = historical(c, store, source)?;
    if row.record.state != OperationState::Preparing
        || row.record.cancel_requested
        || crate::now() >= source.expires
    {
        return Err(Error::forbidden());
    }
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_dtmf_recipients r JOIN native_stream_operations n ON n.operation_record_id=r.operation_record_id JOIN operation_effects e ON e.id=n.effect_id WHERE r.operation_record_id=?1 AND r.target_record_id=?2 AND r.registration_id=?3 AND r.target_json=?4 AND r.disposition IS NULL AND n.closed=0 AND e.state='submitting')",params![source.record,target.offer_record_id(),registration,serde_json::to_string(snapshot).map_err(|_|Error::storage())?],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact || store.native_dtmf_origin_snapshot(c, target)? != *snapshot {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn aggregate(c: &Connection, record: &str, complete: bool) -> Result<serde_json::Value> {
    let (connection,observation):(String,String)=c.query_row("SELECT o.connection_id,d.observation_json FROM operations o JOIN native_dtmf_sources d ON d.operation_record_id=o.record_id WHERE o.record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let observation: SourceObservation =
        serde_json::from_str(&observation).map_err(|_| Error::storage())?;
    let mut q=c.prepare("SELECT target_json,disposition,frame_id FROM native_dtmf_recipients WHERE operation_record_id=?1 ORDER BY target_record_id").map_err(|_|Error::storage())?;
    let rows = q
        .query_map([record], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut recipients = Vec::with_capacity(rows.len());
    for (snapshot, disposition, frame) in rows {
        let target: serde_json::Value =
            serde_json::from_str(&snapshot).map_err(|_| Error::storage())?;
        recipients.push(serde_json::json!({"connection_id":target["connection_id"],"participant_id":target["participant_id"],"membership_epoch":target["membership_epoch"],"generation":target["generation"],"disposition":disposition,"frame_id":frame}));
    }
    Ok(
        serde_json::json!({"connection_id":connection,"stream_revision":observation.stream_revision,"resources_changed":false,"signal_source":"uctp-native","source_observation_id":observation.envelope_id,"received_at":observation.received_at,"local_submission_only":true,"local_work_complete":complete,"recipients":recipients}),
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
    let (attempted,saved):(bool,Option<String>)=c.query_row("SELECT attempted,disposition FROM native_dtmf_recipients WHERE operation_record_id=?1 AND target_record_id=?2",params![record,target],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    if let Some(saved) = saved {
        return if saved == disposition.as_str() {
            Ok(())
        } else {
            Err(conflict())
        };
    }
    if !attempted && disposition != Disposition::NotStarted {
        return Err(Error::forbidden());
    }
    let row = rows::by_record(c, record)?;
    if !matches!(
        row.record.state,
        OperationState::Preparing | OperationState::Unknown
    ) {
        return Err(conflict());
    }
    let journal = format!("evt_{}", uuid::Uuid::new_v4().simple());
    c.execute("UPDATE native_dtmf_recipients SET disposition=?3,provenance=?4,observed_at=?5,journal_id=?6 WHERE operation_record_id=?1 AND target_record_id=?2 AND disposition IS NULL",params![record,target,disposition.as_str(),if recovery {"runtime_recovery"} else {"local_writer"},crate::now(),journal]).map_err(|_|Error::storage())?;
    if row.record.revision >= 9007199254740990 {
        return Err(conflict());
    }
    c.execute(
        "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
        params![record, crate::now()],
    )
    .map_err(|_| Error::storage())?;
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "DTMF outcome reservation unavailable",
        ));
    }
    let data = serde_json::json!({"source_kind":"session","source_id":row.record.session_id,"operation_id":row.record.operation_id,"state":row.record.state});
    c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json) VALUES(?1,?2,?3,'operation_updated',?4,?5,?6)",params![journal,row.owner,row.record.conversation_id,record,row.record.revision+1,data.to_string()]).map_err(|_|Error::storage())?;
    Ok(())
}
fn finish(c: &Connection, store: &Store, record: &str) -> Result<OperationRecord> {
    let row = rows::by_record(c, record)?;
    let (pending,unknown,submitted,total):(u64,u64,u64,u64)=c.query_row("SELECT COALESCE(SUM(disposition IS NULL),0),COALESCE(SUM(disposition='outcome_unknown'),0),COALESCE(SUM(disposition='local_submitted'),0),COUNT(*) FROM native_dtmf_recipients WHERE operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::storage())?;
    if pending != 0 || total == 0 {
        return Err(conflict());
    }
    let result = aggregate(c, record, true)?;
    let desired = if unknown > 0 {
        OperationState::Unknown
    } else if submitted == total {
        OperationState::Completed
    } else if submitted == 0 && row.record.cancel_requested {
        OperationState::Cancelled
    } else {
        OperationState::Failed
    };
    if row.record.state.terminal() {
        let same = if desired == OperationState::Completed {
            row.record.result.as_ref() == Some(&result)
        } else {
            row.record.problem.as_ref().and_then(|p| p.get("signal")) == Some(&result)
        };
        return if row.record.state == desired && same {
            Ok(row.record)
        } else {
            Err(conflict())
        };
    }
    if !matches!(
        row.record.state,
        OperationState::Preparing | OperationState::Unknown
    ) {
        return Err(conflict());
    }
    c.execute("UPDATE operation_effects SET state=?2 WHERE operation_record_id=?1 AND provider='native-stream' AND state IN ('submitting','unknown')",params![record,if unknown>0 {"unknown"} else if submitted==total {"confirmed"} else {"failed"}]).map_err(|_|Error::storage())?;
    let problem = if desired == OperationState::Completed {
        None
    } else {
        let (code, message) = match desired {
            OperationState::Unknown => (
                "signal-outcome-unknown",
                "DTMF local submission outcome is uncertain",
            ),
            OperationState::Cancelled => (
                "signal-cancelled",
                "DTMF was cancelled without local submission",
            ),
            _ => (
                if submitted == 0 {
                    "signal-not-submitted"
                } else {
                    "partial-submission"
                },
                "DTMF was not submitted to every intended recipient",
            ),
        };
        Some(serde_json::json!({"code":code,"message":message,"signal":result}))
    };
    let same_problem = row.record.problem == problem;
    c.execute(
        "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
        params![record, problem.as_ref().map(serde_json::Value::to_string)],
    )
    .map_err(|_| Error::storage())?;
    let mut row = row;
    if desired == OperationState::Completed && row.record.state == OperationState::Preparing {
        rows::change(c, &row, OperationState::Committing, None, store)?;
        row = rows::by_record(c, record)?;
    }
    if desired == row.record.state {
        if !same_problem {
            c.execute(
                "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
                params![record, crate::now()],
            )
            .map_err(|_| Error::storage())?;
            rows::event(c, record, "operation_updated", store)?;
        }
    } else {
        rows::change(
            c,
            &row,
            desired,
            if desired == OperationState::Completed {
                Some(result)
            } else {
                None
            },
            store,
        )?;
    }
    // All owned calls have stopped. Unknown remains unknown; it cannot be
    // resubmitted, but does not reserve this connection's control lane forever.
    close(c, &row)?;
    workers::invalidate(c, record)?;
    Ok(rows::by_record(c, record)?.record)
}

impl Store {
    /// Select only currently permitted original recipients using the existing
    /// invocation interval. This returns metadata, never a queued IO grant.
    pub fn select_native_dtmf_recipients_with_peer(
        &self,
        lease: &NativeStreamLease,
        source: &NativeMediaOrigin,
        candidates: Vec<NativeDtmfRecipient>,
        proof: &PeerAuthorityGuard,
    ) -> Result<Vec<NativeDtmfRecipient>> {
        let credential = proof
            .request
            .credentials
            .last()
            .ok_or_else(Error::forbidden)?;
        let projected = self.validate_native_stream_attempt(credential, proof, lease)?;
        if intent(proof)?.command != Command::Dtmf
            || projected.parent_record_id != source.offer_record_id()
            || candidates.len() > self.limits().connections.min(512)
        {
            return Err(Error::forbidden());
        }
        self.validate_native_dtmf_source_with_peer(source, proof, lease)?;
        let c = self.try_connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let mut selected = Vec::new();
        for target in candidates {
            match self.native_dtmf_pair_at(&c, source, &target.origin) {
                Ok((scopes, expires))
                    if expires > crate::now()
                        && self.authority_covers(Store::peer_scopes(proof), &scopes) =>
                {
                    selected.push(target)
                }
                Ok(_) => return Err(Error::forbidden()),
                Err(error) if matches!(error.code.as_str(), "forbidden" | "not-recipient") => (),
                Err(error) => return Err(error),
            }
        }
        self.require_peer_source(&c, proof, credential)?;
        Ok(selected)
    }
    /// Trusted accepted-observation boundary. Freeze exact original source and
    /// intended registrations under this invocation's existing complete readers.
    /// A successful freeze never reconstructs another batch from the same lease.
    pub fn prepare_native_dtmf_with_peer(
        &self,
        lease: &NativeStreamLease,
        source: Arc<NativeMediaOrigin>,
        recipients: Vec<NativeDtmfRecipient>,
        proof: &PeerAuthorityGuard,
        observation: SourceObservation,
    ) -> Result<NativeDtmfBatch> {
        observation.validate()?;
        let credential = proof
            .request
            .credentials
            .last()
            .ok_or_else(Error::forbidden)?;
        let projected = self.validate_native_stream_attempt(credential, proof, lease)?;
        let wire = intent(proof)?.clone();
        if wire.command != Command::Dtmf
            || projected.parent_record_id != source.offer_record_id()
            || observation.stream_revision != lease.expected_revision
            || observation.credential_revision != source.credential_epoch() + 1
            || observation.signing_key_id != proof.request.key_id
            || observation.signing_algorithm != proof.request.algorithm
            || recipients.is_empty()
            || recipients.len() > self.limits().connections.min(512)
        {
            return Err(Error::forbidden());
        }
        self.validate_native_dtmf_source_with_peer(&source, proof, lease)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let row = outcomes::exact(&tx, self, lease)?;
        if row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || observation.received_at.timestamp() < row.record.created_at
            || observation.received_at.timestamp() > crate::now()
            || observation.received_at.timestamp() >= lease.expires_at()
        {
            return Err(Error::forbidden());
        }
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_dtmf_sources WHERE operation_record_id=?1)",
                [lease.record_id()],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if exists {
            return Err(conflict());
        }
        let mut seen = BTreeSet::new();
        let mut registrations = BTreeSet::new();
        let mut frozen = Vec::with_capacity(recipients.len());
        for target in recipients {
            id(&target.registration_id)?;
            if !seen.insert(target.origin.offer_record_id().to_owned())
                || !registrations.insert(target.registration_id.clone())
            {
                return Err(conflict());
            }
            let (scopes, expires) = self.native_dtmf_pair_at(&tx, &source, &target.origin)?;
            if expires <= crate::now() || !self.authority_covers(Store::peer_scopes(proof), &scopes)
            {
                return Err(Error::forbidden());
            }
            let snapshot = self.native_dtmf_origin_snapshot(&tx, &target.origin)?;
            frozen.push((target, snapshot));
        }
        if !crate::conversations::capacity(&tx, frozen.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "DTMF outcome capacity exhausted",
            ));
        }
        tx.execute("INSERT INTO native_dtmf_sources(operation_record_id,runtime_epoch,observation_json) VALUES(?1,?2,?3)",params![lease.record_id(),self.runtime_epoch(),serde_json::to_string(&observation).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
        for (target, snapshot) in &frozen {
            tx.execute("INSERT INTO native_dtmf_recipients(operation_record_id,target_record_id,registration_id,target_json) VALUES(?1,?2,?3,?4)",params![lease.record_id(),target.origin.offer_record_id(),target.registration_id,snapshot.to_string()]).map_err(|_|Error::storage())?;
        }
        tx.execute(
            "UPDATE native_dtmf_sources SET sealed=1 WHERE operation_record_id=?1",
            [lease.record_id()],
        )
        .map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        tx.commit().map_err(|_| Error::storage())?;
        let source = Arc::new(Source {
            record: lease.record.clone(),
            effect: lease.effect.clone(),
            runtime: lease.runtime.clone(),
            origin: source,
            wire,
            observation,
            owner: Arc::downgrade(&self.require_runtime_owner()?),
            expires: lease.expires_at(),
        });
        let targets = frozen
            .into_iter()
            .map(|(t, snapshot)| NativeDtmfTarget {
                source: source.clone(),
                origin: t.origin,
                registration: t.registration_id,
                snapshot,
            })
            .collect();
        Ok(NativeDtmfBatch { source, targets })
    }
    /// Fresh independent named signal admission. No initial invocation is held.
    pub fn try_native_dtmf_read(&self, target: &NativeDtmfTarget) -> Result<NativeDtmfRead> {
        let media = self.try_native_signal_read(&target.origin, &target.source.origin)?;
        let c = self.try_connection()?;
        let (scopes, expires) =
            self.native_dtmf_pair_at(&c, &target.source.origin, &target.origin)?;
        if !self.authority_covers(media.interval(), &scopes) || !media.current() {
            return Err(Error::forbidden());
        }
        current(
            &c,
            self,
            &target.source,
            &target.origin,
            &target.registration,
            &target.snapshot,
        )?;
        Ok(NativeDtmfRead {
            media,
            source: target.source.clone(),
            target: target.origin.clone(),
            registration: target.registration.clone(),
            snapshot: target.snapshot.clone(),
            expires: expires.min(target.source.expires),
        })
    }
    /// Call after engine metadata preparation, before possible IO, under the
    /// same actual signal readers. Repeated acquisition is for the same frame.
    pub fn try_start_native_dtmf_write(&self, read: &NativeDtmfRead, frame: &str) -> Result<()> {
        id(frame)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        current(
            &tx,
            self,
            &read.source,
            &read.target,
            &read.registration,
            &read.snapshot,
        )?;
        let existing:Option<String>=tx.query_row("SELECT frame_id FROM native_dtmf_recipients WHERE operation_record_id=?1 AND target_record_id=?2",params![read.source.record,read.target.offer_record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if let Some(existing) = existing {
            if existing != frame {
                return Err(conflict());
            }
        } else {
            tx.execute("UPDATE native_dtmf_recipients SET attempted=1,frame_id=?3 WHERE operation_record_id=?1 AND target_record_id=?2 AND attempted=0 AND disposition IS NULL",params![read.source.record,read.target.offer_record_id(),frame]).map_err(|_|Error::storage())?;
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Trusted actual local outcome. Captured facts survive rights loss and
    /// retirement; they never authorize another write or change an old fact.
    pub fn try_record_native_dtmf_outcome(
        &self,
        target: &NativeDtmfTarget,
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
    /// Call only after all owned recipient jobs stopped and their local facts
    /// were durably recorded. This consumes no new authority and performs no IO.
    pub fn finish_native_dtmf(&self, batch: &NativeDtmfBatch) -> Result<OperationRecord> {
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        historical(&tx, self, &batch.source)?;
        let result = finish(&tx, self, &batch.source.record)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Abandoned queued work proves no write only when no frame was reserved.
    /// A reserved frame recovers as unknown; neither case schedules replay.
    pub fn recover_native_dtmf(&self, limit: u32) -> Result<u32> {
        if !(1..=256).contains(&limit) {
            return Err(invalid());
        }
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let records = {
            let mut q=tx.prepare("SELECT d.operation_record_id FROM native_dtmf_sources d JOIN operations o ON o.record_id=d.operation_record_id JOIN native_stream_operations n ON n.operation_record_id=o.record_id WHERE n.closed=0 AND (d.runtime_epoch!=?1 OR o.expires_at<=?2) ORDER BY d.operation_record_id LIMIT ?3").map_err(|_|Error::storage())?;
            let rows = q
                .query_map(params![self.runtime_epoch(), crate::now(), limit], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            rows
        };
        for record in &records {
            let pending = {
                let mut q=tx.prepare("SELECT target_record_id,attempted FROM native_dtmf_recipients WHERE operation_record_id=?1 AND disposition IS NULL ORDER BY target_record_id").map_err(|_|Error::storage())?;
                let rows = q
                    .query_map([record], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?))
                    })
                    .map_err(|_| Error::storage())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|_| Error::storage())?;
                rows
            };
            for (target, attempted) in pending {
                outcome(
                    &tx,
                    self,
                    record,
                    &target,
                    if attempted {
                        Disposition::OutcomeUnknown
                    } else {
                        Disposition::NotStarted
                    },
                    true,
                )?;
            }
            finish(&tx, self, record)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(records.len() as u32)
    }
}
