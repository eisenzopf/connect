//! Trusted runtime observations; these are not deserializable wire commands.
use super::*;
#[derive(Clone, PartialEq, Eq)]
pub struct NativeReadyObservation {
    pub private_consultation: Option<String>,
    pub handoff_preparation: bool,
    pub conversation_id: String,
    pub session_id: String,
    pub runtime_connection_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub endpoint_id: String,
    pub device_id: String,
    pub generation: u64,
    pub stream_revision: u64,
    pub participant_revision: u64,
    pub streams: Vec<String>,
}
#[derive(Clone, PartialEq, Eq)]
pub enum NativeOfferObservation {
    Ready(NativeReadyObservation),
    Retired {
        runtime_connection_id: String,
        generation: u64,
    },
}
/// The service implementation must derive this from its actual sealed native
/// activation and managed admission, or the authoritative adapter's absence.
/// Keep engine custody/fences through this call; never enter Store from here.
/// No network parser can construct an implementation or a native attempt lease.
pub trait NativeOfferRuntimeObservation: Send + Sync {
    fn observe(&self) -> Result<NativeOfferObservation>;
    /// Required for a Ready observation. Retired observations have no bindings.
    fn bindings(&self) -> Result<Vec<connect_protocol::native_streams::CodecBinding>> {
        Err(Error::forbidden())
    }
}
fn bound(c: &Connection, lease: &NativeOfferLease) -> Result<rows::Saved> {
    let row = rows::by_record(c, &lease.record)?;
    let matches:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_offer_operations n JOIN connection_offers mapping ON mapping.connection_id=?2 WHERE n.operation_record_id=?1 AND n.attempt_id=?3 AND n.attempt_runtime=?4 AND n.effect_id=?5 AND n.generation=?6 AND mapping.runtime_connection_id=?7)",params![lease.record,lease.connection,lease.attempt,lease.runtime,lease.effect,lease.generation,lease.core_connection],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !matches || row.record.connection_id.as_deref() != Some(lease.connection.as_str()) {
        return Err(Error::forbidden());
    }
    Ok(row)
}
fn fact(
    c: &Connection,
    store: &Store,
    row: &rows::Saved,
    kind: &str,
) -> Result<connect_protocol::events::ConnectionState> {
    let value=c.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[&row.record.connection_id],|r|Ok(connect_protocol::events::ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?})).map_err(|_|Error::storage())?;
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
            "Native lifecycle journal capacity exhausted",
        ));
    }
    for recipient in recipients {
        let mut data = serde_json::json!({"source_kind":"session","source_id":value.session_id,"session_id":value.session_id,"connection_id":value.connection_id,"participant_id":value.participant_id,"identity_id":row.owner,"connection":value});
        if recipient == row.owner {
            data["operation_id"] = serde_json::json!(row.record.operation_id)
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,row.record.conversation_id,kind,row.record.record_id,value.revision,data.to_string(),serde_json::json!({"kind":"connect-native-runtime","correlation_id":value.connection_id}).to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(value)
}
impl Store {
    pub fn complete_native_offer_with_peer(
        &self,
        credential: &str,
        lease: &NativeOfferLease,
        proof: &PeerAuthorityGuard,
        observation: &dyn NativeOfferRuntimeObservation,
    ) -> Result<OperationRecord> {
        self.validate_native_offer_attempt(credential, lease, proof)?;
        let observed = observation.observe()?;
        let NativeOfferObservation::Ready(ready) = &observed else {
            return Err(Error::forbidden());
        };
        if ready.runtime_connection_id != lease.core_connection
            || ready.generation != lease.generation
            || ready.stream_revision == 0
            || ready.streams.is_empty()
            || ready.streams.len() > 32
            || ready.participant_revision == 0
        {
            return Err(Error::forbidden());
        }
        let mut unique = std::collections::BTreeSet::new();
        for id in &ready.streams {
            connect_protocol::transitions::id(id)?;
            if !unique.insert(id) {
                return Err(Error::forbidden());
            }
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        self.require_peer_source(&tx, proof, credential)?;
        let row = bound(&tx, lease)?;
        let actor = actor_from_connection(&tx, credential)?;
        authority::read(&tx, &actor, &row)?;
        covered(self, proof, &row)?;
        if row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || crate::now() >= lease.expires_at
        {
            return Err(Error::forbidden());
        }
        let (_, _, projection) = self.prepared_connection_at(&tx, credential, proof)?;
        let binding = &projection.connection;
        if ready.handoff_preparation != projection.handoff_preparation
            || ready.private_consultation != projection.private_consultation
            || ready.conversation_id != binding.conversation_id
            || ready.session_id != binding.session_id
            || ready.participant_id != binding.participant_id
            || ready.membership_epoch != binding.membership_epoch
            || ready.endpoint_id != binding.endpoint_id
            || ready.device_id != projection.device_id
        {
            return Err(Error::forbidden());
        }
        let offered = projection.offer["streams_offered"]
            .as_array()
            .ok_or_else(Error::storage)?;
        if ready.streams.iter().any(|id| {
            !offered
                .iter()
                .any(|s| s["id"].as_str() == Some(id.as_str()))
        }) {
            return Err(Error::forbidden());
        }
        let bindings = observation.bindings()?;
        crate::native_stream_projection::validate(&bindings, ready.generation)?;
        let ids: std::collections::BTreeSet<_> =
            bindings.iter().map(|b| b.stream_id.as_str()).collect();
        if ids != ready.streams.iter().map(String::as_str).collect()
            || bindings.iter().any(|b| {
                !offered
                    .iter()
                    .any(|v| v["id"] == b.stream_id && v["kind"] == b.medium)
            })
        {
            return Err(Error::forbidden());
        }
        let mut details = serde_json::json!({"streams":ready.streams,"stream_revision":ready.stream_revision,"participant_revision":ready.participant_revision});
        if ready.handoff_preparation {
            details["handoff_preparation"] = serde_json::json!({"profile":"connect-handoff-preparation-v1","prepared_muted":true});
        }
        if let Some(record) = &ready.private_consultation {
            details["private_consultation"] = serde_json::json!({"operation_record_id":record,"profile":"connect-private-audio-v1","prepared_muted":true});
        }
        let mut readiness = details.clone();
        crate::native_stream_projection::replace(
            &mut readiness,
            ready.generation,
            None,
            ready.stream_revision,
            &bindings,
        )?;
        crate::native_stream_projection::save(
            &tx,
            &lease.connection,
            ready.generation,
            ready.stream_revision,
            &readiness,
        )?;
        let changed=tx.execute("UPDATE connections SET state='ready',revision=revision+1,observed_runtime_id=?2,observed_at=?3,readiness_json=?4 WHERE id=?1 AND state='preparing' AND generation=?5 AND observed_runtime_id IS NULL",params![lease.connection,self.runtime_epoch(),crate::now(),details.to_string(),lease.generation]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(Error::forbidden());
        }
        tx.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [&lease.effect],
        )
        .map_err(|_| Error::storage())?;
        // Release pending reservations and replace them with active cleanup room
        // before inserting the completed fact and ready audience fanout.
        tx.execute("UPDATE operations SET state='completed',revision=revision+1,updated_at=?2,result_json=?3 WHERE record_id=?1",params![lease.record,crate::now(),details.to_string()]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record,lease.connection,self.runtime_epoch(),lease.generation]).map_err(|_|Error::storage())?;
        rows::event(&tx, &lease.record, "operation_updated", self)?;
        fact(&tx, self, &row, "connection_ready")?;
        if observation.observe()? != observed || observation.bindings()? != bindings {
            return Err(Error::forbidden());
        }
        self.require_peer_source(&tx, proof, credential)?;
        workers::invalidate(&tx, &lease.record)?;
        let result = rows::view(&tx, &actor, rows::by_record(&tx, &lease.record)?)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Actual local adapter cleanup can settle an attempted exchange after the
    /// request/credential is gone. The installation owner and exact attempt bind
    /// it; a wire cancellation or an operator assertion cannot call this path.
    pub fn settle_native_offer_cleanup(
        &self,
        lease: &NativeOfferLease,
        observation: &dyn NativeOfferRuntimeObservation,
    ) -> Result<()> {
        if lease.runtime != self.runtime_epoch() {
            return Err(Error::forbidden());
        }
        self.settle_native_offer_cleanup_at(lease, observation)
    }
    pub(super) fn settle_native_offer_cleanup_at(
        &self,
        lease: &NativeOfferLease,
        observation: &dyn NativeOfferRuntimeObservation,
    ) -> Result<()> {
        let owner = self.require_runtime_owner()?;
        let expected = NativeOfferObservation::Retired {
            runtime_connection_id: lease.core_connection.clone(),
            generation: lease.generation,
        };
        if observation.observe()? != expected {
            return Err(Error::forbidden());
        }
        let scopes = {
            let c = self.connection()?;
            admission::row_scopes(&bound(&c, lease)?)?
        };
        let held = self.acquire_authority(
            &scopes,
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = bound(&tx, lease)?;
        admission::require_row(self, &held, &row)?;
        let retired: bool = tx
            .query_row(
                "SELECT state='retired' FROM connections WHERE id=?1",
                [&lease.connection],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !row.record.state.terminal() {
            tx.execute("UPDATE operation_effects SET state='failed' WHERE id=?1 AND state IN ('submitting','unknown')",[&lease.effect]).map_err(|_|Error::storage())?;
            let mut current = row;
            if current.record.state == OperationState::Ready {
                rows::change(&tx, &current, OperationState::Interrupted, None, self)?;
                current = rows::by_record(&tx, &lease.record)?
            }
            rows::change(
                &tx,
                &current,
                if current.record.cancel_requested {
                    OperationState::Cancelled
                } else {
                    OperationState::Failed
                },
                None,
                self,
            )?;
        }
        tx.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE operation_record_id=?1",
            [&lease.record],
        )
        .map_err(|_| Error::storage())?;
        if !retired {
            // Absence of this old Core route cannot retire a live successor.
            // Session end/leave fences the original by one generation before
            // physical cleanup: only that recorded terminal, unobserved slot
            // may finish retirement here. Exclusive successor-owner recovery also
            // settles its prior epoch's interrupted original; the exact old
            // mapping and absence observation remain required. No branch
            // grants new execution authority.
            let changed=tx.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1 AND ((generation=?2 AND (observed_runtime_id IS NULL OR observed_runtime_id=?3)) OR (observed_runtime_id IS NULL AND readiness_json IS NULL AND ((state='retiring' AND (generation=?2+1 OR (?3!=?4 AND generation>?2)) AND (EXISTS(SELECT 1 FROM sessions s WHERE s.id=connections.session_id AND s.state IN ('ended','cancelled','rejected','failed')) OR EXISTS(SELECT 1 FROM session_participants sp WHERE sp.session_id=connections.session_id AND sp.participant_id=connections.participant_id AND sp.membership_epoch=connections.membership_epoch AND sp.state IN ('left','removed')))) OR (state='interrupted' AND ?3!=?4 AND generation>?2))))",params![lease.connection,lease.generation,lease.runtime,self.runtime_epoch()]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(Error::forbidden());
            }
            let row = rows::by_record(&tx, &lease.record)?;
            fact(&tx, self, &row, "connection_retired")?;
        }
        let settled = rows::by_record(&tx, &lease.record)?;
        if let (Some(cid), Some(sid)) =
            (&settled.record.conversation_id, &settled.record.session_id)
        {
            crate::session_participant_lifecycle::reconcile(&tx, self, cid, sid, None)?;
        }
        if observation.observe()? != expected || !owner.current() {
            return Err(Error::forbidden());
        }
        workers::invalidate(&tx, &lease.record)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(())
    }
}
