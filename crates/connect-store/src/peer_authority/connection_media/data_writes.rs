//! Trusted writer attribution and reserved historical facts, never IO authority.
use super::*;
use connect_protocol::native_data::{WriteDisposition, WriteObservation, WriteProvenance};
use rusqlite::TransactionBehavior;

/// Issued only for an actual native offer attempt. No credential, idle reader,
/// or strong installation ownership is retained by this token.
pub struct NativeDataWriter {
    attempt: super::NativeMediaAdmission,
    owner: Weak<InstallationOwnership>,
    conversation: String,
    session: String,
    connection: String,
    runtime_connection: String,
    participant: String,
    epoch: u64,
    generation: u64,
}
impl std::fmt::Debug for NativeDataWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeDataWriter(redacted)")
    }
}
impl NativeDataWriter {
    fn owner(&self, store: &Store) -> Result<Arc<InstallationOwnership>> {
        let owner = store.require_runtime_owner()?;
        if !self
            .owner
            .upgrade()
            .is_some_and(|o| Arc::ptr_eq(&o, &owner))
            || !owner.current()
        {
            return Err(Error::forbidden());
        }
        Ok(owner)
    }
    /// Exact historical source message and target incarnation; do not check
    /// today's credentials, membership or connection lifecycle here.
    fn source(
        &self,
        c: &Connection,
        store: &Store,
        operation: &str,
        message: &str,
        source: &str,
    ) -> Result<(String, bool)> {
        self.attempt.historical(c, store)?;
        let record: Option<(String,bool)> = c.query_row("SELECT o.record_id,EXISTS(SELECT 1 FROM native_data_recipients r WHERE r.operation_record_id=o.record_id AND r.participant_id=?9 AND r.membership_epoch=?10) FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN native_data_messages d ON d.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id JOIN connection_offers m ON m.connection_id=o.connection_id JOIN native_admission_sources target ON target.record_id=?1 JOIN native_admission_sources t ON t.operation_record_id=target.record_id WHERE o.operation_id=?2 AND d.message_id=?3 AND m.runtime_connection_id=?4 AND o.conversation_id=?5 AND o.session_id=?6 AND n.effect_started=1 AND n.attempt_runtime=?7 AND e.state IN ('submitting','confirmed','unknown') AND o.state IN ('preparing','committing','completed','unknown') AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id AND target.connection_id=?8 AND t.participant_id=?9 AND t.membership_epoch=?10 AND t.generation=?11 AND t.attempt_runtime=n.attempt_runtime", params![self.attempt.record_id(),operation,message,source,self.conversation,self.session,store.runtime_epoch(),self.connection,self.participant,self.epoch,self.generation], |r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        record.ok_or_else(Error::forbidden)
    }
}
fn reserve(c: &Connection, store: &Store, record: &str, target: &str) -> Result<()> {
    let exists: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_data_writes WHERE operation_record_id=?1 AND target_record_id=?2)", params![record,target], |r|r.get(0)).map_err(|_|Error::storage())?;
    if exists {
        return Ok(());
    }
    // The supported native runtime has at most 512 Connections. This also
    // bounds repeated cancelled/replaced target incarnations for one message.
    let count: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM native_data_writes WHERE operation_record_id=?1",
            [record],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if count >= 512 || !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Native data writer journal capacity exhausted",
        ));
    }
    c.execute("INSERT INTO native_data_writes(operation_record_id,target_record_id,runtime_epoch) VALUES(?1,?2,?3)", params![record,target,store.runtime_epoch()]).map_err(|_|Error::storage())?;
    Ok(())
}
fn finish(
    c: &Connection,
    store: &Store,
    record: &str,
    target: &str,
    disposition: WriteDisposition,
    provenance: WriteProvenance,
) -> Result<()> {
    let existing: Option<String> = c.query_row("SELECT disposition FROM native_data_writes WHERE operation_record_id=?1 AND target_record_id=?2", params![record,target], |r|r.get(0)).map_err(|_|Error::storage())?;
    if let Some(existing) = existing {
        return if existing == disposition.as_str() {
            Ok(())
        } else {
            Err(Error::new(
                "conflict",
                "Native writer outcome already recorded",
            ))
        };
    }
    let (owner,cid,sid,operation,message,source,connection,pid,epoch,generation): (String,String,String,String,String,String,String,String,u64,u64) = c.query_row("SELECT o.owner_id,o.conversation_id,o.session_id,o.operation_id,d.message_id,o.connection_id,target.connection_id,t.participant_id,t.membership_epoch,t.generation FROM operations o JOIN native_data_messages d ON d.operation_record_id=o.record_id JOIN native_admission_sources target ON target.record_id=?2 JOIN native_admission_sources t ON t.operation_record_id=target.record_id WHERE o.record_id=?1", params![record,target], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).map_err(|_|Error::storage())?;
    let event = format!("evt_{}", uuid::Uuid::new_v4().simple());
    let (sender, sender_epoch): (String, u64) = c.query_row("SELECT participant_id,membership_epoch FROM native_stream_operations WHERE operation_record_id=?1", [record], |r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let observed_at = crate::now();
    let value = WriteObservation {
        message_id: message.clone(),
        source_connection_id: source,
        target_connection_id: connection.clone(),
        participant_id: pid.clone(),
        membership_epoch: epoch,
        generation,
        disposition,
        provenance,
        observed_at,
    };
    // Consume precisely this reservation before inserting its one owner-only
    // journal fact. Both changes roll back together on storage refusal.
    c.execute("UPDATE native_data_writes SET disposition=?3,provenance=?4,observed_at=?5,journal_id=?6 WHERE operation_record_id=?1 AND target_record_id=?2 AND disposition IS NULL", params![record,target,disposition.as_str(),if provenance==WriteProvenance::LocalWriter {"local_writer"} else {"runtime_recovery"},observed_at,event]).map_err(|_|Error::storage())?;
    if !crate::conversations::capacity(c, 1, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Native writer reservation unavailable",
        ));
    }
    let data = serde_json::json!({"source_kind":"message","source_id":message,"session_id":sid,"operation_id":operation,"connection_id":connection,"participant_id":pid,"native_data_write":value});
    c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json,observed_at,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,'native_data_write',?4,1,?5,?6,?7,?8,?9)", params![event,owner,cid,record,data.to_string(),serde_json::json!({"kind":if provenance==WriteProvenance::LocalWriter {"connect-native-writer"} else {"connect-native-recovery"},"correlation_id":connection}).to_string(),observed_at,sender,sender_epoch]).map_err(|_|Error::storage())?;
    Ok(())
}
impl Store {
    pub fn native_data_writer(&self, origin: &NativeMediaOrigin) -> Result<NativeDataWriter> {
        current_owner(self, origin)?;
        let c = self.try_connection()?;
        origin.attempt.current(&c, self)?;
        Ok(NativeDataWriter {
            attempt: origin.attempt.clone(),
            owner: origin.owner.clone(),
            conversation: origin.conversation_id().into(),
            session: origin.session_id().into(),
            connection: origin.connection_id().into(),
            runtime_connection: origin.runtime_connection_id().into(),
            participant: origin.participant_id().into(),
            epoch: origin.membership_epoch(),
            generation: origin.generation(),
        })
    }
    /// Trusted delivery-policy boundary, after fresh source/target/message
    /// readers are acquired and before possible IO. This grants no permission.
    /// Refuse contention immediately; retries reserve the same original slot.
    pub fn try_reserve_native_data_write(
        &self,
        writer: &NativeDataWriter,
        operation: &str,
        message: &str,
        source: &str,
    ) -> Result<()> {
        let owner = writer.owner(self)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (record, addressed) = writer.source(&tx, self, operation, message, source)?;
        if !addressed {
            return Err(Error::forbidden());
        }
        writer.attempt.current(&tx, self)?;
        let live: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN connections c ON c.id=?2 JOIN native_connection_lifetimes life ON life.operation_record_id=?3 WHERE o.record_id=?1 AND o.cancel_requested=0 AND o.expires_at>?4 AND o.state IN ('preparing','committing','completed') AND c.generation=?5 AND c.observed_runtime_id=?6 AND c.state IN ('ready','active') AND life.retired=0)", params![record,writer.connection,writer.attempt.record_id(),crate::now(),writer.generation,self.runtime_epoch()], |r|r.get(0)).map_err(|_|Error::storage())?;
        if !live {
            return Err(Error::forbidden());
        }
        let finished: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_data_writes WHERE operation_record_id=?1 AND target_record_id=?2 AND disposition IS NOT NULL)", params![record,writer.attempt.record_id()], |r|r.get(0)).map_err(|_|Error::storage())?;
        if finished {
            return Err(Error::forbidden());
        }
        reserve(&tx, self, &record, writer.attempt.record_id())?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// Trusted observer only: map the actual native writer disposition, never
    /// a request field. Historical recording survives revocation/retirement and
    /// cannot revive a route, authorize a write, or complete source queue work.
    pub fn try_record_native_data_write(
        &self,
        writer: &NativeDataWriter,
        operation: &str,
        message: &str,
        source: &str,
        target: &str,
        disposition: WriteDisposition,
    ) -> Result<()> {
        let owner = writer.owner(self)?;
        if target != writer.runtime_connection {
            return Err(Error::forbidden());
        }
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (record, addressed) = writer.source(&tx, self, operation, message, source)?;
        if !addressed {
            // Static audience exclusion produced no IO and no recipient effect.
            // Acknowledge only the actual refusal so the native resource can
            // release cleanup custody; expose no fact about an excluded route.
            return if disposition == WriteDisposition::NotStarted && owner.current() {
                Ok(())
            } else {
                Err(Error::forbidden())
            };
        }
        let reserved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_data_writes WHERE operation_record_id=?1 AND target_record_id=?2)", params![record,writer.attempt.record_id()], |r|r.get(0)).map_err(|_|Error::storage())?;
        if !reserved {
            // A queued item can be refused before reaching current_data. It
            // made no IO attempt; record only this refusal when room exists.
            if disposition != WriteDisposition::NotStarted {
                return Err(Error::forbidden());
            }
            reserve(&tx, self, &record, writer.attempt.record_id())?;
        }
        finish(
            &tx,
            self,
            &record,
            writer.attempt.record_id(),
            disposition,
            WriteProvenance::LocalWriter,
        )?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// A predecessor reservation proves only that IO might have started. Never
    /// infer success, reissue the publication, or borrow its observer token.
    pub fn recover_native_data_writes(&self, limit: u32) -> Result<u32> {
        if !(1..=256).contains(&limit) {
            return Err(Error::new(
                "invalid-request",
                "Invalid native writer recovery limit",
            ));
        }
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let ids = {
            let mut q = tx.prepare("SELECT operation_record_id,target_record_id FROM native_data_writes WHERE disposition IS NULL AND runtime_epoch!=?1 ORDER BY operation_record_id,target_record_id LIMIT ?2").map_err(|_|Error::storage())?;
            let rows = q
                .query_map(params![self.runtime_epoch(), limit], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(|_| Error::storage())?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?
        };
        for (record, target) in &ids {
            finish(
                &tx,
                self,
                record,
                target,
                WriteDisposition::SubmissionUnknown,
                WriteProvenance::RuntimeRecovery,
            )?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ids.len() as u32)
    }
}
