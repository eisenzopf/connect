//! Retain a proven local business rejection without inventing a physical slot.
use super::*;
impl Store {
    pub(super) fn rejected_native_preparation_at(
        &self,
        c: &Connection,
        credential: &str,
        proof: &PeerAuthorityGuard,
        wire: &OfferIntent,
        hash: &str,
        json: &str,
    ) -> Result<Option<OperationRecord>> {
        let actor = actor_from_connection(c, credential)?;
        let preparation = connect_protocol::connection_control::PrepareRequest::new(
            wire.clone(),
            "rejection-only".into(),
        );
        let Some(saved) = crate::operations::receipts::receipt(
            c,
            &actor.identity_id,
            &preparation.preparation_id,
        )?
        else {
            return Ok(None);
        };
        let Err(problem) = &saved.result else {
            return Ok(None);
        };
        let connect_protocol::Operation::ConnectionPrepare { request } = &saved.operation else {
            return Err(Error::forbidden());
        };
        if request.intent != *wire {
            return Err(conflict());
        }
        self.require_peer_source(c, proof, credential)?;
        crate::operations::receipts::check_receipt_authority(c, &actor, &saved, None)?;
        crate::audience::require_original(
            c,
            &actor,
            &wire.conversation_id,
            connect_protocol::membership::ContentKind::Session,
            &wire.session_id,
        )?;
        // This is historical failure projection, never endpoint allocation or a
        // new media grant. A revoked old endpoint cannot convert failure to IO.
        let (pid, epoch) = saved.participant.as_ref().ok_or_else(Error::storage)?;
        let private = rows::find(
            c,
            &authority::tenant(c, &actor)?,
            &actor.identity_id,
            &preparation.preparation_id,
        )?
        .ok_or_else(Error::storage)?;
        if private.record.state != OperationState::Failed
            || private.record.session_id.as_deref() != Some(wire.session_id.as_str())
        {
            return Err(Error::storage());
        }
        capacity(c, self, 1)?;
        if !crate::conversations::capacity(c, 1, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Native rejection journal capacity exhausted",
            ));
        }
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        let expires = (crate::now() + OFFER_TTL).min(proof.expires_at());
        if expires <= crate::now() {
            return Err(Error::forbidden());
        }
        c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,expected_revision,intent_json,state,problem_json,created_at,updated_at,expires_at) VALUES(?1,?2,?3,?4,?5,'connection.offer',?6,?7,?8,?9,'failed',?10,?11,?11,?12)",params![record,private.tenant,actor.identity_id,wire.operation_id,hash,wire.conversation_id,private.record.session_id,wire.expected_revision,json,serde_json::to_string(problem).map_err(|_|Error::storage())?,crate::now(),expires]).map_err(|_|Error::storage())?;
        c.execute(
            "INSERT INTO native_offer_rejections VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                record,
                private.record.record_id,
                credential,
                proof.request.key_id,
                proof.key.device_id,
                proof.expires_at()
            ],
        )
        .map_err(|_| Error::storage())?;
        c.execute("INSERT INTO operation_members(operation_record_id,participant_id,identity_id,membership_epoch) VALUES(?1,?2,?3,?4)",params![record,pid,actor.identity_id,epoch]).map_err(|_|Error::storage())?;
        let row = rows::by_record(c, &record)?;
        covered(self, proof, &row)?;
        rows::event(c, &record, "operation_updated", self)?;
        Ok(Some(rows::view(c, &actor, rows::by_record(c, &record)?)?))
    }
}

impl Store {
    /// One native control entry point for local preparation and wire status.
    /// Existing wire/private receipts are read first; replay never selects a new
    /// endpoint or repeats a failed preparation. This performs no physical IO.
    pub fn prepare_native_offer_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let wire = intent(proof)?;
        let preparation = connect_protocol::connection_control::PrepareRequest::new(
            wire.clone(),
            "lookup-only".into(),
        );
        let exists = {
            let c = self.connection()?;
            self.require_peer_source(&c, proof, credential)?;
            let actor = actor_from_connection(&c, credential)?;
            c.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE owner_id=?1 AND operation_id IN (?2,?3))",params![actor.identity_id,wire.operation_id,preparation.preparation_id],|r|r.get::<_,bool>(0)).map_err(|_|Error::storage())?
        };
        if exists {
            return self.accept_native_offer_with_peer(credential, proof);
        }
        let endpoint = self.select_native_endpoint_with_peer(credential, proof)?;
        let local = self.prepare_connection_with_peer(credential, wire.clone(), endpoint, proof);
        if let Err(problem) = local {
            // A storage/authority failure may leave no retained business receipt.
            // It is never converted into a fabricated terminal wire outcome.
            let saved = {
                let c = self.connection()?;
                self.require_peer_source(&c, proof, credential)?;
                let actor = actor_from_connection(&c, credential)?;
                crate::operations::receipts::receipt(
                    &c,
                    &actor.identity_id,
                    &preparation.preparation_id,
                )?
                .is_some()
            };
            if !saved {
                return Err(problem);
            }
        }
        self.accept_native_offer_with_peer(credential, proof)
    }
}

/// Trusted native planner failure before any attempt has been issued. This is
/// not deserializable and is not a wire operation or a cleanup observation.
#[derive(Clone, Copy)]
pub enum NativeOfferPreflightRefusal {
    UnsupportedProfile,
    InvalidParameters,
    Capacity,
}
impl NativeOfferPreflightRefusal {
    fn problem(self) -> Error {
        match self {
            Self::UnsupportedProfile => Error::new(
                "unsupported-media-profile",
                "Native media profile cannot satisfy this offer",
            ),
            Self::InvalidParameters => {
                Error::new("invalid-request", "Native media parameters are invalid")
            }
            Self::Capacity => Error::new(
                "capacity-exceeded",
                "Native media profile capacity exhausted",
            ),
        }
    }
}
impl Store {
    /// The checked native planner can refuse a dormant accepted offer. A durable
    /// attempt marker forbids this path even if no physical result was observed.
    pub fn refuse_native_offer_before_effect_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        reason: NativeOfferPreflightRefusal,
    ) -> Result<OperationRecord> {
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
        covered(self, proof, &row)?;
        if !row.native
            || digest(&serde_json::to_value(wire).map_err(|_| Error::storage())?)?.0 != row.digest
        {
            return Err(conflict());
        }
        let source:Option<(bool,String,String,String)>=tx.query_row("SELECT effect_started,credential_id,key_id,device_id FROM native_offer_operations WHERE operation_record_id=?1",[&row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let (started, original, key, device) = source.ok_or_else(Error::forbidden)?;
        if started
            || original != credential
            || key != proof.request.key_id
            || device != proof.key.device_id
        {
            return Err(Error::forbidden());
        }
        let problem = serde_json::to_string(&reason.problem()).map_err(|_| Error::storage())?;
        if row.record.state == OperationState::Failed {
            let saved: Option<String> = tx
                .query_row(
                    "SELECT problem_json FROM operations WHERE record_id=?1",
                    [&row.record.record_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if saved.as_deref() != Some(problem.as_str()) {
                return Err(conflict());
            }
        } else {
            if row.record.state != OperationState::Accepted
                || row.record.cancel_requested
                || row.record.expires_at.is_none_or(|v| v <= crate::now())
            {
                return Err(Error::forbidden());
            }
            self.prepared_connection_at(&tx, credential, proof)?;
            tx.execute(
                "UPDATE operations SET problem_json=?2 WHERE record_id=?1",
                params![row.record.record_id, problem],
            )
            .map_err(|_| Error::storage())?;
            rows::change(&tx, &row, OperationState::Failed, None, self)?;
            retire_dormant(&tx, &row, self)?;
        }
        self.require_peer_source(&tx, proof, credential)?;
        let result = rows::view(&tx, &actor, rows::by_record(&tx, &row.record.record_id)?)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
