//! Native wire exchanges share operation identity, journal and recovery state.
//! These methods allocate no transport and never infer media readiness.
use super::*;
use crate::{PeerAuthorityGuard, PeerCommand};
use connect_protocol::connection_control::OfferIntent;
mod observations;
mod recovery;
mod rejection;
pub use observations::{
    NativeOfferObservation, NativeOfferRuntimeObservation, NativeReadyObservation,
};
pub use recovery::NativeOfferRecoveryObservation;
pub use rejection::NativeOfferPreflightRefusal;
const OFFER_TTL: i64 = 30;

/// One server-issued attempt. Neither serialized nor constructed from wire data.
/// Once this is issued, a missing observation means uncertainty, not safe retry.
pub struct NativeOfferLease {
    record: String,
    attempt: String,
    effect: String,
    runtime: String,
    operation: String,
    connection: String,
    core_connection: String,
    generation: u64,
    expires_at: i64,
    source_expires_at: i64,
}
/// Immutable custody metadata only. Each engine read must check the ledger
/// again; retaining this value holds neither a command grant nor worker rights.
#[derive(Clone)]
pub(crate) struct NativeOfferMediaBinding {
    record: String,
    attempt: String,
    effect: String,
    runtime: String,
    connection: String,
    core_connection: String,
    generation: u64,
    expires_at: i64,
}
impl NativeOfferMediaBinding {
    /// Immutable attribution only, including after expiry or actual retirement.
    pub(crate) fn historical(&self, c: &Connection, store: &Store) -> Result<()> {
        let exact: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN connection_offers m ON m.connection_id=o.connection_id WHERE o.record_id=?1 AND o.connection_id=?2 AND n.generation=?3 AND m.runtime_connection_id=?4 AND n.effect_started=1 AND n.attempt_id=?5 AND n.attempt_runtime=?6 AND n.effect_id=?7 AND o.expires_at=?8)", params![self.record,self.connection,self.generation,self.core_connection,self.attempt,self.runtime,self.effect,self.expires_at], |r|r.get(0)).map_err(|_|Error::storage())?;
        if self.runtime != store.runtime_epoch() || !exact {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn record_id(&self) -> &str {
        &self.record
    }
    pub(crate) fn current(&self, c: &Connection, store: &Store) -> Result<Option<i64>> {
        if self.runtime != store.runtime_epoch() {
            return Err(Error::forbidden());
        }
        // The pending worker may finish only within its original deadline. Once
        // a real completion is committed, media follows current source authority
        // rather than extending or reusing the old negotiation lease.
        let state: Option<String> = c.query_row(
            "SELECT o.state FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id JOIN connection_offers mapping ON mapping.connection_id=o.connection_id WHERE o.record_id=?1 AND o.connection_id=?2 AND n.generation=?3 AND mapping.runtime_connection_id=?4 AND n.effect_started=1 AND n.attempt_id=?5 AND n.attempt_runtime=?6 AND n.effect_id=?7 AND o.cancel_requested=0 AND o.expires_at=?8 AND ((o.state='preparing' AND e.state='submitting' AND o.lease_owner=?5 AND o.worker_runtime=?6 AND o.lease_expires_at=?8 AND ?9<?8) OR (o.state='completed' AND e.state='confirmed'))",
            params![self.record,self.connection,self.generation,self.core_connection,self.attempt,self.runtime,self.effect,self.expires_at,crate::now()], |r|r.get(0),
        ).optional().map_err(|_|Error::storage())?;
        match state.as_deref() {
            Some("preparing") => Ok(Some(self.expires_at)),
            Some("completed") => Ok(None),
            _ => Err(Error::forbidden()),
        }
    }
}
impl NativeOfferLease {
    pub(crate) fn source_expires_at(&self) -> i64 {
        self.source_expires_at
    }
    pub(crate) fn media_binding(&self) -> NativeOfferMediaBinding {
        NativeOfferMediaBinding {
            record: self.record.clone(),
            attempt: self.attempt.clone(),
            effect: self.effect.clone(),
            runtime: self.runtime.clone(),
            connection: self.connection.clone(),
            core_connection: self.core_connection.clone(),
            generation: self.generation,
            expires_at: self.expires_at,
        }
    }
    pub fn effect_id(&self) -> &str {
        &self.effect
    }
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn operation_id(&self) -> &str {
        &self.operation
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
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}
impl std::fmt::Debug for NativeOfferLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeOfferLease(redacted)")
    }
}
fn intent(proof: &PeerAuthorityGuard) -> Result<&OfferIntent> {
    match &proof.request.command {
        PeerCommand::NativeConnectionOffer(intent) => Ok(intent),
        _ => Err(Error::forbidden()),
    }
}
/// The durable, never-started effect marker proves absence for this dormant
/// slot. An attempted effect must use actual attributed runtime cleanup instead.
fn retire_dormant(c: &Connection, row: &rows::Saved, store: &Store) -> Result<()> {
    let changed = c.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1 WHERE id=?1 AND state IN ('preparing','retiring') AND observed_runtime_id IS NULL AND readiness_json IS NULL AND EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.operation_record_id=?2 AND n.effect_started=0 AND n.generation<=connections.generation)",params![row.record.connection_id,row.record.record_id]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(Error::new(
            "reconciliation-required",
            "Dormant native slot requires attributed cleanup",
        ));
    }
    let value = c.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces,n.readiness_json,n.observed_runtime_id FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[&row.record.connection_id],|r|Ok(connect_protocol::events::ConnectionState {
        connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?,stream_revision:crate::events::stream_revision(r.get(8)?,r.get(10)?,r.get(11)?)?,
    })).map_err(|_|Error::storage())?;
    let resources: String = c
        .query_row(
            "SELECT resources_json FROM native_offer_operations WHERE operation_record_id=?1",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let recipients: Vec<String> = serde_json::from_str(&resources).map_err(|_| Error::storage())?;
    if !crate::conversations::capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Native retirement journal capacity exhausted",
        ));
    }
    for recipient in recipients {
        let mut data = serde_json::json!({"source_kind":"session","source_id":value.session_id,"session_id":value.session_id,"connection_id":value.connection_id,"participant_id":value.participant_id,"identity_id":row.owner,"connection":value});
        if recipient == row.owner {
            data["operation_id"] = serde_json::json!(row.record.operation_id);
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,'connection_retired',?4,?5,?6,?7)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,row.record.conversation_id,row.record.record_id,value.revision,data.to_string(),serde_json::json!({"kind":"connect-native-pre-effect-retirement","correlation_id":value.connection_id}).to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
pub(super) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    actor.require("connection.offer")?;
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
    let (pid,epoch,resources):(String,u64,String)=c.query_row(
        "SELECT participant_id,membership_epoch,resources_json FROM native_offer_operations WHERE operation_record_id=?1 UNION ALL SELECT receipt.actor_participant_id,receipt.actor_membership_epoch,receipt.resources_json FROM native_offer_rejections n JOIN operation_control_receipts receipt ON receipt.operation_record_id=n.preparation_record_id WHERE n.operation_record_id=?1",
        [&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|_|Error::storage())?;
    let member = crate::audience::member(c, &actor.identity_id, cid, true)?;
    if member.participant_id != pid || member.membership_epoch != epoch {
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
    if resources.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn recover_one(c: &Connection, row: &rows::Saved, store: &Store) -> Result<()> {
    if row.record.state.terminal() || row.record.state == OperationState::Unknown {
        return Ok(());
    }
    let (started,runtime):(bool,Option<String>)=c.query_row(
        "SELECT effect_started,attempt_runtime FROM native_offer_operations WHERE operation_record_id=?1",
        [&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_|Error::storage())?;
    let expired = row.record.expires_at.is_none_or(|v| v <= crate::now());
    let abandoned = started
        && (runtime.as_deref() != Some(store.runtime_epoch())
            || row.lease_until.is_none_or(|v| v <= crate::now()));
    if expired || abandoned {
        let next = if started {
            OperationState::Unknown
        } else {
            OperationState::Failed
        };
        if started {
            c.execute("UPDATE operation_effects SET state='unknown' WHERE id=(SELECT effect_id FROM native_offer_operations WHERE operation_record_id=?1) AND state='submitting'", [&row.record.record_id]).map_err(|_|Error::storage())?;
        }
        rows::change(c, row, next, None, store)?;
        if !started {
            retire_dormant(c, row, store)?;
        }
        // The immutable native attempt remains attributed even after a worker
        // is fenced. A later request cannot reuse it to submit another effect.
        super::workers::invalidate(c, &row.record.record_id)?;
    }
    Ok(())
}
pub(super) fn cancel(c: &Connection, row: &rows::Saved, store: &Store) -> Result<&'static str> {
    let started: bool = c
        .query_row(
            "SELECT effect_started FROM native_offer_operations WHERE operation_record_id=?1",
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
        c.execute("UPDATE operation_effects SET state='unknown' WHERE id=(SELECT effect_id FROM native_offer_operations WHERE operation_record_id=?1) AND state='submitting'", [&row.record.record_id]).map_err(|_|Error::storage())?;
        let mut row = rows::by_record(c, &row.record.record_id)?;
        if row.record.state == OperationState::Ready {
            rows::change(c, &row, OperationState::Interrupted, None, store)?;
            row = rows::by_record(c, &row.record.record_id)?;
        }
        if row.record.state == OperationState::Unknown {
            c.execute(
                "UPDATE operations SET revision=revision+1,updated_at=?2 WHERE record_id=?1",
                params![row.record.record_id, crate::now()],
            )
            .map_err(|_| Error::storage())?;
            rows::event(c, &row.record.record_id, "operation_updated", store)?;
        } else {
            rows::change(c, &row, OperationState::Unknown, None, store)?;
        }
        super::workers::invalidate(c, &row.record.record_id)?;
        Ok("reconciliation_required")
    } else {
        rows::change(c, row, OperationState::Cancelled, None, store)?;
        // The durable pre-effect marker proves this slot never entered native
        // allocation. No observation is invented for an attempted exchange.
        retire_dormant(c, row, store)?;
        Ok("cancelled_before_commit")
    }
}
fn covered(store: &Store, proof: &PeerAuthorityGuard, row: &rows::Saved) -> Result<()> {
    if !store.authority_covers(Store::peer_scopes(proof), &admission::row_scopes(row)?) {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    pub fn accept_native_offer_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let mut wire = intent(proof)?.clone();
        wire.normalize()?;
        let (hash, json) = digest(&serde_json::to_value(&wire).map_err(|_| Error::storage())?)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let actor = actor_from_connection(&tx, credential)?;
        let tenant = authority::tenant(&tx, &actor)?;
        if let Some(row) = rows::find(&tx, &tenant, &actor.identity_id, &wire.operation_id)? {
            authority::read(&tx, &actor, &row)?;
            if !row.native || row.digest != hash {
                return Err(conflict());
            }
            covered(self, proof, &row)?;
            recover_one(&tx, &row, self)?;
            let row = rows::by_record(&tx, &row.record.record_id)?;
            let result = rows::view(&tx, &actor, row)?;
            self.require_peer_source(&tx, proof, credential)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        crate::operations::reject_control_id(&tx, &actor, &wire.operation_id)?;
        if let Some(record) =
            self.rejected_native_preparation_at(&tx, credential, proof, &wire, &hash, &json)?
        {
            self.require_peer_source(&tx, proof, credential)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(record);
        }
        crate::session_reconnect::permit_offer(&tx, self, &wire.session_id)?;
        let (preparation, _, view) = self.prepared_connection_at(&tx, credential, proof)?;
        let resources:String=tx.query_row("SELECT resources_json FROM operation_control_receipts WHERE operation_record_id=?1",[&preparation],|r|r.get(0)).map_err(|_|Error::storage())?;
        let recipients: Vec<String> =
            serde_json::from_str(&resources).map_err(|_| Error::storage())?;
        let reserve = recipients.len() as u64 + 1;
        capacity(&tx, self, 2)?; // root and the first explicit cancel decision
        if !crate::conversations::capacity(&tx, 1 + 2 * reserve + 2, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Native offer journal capacity exhausted",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        let expires = (crate::now() + OFFER_TTL).min(proof.expires_at());
        if expires <= crate::now() {
            return Err(Error::forbidden());
        }
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,'connection.offer',?6,?7,?8,?9,?10,'accepted',?11,?11,?12)",
            params![record,tenant,actor.identity_id,wire.operation_id,hash,wire.conversation_id,wire.session_id,wire.connection_id,wire.expected_revision,json,crate::now(),expires]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_offer_operations(operation_record_id,preparation_record_id,credential_id,key_id,device_id,endpoint_id,participant_id,membership_epoch,generation,resources_json,source_expires_at,terminal_reserve) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![record,preparation,credential,proof.request.key_id,proof.key.device_id,view.connection.endpoint_id,view.connection.participant_id,view.connection.membership_epoch,view.connection.generation,resources,proof.expires_at(),reserve]).map_err(|_|Error::storage())?;
        tx.execute(
            "INSERT INTO native_offer_runtime_admissions VALUES(?1,?2)",
            params![record, self.runtime_epoch()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) VALUES(?1,?2,?3,?4)",params![record,view.connection.participant_id,actor.identity_id,view.connection.membership_epoch]).map_err(|_|Error::storage())?;
        rows::event(&tx, &record, "operation_accepted", self)?;
        self.require_peer_source(&tx, proof, credential)?;
        let result = rows::view(&tx, &actor, rows::by_record(&tx, &record)?)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Commit uncertainty BEFORE any physical allocation or protocol exchange.
    /// Replays/status reads never produce another attempt, even after restart.
    pub fn begin_native_offer_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<Option<(NativeOfferLease, OperationRecord)>> {
        let wire = intent(proof)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let actor = actor_from_connection(&tx, credential)?;
        let row = rows::find(
            &tx,
            &authority::tenant(&tx, &actor)?,
            &actor.identity_id,
            &wire.operation_id,
        )?
        .ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        if !row.native {
            return Err(Error::forbidden());
        }
        let (hash, _) = digest(&serde_json::to_value(wire).map_err(|_| Error::storage())?)?;
        if row.digest != hash {
            return Err(conflict());
        }
        covered(self, proof, &row)?;
        recover_one(&tx, &row, self)?;
        let row = rows::by_record(&tx, &row.record.record_id)?;
        if row.record.state != OperationState::Accepted {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        let (started,original_credential,key,device,source_expires_at):(bool,String,String,String,i64)=tx.query_row(
            "SELECT effect_started,credential_id,key_id,device_id,source_expires_at FROM native_offer_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).map_err(|_|Error::storage())?;
        if started {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        }
        if original_credential != credential
            || key != proof.request.key_id
            || device != proof.key.device_id
        {
            return Err(Error::forbidden());
        }
        let (_, _, view) = self.prepared_connection_at(&tx, credential, proof)?;
        let attempt = format!("nattempt_{}", uuid::Uuid::new_v4().simple());
        let effect = format!("neffect_{}", uuid::Uuid::new_v4().simple());
        // Wire operation IDs are owner-scoped; effect correlation keys are
        // process/provider-wide. The durable record carries the full namespace.
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES(?1,?2,'native-media',?2,'none','submitting',1,1)", params![effect,row.record.record_id]).map_err(|_|Error::storage())?;

        tx.execute("UPDATE native_offer_operations SET effect_started=1,attempt_id=?2,attempt_runtime=?3,effect_id=?4 WHERE operation_record_id=?1 AND effect_started=0",params![row.record.record_id,attempt,self.runtime_epoch(),effect]).map_err(|_|Error::storage())?;
        let expires = row.record.expires_at.ok_or_else(Error::storage)?;
        tx.execute("UPDATE operations SET worker_generation=worker_generation+1,lease_owner=?2,lease_expires_at=?3,worker_runtime=?4 WHERE record_id=?1",params![row.record.record_id,attempt,expires,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        rows::change(&tx, &row, OperationState::Preparing, None, self)?;
        self.require_peer_source(&tx, proof, credential)?;
        let record = rows::view(&tx, &actor, rows::by_record(&tx, &row.record.record_id)?)?;
        let lease = NativeOfferLease {
            record: row.record.record_id,
            attempt,
            effect,
            runtime: self.runtime_epoch().into(),
            operation: wire.operation_id.clone(),
            connection: wire.connection_id.clone(),
            core_connection: view.runtime_connection_id,
            generation: view.connection.generation,
            expires_at: expires,
            source_expires_at,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some((lease, record)))
    }
    pub fn recover_native_offers(&self, limit: u32) -> Result<u32> {
        if !(1..=256).contains(&limit) {
            return Err(invalid());
        }
        let owner = self.require_runtime_owner()?;
        let (ids, scopes) = {
            let c = self.connection()?;
            let mut statement=c.prepare("SELECT o.record_id FROM operations o JOIN native_offer_operations n ON n.operation_record_id=o.record_id WHERE o.state NOT IN ('completed','cancelled','failed','unknown') AND (o.expires_at<=?1 OR (n.effect_started=1 AND (n.attempt_runtime IS NOT ?2 OR o.lease_expires_at IS NULL OR o.lease_expires_at<=?1))) ORDER BY o.created_at,o.record_id LIMIT ?3").map_err(|_|Error::storage())?;
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

impl Store {
    /// Reservation validation for the exact in-flight native source. This is
    /// not a media grant: Core still needs current native/packet policy and the
    /// actual physical PeerAdmission. No new attempt or deadline is issued.
    pub fn validate_native_offer_attempt(
        &self,
        credential: &str,
        lease: &NativeOfferLease,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let wire = intent(proof)?;
        let c = self.try_connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = actor_from_connection(&c, credential)?;
        let row = rows::by_record(&c, &lease.record)?;
        authority::read(&c, &actor, &row)?;
        if !row.native
            || wire.operation_id != lease.operation
            || row.record.operation_id != lease.operation
            || row.owner != actor.identity_id
            || row.record.connection_id.as_deref() != Some(lease.connection.as_str())
            || row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || row.record.expires_at != Some(lease.expires_at)
            || crate::now() >= lease.expires_at
            || lease.runtime != self.runtime_epoch()
        {
            return Err(Error::forbidden());
        }
        let (hash, _) = digest(&serde_json::to_value(wire).map_err(|_| Error::storage())?)?;
        if hash != row.digest {
            return Err(conflict());
        }
        let (attempt,runtime,endpoint,effect,original_credential,key):(String,String,String,String,String,String)=c.query_row(
            "SELECT attempt_id,attempt_runtime,endpoint_id,effect_id,credential_id,key_id FROM native_offer_operations WHERE operation_record_id=?1 AND effect_started=1",
            [&lease.record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
        ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        if attempt != lease.attempt
            || runtime != lease.runtime
            || effect != lease.effect
            || original_credential != credential
            || key != proof.request.key_id
        {
            return Err(Error::forbidden());
        }
        self.require_peer_control(
            &c,
            proof,
            credential,
            &connect_protocol::Operation::ConnectionPrepare {
                request: connect_protocol::connection_control::PrepareRequest::new(
                    wire.clone(),
                    endpoint,
                ),
            },
        )?;
        let current:bool=c.query_row(
            "SELECT EXISTS(SELECT 1 FROM connections n JOIN connection_offers o ON o.connection_id=n.id JOIN session_participants sp ON sp.session_id=n.session_id AND sp.participant_id=n.participant_id AND sp.membership_epoch=n.membership_epoch JOIN sessions s ON s.id=n.session_id WHERE n.id=?1 AND n.generation=?2 AND o.runtime_connection_id=?3 AND n.identity_id=?4 AND n.state IN ('preparing','ready','active') AND (n.observed_runtime_id IS NULL OR n.observed_runtime_id=?5) AND sp.state='present' AND s.state IN ('accepted','ready','active'))",
            params![lease.connection,lease.generation,lease.core_connection,actor.identity_id,self.runtime_epoch()],|r|r.get(0),
        ).map_err(|_|Error::storage())?;
        if !current {
            return Err(Error::forbidden());
        }
        covered(self, proof, &row)?;
        self.require_peer_source(&c, proof, credential)?;
        Ok(row.record)
    }
}
