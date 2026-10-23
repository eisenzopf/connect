//! A separate Session-policy read admits one exact native codec child. No
//! stream.subscribe or target-initiated connection.update grant is borrowed.
use super::*;
use crate::NativeSessionCodecRead;

impl Store {
    pub fn begin_native_session_codec(
        &self,
        source: &NativeSessionCodecRead,
        operation: &str,
        plan: serde_json::Value,
    ) -> Result<NativeStreamLease> {
        source.require(self)?;
        let view = source.media.target();
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let preferences:String=tx.query_row("SELECT codec_preferences_json FROM session_codec_batches WHERE operation_record_id=?1",[&source.batch],|r|r.get(0)).map_err(|_|Error::storage())?;
        let wire = Intent {
            operation_id: operation.into(),
            conversation_id: view.connection.conversation_id.clone(),
            session_id: view.connection.session_id.clone(),
            connection_id: view.connection.connection_id.clone(),
            expected_revision: Some(source.revision),
            by_participant: None,
            command: Command::Update,
            payload: serde_json::json!({"action":"codec-renegotiate","codec_preferences":serde_json::from_str::<serde_json::Value>(&preferences).map_err(|_|Error::storage())?,"session_codec_batch":source.batch,"plan":plan}),
        };
        wire.validate()?;
        if wire.payload["plan"]
            .as_object()
            .is_none_or(|p| p.is_empty())
        {
            return Err(conflict());
        }
        let (hash, json) = digest(&serde_json::to_value(&wire).map_err(|_| Error::storage())?)?;
        let actor = actor_from_connection(&tx, &source.credential)?;
        // The source read independently checked current original connection.offer
        // rights. Session management is the initiating command for this effect.
        actor.require("connection.offer")?;
        let tenant = authority::tenant(&tx, &actor)?;
        if rows::find(&tx, &tenant, &actor.identity_id, operation)?.is_some() {
            return Err(conflict());
        }
        crate::operations::reject_control_id(&tx, &actor, operation)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_codec_targets t JOIN session_codec_batches b ON b.operation_record_id=t.batch_record_id JOIN operations batch ON batch.record_id=b.operation_record_id JOIN session_control s ON s.session_id=batch.session_id JOIN connections c ON c.id=t.connection_id JOIN native_connection_lifetimes life ON life.connection_id=c.id AND life.operation_record_id=?2 JOIN native_admission_sources parent ON parent.operation_record_id=life.operation_record_id JOIN operations offered ON offered.record_id=parent.operation_record_id JOIN operation_effects effect ON effect.id=parent.effect_id AND effect.operation_record_id=offered.record_id WHERE t.batch_record_id=?1 AND t.connection_id=?3 AND t.generation=?4 AND t.state='queued' AND t.native_operation_record_id IS NULL AND b.runtime_epoch=?5 AND b.expires_at>?6 AND s.codec_policy_operation_id=b.policy_operation_id AND batch.state IN ('accepted','preparing') AND batch.cancel_requested=0 AND c.session_id=batch.session_id AND c.generation=t.generation AND c.state IN ('ready','active') AND c.observed_runtime_id=?5 AND json_extract(c.readiness_json,'$.stream_revision')=?7 AND life.retired=0 AND life.generation=c.generation AND life.runtime_epoch=?5 AND parent.attempt_runtime=?5 AND parent.generation=c.generation AND ((?11=0 AND parent.credential_id=?8 AND parent.key_id=?9 AND parent.device_id=?10 AND parent.source_expires_at>?6 AND NOT EXISTS(SELECT 1 FROM native_media_credential_bindings r WHERE r.parent_record_id=parent.operation_record_id AND r.runtime_epoch=?5 AND r.state='applied')) OR EXISTS(SELECT 1 FROM native_media_credential_bindings r WHERE r.parent_record_id=parent.operation_record_id AND r.runtime_epoch=?5 AND r.state='applied' AND r.epoch=?11 AND r.credential_id=?8 AND r.key_id=?9 AND r.device_id=?10 AND r.source_expires_at>?6 AND NOT EXISTS(SELECT 1 FROM native_media_credential_bindings newer WHERE newer.parent_record_id=r.parent_record_id AND newer.runtime_epoch=r.runtime_epoch AND newer.state='applied' AND newer.epoch>r.epoch))) AND offered.state='completed' AND effect.state='confirmed' AND NOT EXISTS(SELECT 1 FROM native_stream_operations busy WHERE busy.parent_record_id=parent.operation_record_id AND busy.closed=0))",params![source.batch,source.parent,wire.connection_id,view.connection.generation,self.runtime_epoch(),crate::now(),source.revision,source.credential,source.key,source.device,source.credential_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
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
        let expires = source.expires_at.min(crate::now() + 30);
        if expires <= crate::now() {
            return Err(Error::forbidden());
        }
        capacity(&tx, self, 1)?;
        if !crate::conversations::capacity(&tx, 6, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Session codec journal capacity exhausted",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,'connection.update',?6,?7,?8,?9,?10,'accepted',?11,?11,?12)",params![record,tenant,actor.identity_id,operation,hash,wire.conversation_id,wire.session_id,wire.connection_id,source.revision,json,crate::now(),expires]).map_err(|_|Error::storage())?;
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
        let changed=tx.execute("UPDATE session_codec_targets SET state='preparing',native_operation_record_id=?3 WHERE batch_record_id=?1 AND connection_id=?2 AND state='queued' AND native_operation_record_id IS NULL",params![source.batch,wire.connection_id,record]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        super::super::session_codecs::settle(&tx, &source.batch, self)?;
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
            command: Command::Update,
            requested_revision: Some(source.revision),
            data_message: None,
            quality_interval_ms: None,
            gate_payload: None,
            capability_payload: None,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(lease)
    }
    /// This validates the same admitted attempt; it never recovers a lease from
    /// an ID or grants another attempt after an uncertain native call.
    pub fn validate_native_session_codec(
        &self,
        source: &NativeSessionCodecRead,
        lease: &NativeStreamLease,
    ) -> Result<()> {
        source.require(self)?;
        let c = self.try_connection()?;
        let row = outcomes::exact(&c, self, lease)?;
        let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_codec_targets t JOIN session_codec_batches b ON b.operation_record_id=t.batch_record_id JOIN operations batch ON batch.record_id=b.operation_record_id JOIN session_control s ON s.session_id=batch.session_id WHERE t.batch_record_id=?1 AND t.native_operation_record_id=?2 AND t.connection_id=?3 AND t.generation=?4 AND t.state='preparing' AND s.codec_policy_operation_id=b.policy_operation_id AND b.runtime_epoch=?5 AND b.expires_at>?6 AND batch.cancel_requested=0)",params![source.batch,lease.record,lease.connection,lease.generation,self.runtime_epoch(),crate::now()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact
            || lease.command != Command::Update
            || row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || lease.expires_at <= crate::now()
            || lease.parent != source.parent
            || lease.expected_revision != source.revision
            || lease.core_connection != source.media.target().runtime_connection_id
        {
            return Err(Error::forbidden());
        }
        source.require(self)
    }
}
