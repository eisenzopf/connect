//! Separate signed-target allocation custody. No transport is allocated here.
use super::*;
use crate::{PeerAuthorityGuard, PeerCommand, PeerSessionProjection};
use connect_protocol::connection_control::OfferIntent;
mod start;
pub use start::{
    NativeReplacementAudienceCapture, NativeReplacementCommitAttempt,
    NativeReplacementCommitUncertain, NativeReplacementCommitted, NativeReplacementMediaAttempt,
    NativeReplacementMediaCapture, NativeReplacementMediaOrigin, NativeReplacementMediaPhase,
    NativeReplacementMediaRead, NativeReplacementSceneRead, NativeReplacementStartAttempt,
    NativeReplacementStartRead, NativeReplacementStartUncertain, NativeReplacementStarted,
};

pub struct NativeReplacementAllocationRead {
    source: NativeReplacementPreparationRead,
    target: PeerSessionProjection,
    descriptor: serde_json::Value,
}
impl NativeReplacementAllocationRead {
    pub fn current(&self) -> bool {
        self.source.current()
    }
    pub fn source(&self) -> &NativeReplacementPreparationRead {
        &self.source
    }
    pub fn target(&self) -> &PeerSessionProjection {
        &self.target
    }
    pub fn target_intent(&self) -> Result<OfferIntent> {
        serde_json::from_value(self.descriptor["intent"].clone()).map_err(|_| Error::storage())
    }
}
pub struct NativeReplacementAllocationAttempt {
    prepared: NativeReplacementPrepared,
    request: crate::PeerAuthorityRequest,
    attempt: String,
    effect: String,
    target: String,
    expected: serde_json::Value,
}
pub struct NativeReplacementAllocated {
    attempt: NativeReplacementAllocationAttempt,
    observation: String,
}
redacted!(
    NativeReplacementAllocationRead,
    NativeReplacementAllocationAttempt,
    NativeReplacementAllocated
);
impl NativeReplacementAllocationAttempt {
    pub fn record_id(&self) -> &str {
        &self.prepared.record
    }
    pub fn expected_allocation(&self) -> &serde_json::Value {
        &self.expected
    }
    /// Capture the actual reserved Core/peer allocation, not answer, resources,
    /// media readiness, endpoint confirmation or movement commit.
    pub fn allocated(self, observation: serde_json::Value) -> Result<NativeReplacementAllocated> {
        if observation != self.expected {
            return Err(conflict());
        }
        Ok(NativeReplacementAllocated {
            observation: crate::provisioning::canonical_json(&observation)?,
            attempt: self,
        })
    }
}
pub(super) fn current_prepared(
    store: &Store,
    c: &Connection,
    receipt: &NativeReplacementPrepared,
) -> Result<serde_json::Value> {
    local_owner(store, &receipt.owner, &receipt.runtime)?;
    receipt.selection.origin.require(store, c)?;
    let saved: Option<(String,i64)> = c.query_row("SELECT n.plan_json,n.expires_at FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' JOIN operations o ON o.record_id=n.operation_record_id AND o.state='preparing' AND o.cancel_requested=0 WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7",params![receipt.record,receipt.runtime,receipt.reservation,receipt.hash,receipt.attempt,receipt.effect,receipt.observation],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    let (raw, expires) = saved.ok_or_else(Error::forbidden)?;
    if expires <= crate::now() {
        return Err(Error::forbidden());
    }
    let saved: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
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
    if plan(store, c, &receipt.selection, &request, runtime)? != saved {
        return Err(conflict());
    }
    Ok(saved)
}
fn target_facts(
    store: &Store,
    c: &Connection,
    prepared: &NativeReplacementPrepared,
    credential: &str,
    proof: &PeerAuthorityGuard,
    saved: &serde_json::Value,
) -> Result<(connect_protocol::membership::MemberView, serde_json::Value)> {
    target_facts_for_profile(
        store,
        c,
        prepared,
        credential,
        proof,
        saved,
        "connect-replacement-preparation-v1",
    )
}
fn target_facts_for_profile(
    store: &Store,
    c: &Connection,
    prepared: &NativeReplacementPrepared,
    credential: &str,
    proof: &PeerAuthorityGuard,
    saved: &serde_json::Value,
    profile: &str,
) -> Result<(connect_protocol::membership::MemberView, serde_json::Value)> {
    store.require_peer_source(c, proof, credential)?;
    let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
        return Err(Error::forbidden());
    };
    let mut intent = intent.clone();
    intent.normalize()?;
    let marker = &intent.offer["replacement_preparation"];
    let actor = actor_from_connection(c, credential)?;
    actor.require("connection.offer")?;
    let context = prepared.selection.context(store, c)?;
    if marker.as_object().map(|v| v.len()) != Some(2)
        || marker["profile"] != profile
        || marker["operation_record_id"] != prepared.record
        || intent.offer.get("private_consultation").is_some()
        || intent.offer.get("handoff_preparation").is_some()
        || intent.conversation_id != context.conversation_id
        || intent.session_id != context.session_id
        || intent.connection_id != saved["candidate_connection_id"]
        || intent.operation_id != prepared.selection.origin.request().operation_id
        || intent.expected_revision != Some(0)
        || actor.identity_id != context.identity_id
        || proof.key.device_id != context.target_device_id
        || proof.request.device.id != context.target_device_id
        || intent
            .by_participant
            .as_ref()
            .is_some_and(|p| p != &context.participant_id)
        || intent.offer["substrate"].as_str() != Some(context.target_transport.as_str())
        || !matches!(
            context.target_transport,
            connect_protocol::provisioning::EndpointTransport::Quic
                | connect_protocol::provisioning::EndpointTransport::Webtransport
        )
    {
        return Err(Error::forbidden());
    }
    let streams = intent.offer["streams_offered"]
        .as_array()
        .ok_or_else(invalid)?;
    if streams.len() != 1 || streams[0]["kind"] != "audio" || streams[0]["direction"] != "sendrecv"
    {
        return Err(Error::new(
            "unsupported-media-profile",
            "Replacement target requires one isolated duplex audio stream",
        ));
    }
    // Reuse this exact admitted fence. Never acquire a nested original-source
    // reader behind a queued writer, nor fall back from an insufficient proof.
    let member = crate::audience::member(c, &actor.identity_id, &context.conversation_id, true)?;
    if member.participant_id != context.participant_id
        || member.membership_epoch != context.membership_epoch
    {
        return Err(Error::forbidden());
    }
    let descriptor = serde_json::json!({"intent":intent,"credential_id":credential,"key_id":proof.request.key_id,"algorithm":proof.request.algorithm,"public_key":proof.key.public_key,"device_id":proof.key.device_id,"endpoint_id":context.target_endpoint_id,"endpoint_revision":context.target_endpoint_revision});
    Ok((member, descriptor))
}

impl Store {
    /// Planning only: this can be retried on transient SQL/authority contention.
    /// The resulting owned reader is used immediately by the once-only begin.
    pub fn native_replacement_allocation_read_with_peer(
        &self,
        prepared: &NativeReplacementPrepared,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeReplacementAllocationRead> {
        let (member, descriptor) = {
            let c = self.try_connection()?;
            let saved = current_prepared(self, &c, prepared)?;
            target_facts(self, &c, prepared, credential, proof, &saved)?
        };
        let source = prepared.selection.read_with_peer(self, proof)?;
        let mut target = source.source().session().clone();
        target.actor = member;
        Ok(NativeReplacementAllocationRead {
            source,
            target,
            descriptor,
        })
    }
    pub fn begin_native_replacement_allocation_with_peer(
        &self,
        prepared: &NativeReplacementPrepared,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<(
        NativeReplacementAllocationAttempt,
        NativeReplacementAllocationRead,
    )> {
        let read =
            self.native_replacement_allocation_read_with_peer(prepared, credential, proof)?;
        self.begin_native_replacement_allocation_with_read(prepared, credential, proof, read)
    }
    /// Mark one possible allocation before invoking Core. Never retry this
    /// boundary; only its preceding metadata-only read is retryable.
    pub fn begin_native_replacement_allocation_with_read(
        &self,
        prepared: &NativeReplacementPrepared,
        credential: &str,
        proof: &PeerAuthorityGuard,
        read: NativeReplacementAllocationRead,
    ) -> Result<(
        NativeReplacementAllocationAttempt,
        NativeReplacementAllocationRead,
    )> {
        if !read.current() || read.source.selection().origin.record != prepared.record {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let saved = current_prepared(self, &tx, prepared)?;
        let (_, descriptor) = target_facts(self, &tx, prepared, credential, proof, &saved)?;
        if read.descriptor != descriptor || !read.current() {
            return Err(Error::forbidden());
        }
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='allocate')",[&prepared.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement allocation was already attempted",
            ));
        }
        let attempt = format!("nra_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        let target = crate::provisioning::canonical_json(&descriptor)?;
        if target.len() > 32768 {
            return Err(invalid());
        }
        let mut expected = saved["expected_preparation"].clone();
        expected["target_operation_id"] = descriptor["intent"]["operation_id"].clone();
        expected["target_key_id"] = descriptor["key_id"].clone();
        expected["allocated"] = serde_json::json!(true);
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-allocate',?1,'none','prepared',?3)",params![effect,prepared.record,target]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'allocate',?2,?3)",params![prepared.record,attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='allocate'",[&prepared.record]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &prepared.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            NativeReplacementAllocationAttempt {
                prepared: prepared.clone(),
                request: proof.request.clone(),
                attempt,
                effect,
                target,
                expected,
            },
            read,
        ))
    }
    /// A missing/failed allocation return is uncertainty. It never authorizes
    /// a second invocation, including after a controller or runtime changes.
    pub fn native_replacement_allocation_uncertain_with_peer(
        &self,
        attempt: NativeReplacementAllocationAttempt,
        proof: &PeerAuthorityGuard,
    ) -> Result<()> {
        self.record_replacement_allocation(&attempt, None, Some(Self::peer_scopes(proof)))
            .map(|_| ())
    }
    pub fn native_replacement_allocation_uncertain(
        &self,
        attempt: NativeReplacementAllocationAttempt,
    ) -> Result<()> {
        self.record_replacement_allocation(&attempt, None, None)
            .map(|_| ())
    }
    pub fn record_native_replacement_allocated(
        &self,
        receipt: &NativeReplacementAllocated,
    ) -> Result<OperationRecord> {
        self.record_replacement_allocation(&receipt.attempt, Some(&receipt.observation), None)
    }
    /// Reuse an already held target interval for historical capture. The owned
    /// receipt supplies attribution; this grants no new target IO or replay.
    pub fn record_native_replacement_allocated_with_peer(
        &self,
        receipt: &NativeReplacementAllocated,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        self.record_replacement_allocation(
            &receipt.attempt,
            Some(&receipt.observation),
            Some(Self::peer_scopes(proof)),
        )
    }
    fn record_replacement_allocation(
        &self,
        attempt: &NativeReplacementAllocationAttempt,
        observation: Option<&str>,
        interval: Option<&Arc<crate::AuthorityGuardSet>>,
    ) -> Result<OperationRecord> {
        let prepared = &attempt.prepared;
        local_owner(self, &prepared.owner, &prepared.runtime)?;
        let held = self.stable_authority_with(
            interval,
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &prepared.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &prepared.record)?;
        admission::require_row(self, &held, &row)?;
        let old: Option<Option<String>> = tx.query_row("SELECT s.observation_json FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='allocate' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND e.observation_json=?7",params![prepared.record,prepared.runtime,prepared.reservation,prepared.hash,attempt.attempt,attempt.effect,attempt.target],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let old = old.ok_or_else(Error::forbidden)?;
        if let Some(observation) = observation {
            if let Some(old) = old {
                if old != observation {
                    return Err(conflict());
                }
                return Ok(row.record);
            }
            if !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            ) {
                return Err(conflict());
            }
            tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='allocate'",params![prepared.record,observation]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&attempt.effect]).map_err(|_|Error::storage())?;
        } else {
            if old.is_some() {
                return Err(conflict());
            }
            tx.execute(
                "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
                [&attempt.effect],
            )
            .map_err(|_| Error::storage())?;
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1 AND state='preparing'",
                [&prepared.record],
            )
            .map_err(|_| Error::storage())?;
            workers::invalidate(&tx, &prepared.record)?;
        }
        native_handoffs::touch(&tx, &prepared.record, self)?;
        let result = rows::by_record(&tx, &prepared.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

impl Store {
    /// Exact target phase replay returns progress only. No new attempt, source
    /// grant, result or provider detail is produced, even for a terminal root.
    pub fn replay_native_replacement_allocation_with_peer(
        &self,
        captured: &NativeReplacementAllocated,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let attempt = &captured.attempt;
        let prepared = &attempt.prepared;
        local_owner(self, &prepared.owner, &prepared.runtime)?;
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = actor_from_connection(&c, credential)?;
        actor.require("connection.offer")?;
        let crate::PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let frozen: serde_json::Value =
            serde_json::from_str(&attempt.target).map_err(|_| Error::storage())?;
        if frozen["intent"] != serde_json::to_value(intent).map_err(|_| Error::storage())?
            || frozen["credential_id"] != credential
            || frozen["key_id"] != proof.request.key_id
            || frozen["device_id"] != proof.key.device_id
            || frozen["public_key"] != serde_json::json!(proof.key.public_key)
        {
            return Err(conflict());
        }
        let row = rows::by_record(&c, &prepared.record)?;
        admission::require_row(self, Self::peer_scopes(proof), &row)?;
        let mut record = row.record;
        record.result = None;
        record.problem = None;
        record.effects.clear();
        Ok(record)
    }
    /// After owned cancellation, require actual absence of this exact allocated
    /// native candidate. A prepare receipt alone can never settle this phase.
    pub fn record_native_replacement_allocation_stopped(
        &self,
        captured: &NativeReplacementAllocated,
        absent: &dyn super::super::native_offers::NativeOfferRuntimeObservation,
    ) -> Result<OperationRecord> {
        let attempt = &captured.attempt;
        let prepared = &attempt.prepared;
        local_owner(self, &prepared.owner, &prepared.runtime)?;
        let observed = absent.observe()?;
        let super::super::native_offers::NativeOfferObservation::Retired {
            runtime_connection_id,
            generation,
        } = observed
        else {
            return Err(Error::forbidden());
        };
        if serde_json::json!(runtime_connection_id) != attempt.expected["candidate"]
            || serde_json::json!(generation) != attempt.expected["generation"]
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &prepared.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &prepared.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='allocate' JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.observation_json=?7 AND e.observation_json=?8 AND s.started=1) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase IN ('start','commit') AND started=1)",params![prepared.record,prepared.runtime,prepared.reservation,prepared.hash,attempt.attempt,attempt.effect,captured.observation,attempt.target],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement candidate may have progressed beyond allocation",
            ));
        }
        let cleanup = format!("nrc_{}", attempt.attempt);
        let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE id=?1 AND operation_record_id=?2 AND compensates_effect_id=?3 AND state='confirmed')",params![cleanup,prepared.record,attempt.effect],|r|r.get(0)).map_err(|_|Error::storage())?;
        if already {
            return Ok(row.record);
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        for (id, effect, provider) in [
            (
                format!("nrc_{}", prepared.attempt),
                &prepared.effect,
                "native-replacement-prepare",
            ),
            (cleanup, &attempt.effect, "native-replacement-allocate"),
        ] {
            tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id) VALUES(?1,?2,?3,?1,'none','confirmed',1,1,?4)",params![id,prepared.record,provider,effect]).map_err(|_|Error::storage())?;
        }
        tx.execute("UPDATE operations SET state=?2,problem_json=?3 WHERE record_id=?1",params![prepared.record,if row.record.cancel_requested{"cancelled"}else{"failed"},if row.record.cancel_requested{None}else{Some(serde_json::json!({"code":"allocation-stopped","source":"native-replacement"}).to_string())}]).map_err(|_|Error::storage())?;
        workers::invalidate(&tx, &prepared.record)?;
        native_handoffs::touch(&tx, &prepared.record, self)?;
        let result = rows::by_record(&tx, &prepared.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

impl Store {
    /// Negative lifetime hint only. This metadata check grants no IO or packet
    /// reader; later phases still need their own current owned target proof.
    pub fn check_native_replacement_allocated_target(
        &self,
        captured: &NativeReplacementAllocated,
    ) -> Result<()> {
        local_owner(
            self,
            &captured.attempt.prepared.owner,
            &captured.attempt.prepared.runtime,
        )?;
        self.check_peer_authority(&captured.attempt.request)
    }
}
