//! Durable custody for native stream controls. No transport or route is changed
//! here; an engine attempt is marked uncertain before the runtime may touch it.
mod data;
pub(crate) mod dtmf;
pub use dtmf::{NativeDtmfBatch, NativeDtmfRead, NativeDtmfRecipient, NativeDtmfTarget};
pub use dtmf::{NativeDtmfGatewayRead, NativeDtmfGatewayRecipient, NativeDtmfGatewayTarget};
mod ice;
mod outcomes;
pub use ice::NativeIceRestart;
mod reconciliation;
mod session_codecs;
use super::*;
use crate::{AuthorityMode, AuthorityScope, PeerAuthorityGuard, PeerCommand};
use connect_protocol::native_streams::{Command, Intent};
pub use outcomes::{NativeStreamCommitted, NativeStreamNoEffect};

/// One local attempt, never reconstructed from a signed request or a retry.
/// Completion must later carry an attributed outcome from the owned engine job.
pub struct NativeStreamLease {
    record: String,
    attempt: String,
    effect: String,
    runtime: String,
    parent: String,
    connection: String,
    core_connection: String,
    generation: u64,
    expected_revision: u64,
    expires_at: i64,
    command: Command,
    requested_revision: Option<u64>,
    data_message: Option<String>,
    quality_interval_ms: Option<u64>,
    gate_payload: Option<serde_json::Value>,
    capability_payload: Option<serde_json::Value>,
    ice_restart: Option<NativeIceRestart>,
}
impl std::fmt::Debug for NativeStreamLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeStreamLease(redacted)")
    }
}
impl NativeStreamLease {
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn effect_id(&self) -> &str {
        &self.effect
    }
    pub fn connection_id(&self) -> &str {
        &self.connection
    }
    pub fn runtime_connection_id(&self) -> &str {
        &self.core_connection
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn expected_revision(&self) -> u64 {
        self.expected_revision
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}
fn intent(proof: &PeerAuthorityGuard) -> Result<&Intent> {
    match &proof.request.command {
        PeerCommand::NativeStream(intent) => {
            if intent.command == Command::Reconcile {
                return Err(Error::forbidden());
            }
            intent.validate()?;
            Ok(intent)
        }
        _ => Err(Error::forbidden()),
    }
}
pub(super) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    actor.require(match row.record.command_type.as_str() {
        "message.send" => "send",
        "stream.reconcile" => "stream.subscribe",
        other => other,
    })?;
    let (pid, epoch, resources): (String, u64, String) = c.query_row(
        "SELECT participant_id,membership_epoch,resources_json FROM native_stream_operations WHERE operation_record_id=?1",
        [&row.record.record_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|_|Error::storage())?;
    let cid = row
        .record
        .conversation_id
        .as_deref()
        .ok_or_else(Error::storage)?;
    let sid = row
        .record
        .session_id
        .as_deref()
        .ok_or_else(Error::storage)?;
    let current = crate::audience::member(c, &actor.identity_id, cid, true)?;
    if current.participant_id != pid || current.membership_epoch != epoch {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        actor,
        cid,
        connect_protocol::membership::ContentKind::Session,
        sid,
    )?;
    let resources: Vec<String> = serde_json::from_str(&resources).map_err(|_| Error::storage())?;
    // DTMF's frozen recipients carry its content audience. Original offer
    // resources are only a locking footprint for this independent signal.
    if row.record.command_type != "dtmf.send"
        && resources.iter().any(|id| !actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    if row.record.command_type == "message.send" {
        data::read(c, actor, row)?;
    }
    if row.record.command_type == "dtmf.send" {
        dtmf::recipient_scopes(c, actor, &row.record.record_id)?;
    }
    Ok(())
}
/// Retry/status admission uses retained original provenance even after the real
/// parent retires. This grants metadata only; a new attempt needs a live source.
pub(crate) fn retained_scopes(
    c: &Connection,
    actor: &Actor,
    wire: &Intent,
) -> Result<Option<Vec<AuthorityScope>>> {
    let Some(row) = rows::find(
        c,
        &authority::tenant(c, actor)?,
        &actor.identity_id,
        &wire.operation_id,
    )?
    else {
        return Ok(None);
    };
    authority::read(c, actor, &row)?;
    if !row.native_stream
        || row.record.command_type != wire.command.name()
        || row.record.conversation_id.as_deref() != Some(wire.conversation_id.as_str())
        || row.record.session_id.as_deref() != Some(wire.session_id.as_str())
        || row.record.connection_id.as_deref() != Some(wire.connection_id.as_str())
    {
        return Err(conflict());
    }
    let member = crate::audience::member(c, &actor.identity_id, &wire.conversation_id, true)?;
    if wire
        .by_participant
        .as_ref()
        .is_some_and(|p| p != &member.participant_id)
    {
        return Err(Error::forbidden());
    }
    let mut scopes = admission::row_scopes(&row)?;
    if wire.command == Command::Dtmf {
        scopes.extend(dtmf::recipient_scopes(c, actor, &row.record.record_id)?);
    }
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        &wire.conversation_id,
        AuthorityMode::Read,
    )?);
    let resources: String = c
        .query_row(
            "SELECT resources_json FROM native_stream_operations WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if wire.command == Command::Data {
        if let Some(members) = crate::peer_authority::native_data::frozen(c, actor, wire)? {
            for member in members {
                scopes.push(AuthorityScope::identity(
                    &member.identity_id,
                    AuthorityMode::Read,
                )?);
            }
        }
    }
    for resource in serde_json::from_str::<Vec<String>>(&resources).map_err(|_| Error::storage())? {
        scopes.push(AuthorityScope::identity(&resource, AuthorityMode::Read)?);
    }
    Ok(Some(AuthorityScope::merged(&scopes)?))
}
fn close(c: &Connection, row: &rows::Saved) -> Result<()> {
    c.execute(
        "UPDATE native_stream_operations SET closed=1 WHERE operation_record_id=?1",
        [&row.record.record_id],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
fn fail_before_effect(
    c: &Connection,
    row: &rows::Saved,
    problem: &Error,
    store: &Store,
) -> Result<()> {
    c.execute(
        "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
        params![
            row.record.record_id,
            serde_json::to_string(problem).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    rows::change(c, row, OperationState::Failed, None, store)?;
    close(c, row)
}
pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<()> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(());
    }
    let (started,runtime): (bool,Option<String>) = c.query_row("SELECT effect_started,attempt_runtime FROM native_stream_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    let expired = row.record.expires_at.is_none_or(|t| t <= crate::now());
    let abandoned = started
        && (runtime.as_deref() != Some(store.runtime_epoch())
            || row.lease_until.is_none_or(|t| t <= crate::now()));
    if expired || abandoned {
        if started {
            c.execute("UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND state='submitting'",[&row.record.record_id]).map_err(|_|Error::storage())?;
            rows::change(c, row, OperationState::Unknown, None, store)?;
        } else {
            fail_before_effect(
                c,
                row,
                &Error::new("expired", "Native stream command expired before its effect"),
                store,
            )?;
        }
        workers::invalidate(c, &row.record.record_id)?;
    }
    Ok(())
}
pub(super) fn cancel(c: &Connection, row: &rows::Saved, store: &Store) -> Result<&'static str> {
    let started: bool = c
        .query_row(
            "SELECT effect_started FROM native_stream_operations WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    c.execute(
        "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
        [&row.record.record_id],
    )
    .map_err(|_| Error::storage())?;
    if started {
        c.execute("UPDATE operation_effects SET state='unknown' WHERE operation_record_id=?1 AND state='submitting'",[&row.record.record_id]).map_err(|_|Error::storage())?;
        if row.record.state == OperationState::Unknown {
            c.execute(
                "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
                params![row.record.record_id, crate::now()],
            )
            .map_err(|_| Error::storage())?;
            rows::event(c, &row.record.record_id, "operation_updated", store)?;
        } else {
            rows::change(c, row, OperationState::Unknown, None, store)?;
        }
        workers::invalidate(c, &row.record.record_id)?;
        // Until actual local reconciliation, this connection's control slot
        // remains reserved. Cancellation cannot prove a route was unchanged.
        Ok("reconciliation_required")
    } else {
        rows::change(c, row, OperationState::Cancelled, None, store)?;
        close(c, row)?;
        Ok("cancelled_before_commit")
    }
}
fn checked_row(
    c: &Connection,
    store: &Store,
    credential: &str,
    proof: &PeerAuthorityGuard,
) -> Result<rows::Saved> {
    store.require_peer_source(c, proof, credential)?;
    let actor = actor_from_connection(c, credential)?;
    let wire = intent(proof)?;
    let row = rows::find(
        c,
        &authority::tenant(c, &actor)?,
        &actor.identity_id,
        &wire.operation_id,
    )?
    .ok_or_else(Error::forbidden)?;
    authority::read(c, &actor, &row)?;
    let (hash, _) = digest(&serde_json::to_value(wire).map_err(|_| Error::storage())?)?;
    if !row.native_stream || row.digest != hash {
        return Err(conflict());
    }
    admission::require_row(store, Store::peer_scopes(proof), &row)?;
    Ok(row)
}
impl Store {
    /// Pure authorized replay lookup. A foreign physical caller must not turn
    /// failed preflight into failure/cancellation of an already admitted root.
    pub fn native_stream_record_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<Option<OperationRecord>> {
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = actor_from_connection(&c, credential)?;
        let wire = intent(proof)?;
        if rows::find(
            &c,
            &authority::tenant(&c, &actor)?,
            &actor.identity_id,
            &wire.operation_id,
        )?
        .is_none()
        {
            return Ok(None);
        }
        let row = checked_row(&c, self, credential, proof)?;
        Ok(Some(rows::view(&c, &actor, row)?))
    }
    /// Retain exact stream intent in the common operation namespace before
    /// any engine action. Replay never creates a replacement effect or expiry.
    pub fn accept_native_stream_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        self.accept_native_stream_at(credential, proof, false)?
            .ok_or_else(conflict)
    }
    /// None is a read-only refusal before admission: only an internal managed
    /// reconciliation occupies the original connection. The caller may wait
    /// within its original deadline, retaining this exact terminal or data intent.
    /// Only End/Data use this wait; revision-sensitive controls keep their
    /// ordinary admission. Errors and accepted operations leave this mechanism.
    pub fn try_accept_native_control_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<Option<OperationRecord>> {
        if !matches!(intent(proof)?.command, Command::End | Command::Data) {
            return Err(Error::forbidden());
        }
        self.accept_native_stream_at(credential, proof, true)
    }
    fn accept_native_stream_at(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        wait_managed: bool,
    ) -> Result<Option<OperationRecord>> {
        let wire = intent(proof)?;
        let (hash, json) = digest(&serde_json::to_value(wire).map_err(|_| Error::storage())?)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let actor = actor_from_connection(&tx, credential)?;
        let tenant = authority::tenant(&tx, &actor)?;
        if rows::find(&tx, &tenant, &actor.identity_id, &wire.operation_id)?.is_some() {
            let row = checked_row(&tx, self, credential, proof)?;
            recover_one(&tx, &row, self)?;
            let result = rows::view(&tx, &actor, rows::by_record(&tx, &row.record.record_id)?)?;
            self.require_peer_source(&tx, proof, credential)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(Some(result));
        }
        crate::operations::reject_control_id(&tx, &actor, &wire.operation_id)?;
        let source = self.native_stream_projection_at(&tx, &actor, proof)?;
        let occupied:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations WHERE parent_record_id=?1 AND closed=0)",[&source.parent_record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if occupied {
            if wait_managed {
                let competing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.parent_record_id=?1 AND n.closed=0 AND o.command_type!='stream.reconcile')", [&source.parent_record_id], |r|r.get(0)).map_err(|_|Error::storage())?;
                if !competing {
                    // No ledger/effect/event has been written. Drop this SQL
                    // transaction before the caller waits for reconciliation.
                    return Ok(None);
                }
            }
            return Err(conflict());
        }
        let (resources,parent_expiry):(String,i64)=tx.query_row("SELECT resources_json,source_expires_at FROM native_admission_sources WHERE operation_record_id=?1",[&source.parent_record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
        let source_expiry = proof.expires_at().min(parent_expiry);
        // A valid DTMF sequence itself may consume the full 30 seconds.
        // Admission, metadata capture and terminal packet redundancy need
        // their own bounded margin, still inside the original source expiry.
        let budget = if wire.command == Command::Dtmf {
            40
        } else {
            30
        };
        let expires = (crate::now() + budget).min(source_expiry);
        if expires <= crate::now() {
            return Err(Error::forbidden());
        }
        capacity(&tx, self, 2)?;
        if !crate::conversations::capacity(&tx, 6, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Native stream journal capacity exhausted",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'accepted',?12,?12,?13)",params![record,tenant,actor.identity_id,wire.operation_id,hash,wire.command.name(),wire.conversation_id,wire.session_id,wire.connection_id,wire.expected_revision,json,crate::now(),expires]).map_err(|_|Error::storage())?;
        let connection = &source.connection.connection;
        tx.execute("INSERT INTO native_stream_operations(operation_record_id,parent_record_id,credential_id,key_id,device_id,participant_id,membership_epoch,generation,resources_json,source_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![record,source.parent_record_id,credential,proof.request.key_id,proof.key.device_id,connection.participant_id,connection.membership_epoch,connection.generation,resources,source_expiry]).map_err(|_|Error::storage())?;
        if wire.is_ice_restart() {
            ice::accept(&tx, wire, &record, &source)?;
        }
        tx.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) VALUES(?1,?2,?3,?4)",params![record,connection.participant_id,actor.identity_id,connection.membership_epoch]).map_err(|_|Error::storage())?;
        if wire.command == Command::Data {
            data::accept(&tx, self, &actor, wire, &record, &source)?;
        }
        rows::event(&tx, &record, "operation_accepted", self)?;
        if wire
            .expected_revision
            .is_some_and(|r| r != source.stream_revision)
        {
            fail_before_effect(&tx, &rows::by_record(&tx, &record)?, &conflict(), self)?;
        }
        self.require_peer_source(&tx, proof, credential)?;
        let result = rows::view(&tx, &actor, rows::by_record(&tx, &record)?)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(result))
    }
    /// Mark the one local attempt before calling Core. Only the same current
    /// native owner can execute; a metadata retry under another key cannot.
    pub fn begin_native_stream_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<Option<(NativeStreamLease, OperationRecord)>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = checked_row(&tx, self, credential, proof)?;
        recover_one(&tx, &row, self)?;
        let row = rows::by_record(&tx, &row.record.record_id)?;
        if row.record.state != OperationState::Accepted {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        let actor = actor_from_connection(&tx, credential)?;
        let source = self.native_stream_projection_at(&tx, &actor, proof)?;
        let (parent,original,key,device,started):(String,String,String,String,bool)=tx.query_row("SELECT parent_record_id,credential_id,key_id,device_id,effect_started FROM native_stream_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|Error::storage())?;
        if original != credential
            || key != proof.request.key_id
            || device != proof.key.device_id
            || parent != source.parent_record_id
            || started
        {
            return Err(Error::forbidden());
        }
        let wire = intent(proof)?;
        if wire
            .expected_revision
            .is_some_and(|r| r != source.stream_revision)
        {
            fail_before_effect(&tx, &row, &conflict(), self)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        let attempt = format!("sattempt_{}", uuid::Uuid::new_v4().simple());
        let effect = format!("seffect_{}", uuid::Uuid::new_v4().simple());
        // Independent owners may choose the same logical operation ID.
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES(?1,?2,'native-stream',?2,'none','submitting',1,1)",params![effect,row.record.record_id]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_stream_operations SET effect_started=1,effect_id=?2,attempt_id=?3,attempt_runtime=?4 WHERE operation_record_id=?1 AND effect_started=0 AND closed=0",params![row.record.record_id,effect,attempt,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        let expires = row.record.expires_at.ok_or_else(Error::storage)?;
        tx.execute("UPDATE operations SET worker_generation=worker_generation+1,lease_owner=?2,lease_expires_at=?3,worker_runtime=?4 WHERE record_id=?1",params![row.record.record_id,attempt,expires,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        rows::change(&tx, &row, OperationState::Preparing, None, self)?;
        self.require_peer_source(&tx, proof, credential)?;
        let record = rows::view(&tx, &actor, rows::by_record(&tx, &row.record.record_id)?)?;
        let lease = NativeStreamLease {
            record: row.record.record_id.clone(),
            attempt,
            effect,
            runtime: self.runtime_epoch().into(),
            parent,
            connection: wire.connection_id.clone(),
            core_connection: source.connection.runtime_connection_id,
            generation: source.connection.connection.generation,
            expected_revision: source.stream_revision,
            command: wire.command,
            requested_revision: wire.expected_revision,
            quality_interval_ms: wire.quality_interval_ms(),
            gate_payload: (wire.command == Command::Update
                && connect_protocol::native_gates::is_gate(&wire.payload))
            .then(|| wire.payload.clone()),
            capability_payload: (wire.command == Command::Advertise).then(|| wire.payload.clone()),
            ice_restart: if wire.is_ice_restart() {
                Some(ice::load(&tx, &row.record.record_id)?)
            } else {
                None
            },
            data_message: if wire.command == Command::Data {
                Some(
                    tx.query_row(
                        "SELECT message_id FROM native_data_messages WHERE operation_record_id=?1",
                        [&row.record.record_id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?,
                )
            } else {
                None
            },
            expires_at: expires,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some((lease, record)))
    }
    /// Reject a known preflight failure without ever marking an engine effect.
    pub fn refuse_native_stream_before_effect_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        problem: Error,
    ) -> Result<OperationRecord> {
        if problem.code.is_empty() || problem.code.len() > 128 || problem.message.len() > 1024 {
            return Err(invalid());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = checked_row(&tx, self, credential, proof)?;
        let exact: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations WHERE operation_record_id=?1 AND credential_id=?2 AND key_id=?3 AND device_id=?4 AND effect_started=0)",params![row.record.record_id,credential,proof.request.key_id,proof.key.device_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        if row.record.state == OperationState::Failed {
            let saved: Option<String> = tx
                .query_row(
                    "SELECT problem_json FROM operations WHERE record_id=?1",
                    [&row.record.record_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if saved.as_deref()
                != Some(
                    serde_json::to_string(&problem)
                        .map_err(|_| Error::storage())?
                        .as_str(),
                )
            {
                return Err(conflict());
            }
        } else {
            if row.record.state != OperationState::Accepted
                || row.record.cancel_requested
                || row.record.expires_at.is_none_or(|t| t <= crate::now())
            {
                return Err(Error::forbidden());
            }
            let actor = actor_from_connection(&tx, credential)?;
            self.native_stream_projection_at(&tx, &actor, proof)?;
            fail_before_effect(&tx, &row, &problem, self)?;
        }
        let actor = actor_from_connection(&tx, credential)?;
        let result = rows::view(&tx, &actor, rows::by_record(&tx, &row.record.record_id)?)?;
        self.require_peer_source(&tx, proof, credential)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Check outside engine fences. This never creates or reuses an attempt.
    pub fn validate_native_stream_attempt(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        lease: &NativeStreamLease,
    ) -> Result<crate::NativeStreamProjection> {
        let c = self.connection()?;
        self.validate_native_stream_attempt_at(&c, credential, proof, lease)
    }
    /// Read-only source planning for the exact accepted command. This does not
    /// mint an attempt lease or allow any media effect.
    pub(crate) fn validate_native_stream_preflight_at(
        &self,
        c: &Connection,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<(crate::NativeStreamProjection, i64)> {
        let row = checked_row(c, self, credential, proof)?;
        let expires = row.record.expires_at.ok_or_else(Error::storage)?;
        if row.record.state != OperationState::Accepted
            || row.record.cancel_requested
            || crate::now() >= expires
        {
            return Err(Error::forbidden());
        }
        let actor = actor_from_connection(c, credential)?;
        let source = self.native_stream_projection_at(c, &actor, proof)?;
        let exact: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_stream_operations WHERE operation_record_id=?1 AND parent_record_id=?2 AND credential_id=?3 AND key_id=?4 AND device_id=?5 AND generation=?6 AND source_expires_at>?7 AND effect_started=0 AND effect_id IS NULL AND attempt_id IS NULL AND closed=0)",
            params![row.record.record_id, source.parent_record_id, credential, proof.request.key_id, proof.key.device_id, source.connection.connection.generation, crate::now()],
            |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if !exact
            || intent(proof)?
                .expected_revision
                .is_some_and(|r| r != source.stream_revision)
        {
            return Err(conflict());
        }
        self.require_peer_source(c, proof, credential)?;
        Ok((source, expires))
    }
    pub(crate) fn validate_native_stream_attempt_at(
        &self,
        c: &Connection,
        credential: &str,
        proof: &PeerAuthorityGuard,
        lease: &NativeStreamLease,
    ) -> Result<crate::NativeStreamProjection> {
        let row = checked_row(c, self, credential, proof)?;
        if lease.runtime != self.runtime_epoch()
            || row.record.record_id != lease.record
            || row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || crate::now() >= lease.expires_at
        {
            return Err(Error::forbidden());
        }
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id WHERE n.operation_record_id=?1 AND n.parent_record_id=?2 AND n.attempt_id=?3 AND n.effect_id=?4 AND n.attempt_runtime=?5 AND n.generation=?6 AND n.closed=0 AND n.credential_id=?7 AND n.key_id=?8 AND n.device_id=?9 AND n.source_expires_at>?10 AND o.lease_owner=n.attempt_id AND o.worker_runtime=n.attempt_runtime AND o.lease_expires_at=?11 AND e.state='submitting')",params![lease.record,lease.parent,lease.attempt,lease.effect,lease.runtime,lease.generation,credential,proof.request.key_id,proof.key.device_id,crate::now(),lease.expires_at],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let actor = actor_from_connection(c, credential)?;
        let source = self.native_stream_projection_at(c, &actor, proof)?;
        if source.parent_record_id != lease.parent
            || source.connection.runtime_connection_id != lease.core_connection
            || source.connection.connection.generation != lease.generation
            || source.stream_revision != lease.expected_revision
        {
            return Err(conflict());
        }
        self.require_peer_source(c, proof, credential)?;
        Ok(source)
    }
}

impl Store {
    /// Bounded metadata recovery only. A started command stays unknown until
    /// attributed local engine evidence settles it; it is never resubmitted.
    pub fn recover_native_streams(&self, limit: u32) -> Result<u32> {
        if !(1..=256).contains(&limit) {
            return Err(invalid());
        }
        let owner = self.require_runtime_owner()?;
        let (ids, scopes) = {
            let c = self.connection()?;
            let mut statement=c.prepare("SELECT o.record_id FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id WHERE o.state NOT IN ('completed','cancelled','failed','unknown') AND (o.expires_at<=?1 OR (n.effect_started=1 AND (n.attempt_runtime IS NOT ?2 OR o.lease_expires_at IS NULL OR o.lease_expires_at<=?1))) ORDER BY o.created_at,o.record_id LIMIT ?3").map_err(|_|Error::storage())?;
            let ids = statement
                .query_map(params![crate::now(), self.runtime_epoch(), limit], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            let mut scopes = vec![];
            for id in &ids {
                scopes.extend(admission::row_scopes(&rows::by_record(&c, id)?)?);
            }
            (ids, scopes)
        };
        let held = self.acquire_authority(
            &scopes,
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let mut count = 0;
        for id in ids {
            let row = rows::by_record(&tx, &id)?;
            admission::require_row(self, &held, &row)?;
            let before = row.record.revision;
            recover_one(&tx, &row, self)?;
            if rows::by_record(&tx, &id)?.record.revision != before {
                count += 1;
            }
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(count)
    }
}
