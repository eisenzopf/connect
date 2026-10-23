//! Trusted server materialization uses the existing one-attempt native ledger.
use super::*;
use crate::NativeReconciliationRead;

impl Store {
    /// The caller owns the actual current read, and supplies a bounded exact
    /// engine plan descriptor. This marks one attempt before any native IO.
    /// An existing logical ID is never reconstructed into another engine lease.
    pub fn begin_native_reconciliation(
        &self,
        source: &NativeReconciliationRead,
        operation: &str,
        plan: serde_json::Value,
    ) -> Result<NativeStreamLease> {
        source.require(self)?;
        let view = source.media.target();
        let wire = Intent {
            operation_id: operation.into(),
            conversation_id: view.connection.conversation_id.clone(),
            session_id: view.connection.session_id.clone(),
            connection_id: view.connection.connection_id.clone(),
            expected_revision: Some(source.revision),
            by_participant: None,
            command: Command::Reconcile,
            payload: serde_json::json!({"subscription_record_id":source.subscription,"plan":plan}),
        };
        wire.validate()?;
        let (hash, json) = digest(&serde_json::to_value(&wire).map_err(|_| Error::storage())?)?;
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, &source.credential)?;
        actor.require("stream.subscribe")?;
        let tenant = authority::tenant(&tx, &actor)?;
        if rows::find(&tx, &tenant, &actor.identity_id, operation)?.is_some() {
            return Err(conflict());
        }
        crate::operations::reject_control_id(&tx, &actor, operation)?;
        let exact: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id JOIN connections c ON c.id=o.connection_id WHERE o.record_id=?1 AND n.parent_record_id=?2 AND o.command_type IN ('stream.subscribe','stream.unsubscribe') AND o.state='completed' AND n.closed=1 AND e.state='confirmed' AND n.credential_id=?3 AND n.key_id=?4 AND n.device_id=?5 AND c.id=?6 AND c.generation=?7 AND c.observed_runtime_id=?8 AND c.state IN ('ready','active') AND json_extract(c.readiness_json,'$.stream_revision')=?9 AND NOT EXISTS(SELECT 1 FROM native_stream_operations busy WHERE busy.parent_record_id=n.parent_record_id AND busy.closed=0) AND NOT EXISTS(SELECT 1 FROM operations newer JOIN native_stream_operations next ON next.operation_record_id=newer.record_id WHERE next.parent_record_id=n.parent_record_id AND newer.command_type IN ('stream.subscribe','stream.unsubscribe') AND newer.state='completed' AND newer.rowid>o.rowid))",params![source.subscription,source.parent,source.credential,source.key,source.device,wire.connection_id,view.connection.generation,self.runtime_epoch(),source.revision],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(conflict());
        }
        let resources: String = tx
            .query_row(
                "SELECT resources_json FROM native_admission_sources WHERE operation_record_id=?1",
                [&source.parent],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let expires = (crate::now() + 30).min(source.expires_at);
        if expires <= crate::now() {
            return Err(Error::forbidden());
        }
        capacity(&tx, self, 2)?;
        if !crate::conversations::capacity(&tx, 6, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Native reconciliation journal capacity exhausted",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,'stream.reconcile',?6,?7,?8,?9,?10,'accepted',?11,?11,?12)",params![record,tenant,actor.identity_id,operation,hash,wire.conversation_id,wire.session_id,wire.connection_id,source.revision,json,crate::now(),expires]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_stream_operations(operation_record_id,parent_record_id,credential_id,key_id,device_id,participant_id,membership_epoch,generation,resources_json,source_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![record,source.parent,source.credential,source.key,source.device,view.connection.participant_id,view.connection.membership_epoch,view.connection.generation,resources,source.expires_at]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) VALUES(?1,?2,?3,?4)",params![record,view.connection.participant_id,actor.identity_id,view.connection.membership_epoch]).map_err(|_|Error::storage())?;
        rows::event(&tx, &record, "operation_accepted", self)?;
        let effect = format!("seffect_{}", uuid::Uuid::new_v4().simple());
        let attempt = format!("sattempt_{}", uuid::Uuid::new_v4().simple());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES(?1,?2,'native-stream',?2,'none','submitting',1,1)",params![effect,record]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_stream_operations SET effect_started=1,effect_id=?2,attempt_id=?3,attempt_runtime=?4 WHERE operation_record_id=?1",params![record,effect,attempt,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operations SET worker_generation=worker_generation+1,lease_owner=?2,lease_expires_at=?3,worker_runtime=?4 WHERE record_id=?1",params![record,attempt,expires,self.runtime_epoch()]).map_err(|_|Error::storage())?;
        rows::change(
            &tx,
            &rows::by_record(&tx, &record)?,
            OperationState::Preparing,
            None,
            self,
        )?;
        source.require(self)?;
        let lease = NativeStreamLease {
            record,
            attempt,
            effect,
            runtime: self.runtime_epoch().into(),
            parent: source.parent.clone(),
            connection: wire.connection_id,
            core_connection: view.runtime_connection_id.clone(),
            generation: view.connection.generation,
            expected_revision: source.revision,
            expires_at: expires,
            command: Command::Reconcile,
            requested_revision: Some(source.revision),
            data_message: None,
            quality_interval_ms: None,
            gate_payload: None,
            capability_payload: None,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(lease)
    }
    /// Fresh check before a possible native call. Never reconstruct an attempt
    /// from an operation ID or extend the original source/attempt deadline.
    pub fn validate_native_reconciliation(
        &self,
        source: &NativeReconciliationRead,
        lease: &NativeStreamLease,
    ) -> Result<()> {
        source.require(self)?;
        let c = self.try_connection()?;
        let row = outcomes::exact(&c, self, lease)?;
        if lease.command != Command::Reconcile
            || row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || lease.expires_at <= crate::now()
            || source.parent != lease.parent
            || source.revision != lease.expected_revision
            || source.media.target().runtime_connection_id != lease.core_connection
            || row
                .intent
                .as_ref()
                .and_then(|v| serde_json::from_str::<Intent>(v).ok())
                .is_none_or(|v| v.payload["subscription_record_id"] != source.subscription)
        {
            return Err(Error::forbidden());
        }
        source.require(self)
    }
}
