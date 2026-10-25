//! Once-only native media start and fresh provisional replacement resource reads.
//! A resource fact is not duplex readiness, endpoint confirmation or movement.
use super::*;
mod audience;
pub use audience::NativeReplacementAudienceCapture;
mod commit;
mod phase;
use crate::{AuthorityGuardSet, AuthorityScope, PeerConnectionProjection};
pub use commit::{
    NativeReplacementCommitAttempt, NativeReplacementCommitUncertain, NativeReplacementCommitted,
};
pub use phase::{
    NativeReplacementMediaAttempt, NativeReplacementMediaCapture, NativeReplacementMediaPhase,
};

pub struct NativeReplacementStartRead {
    source: NativeReplacementPreparationRead,
    target: PeerConnectionProjection,
    descriptor: serde_json::Value,
}
impl NativeReplacementStartRead {
    pub fn current(&self) -> bool {
        self.source.current()
    }
    pub fn expires_at(&self) -> i64 {
        self.source.expires_at()
    }
    pub fn source(&self) -> &NativeReplacementPreparationRead {
        &self.source
    }
    pub fn target(&self) -> &PeerConnectionProjection {
        &self.target
    }
}
pub struct NativeReplacementStartAttempt {
    allocated: Arc<NativeReplacementAllocated>,
    request: crate::PeerAuthorityRequest,
    attempt: String,
    effect: String,
    target: String,
    expected: serde_json::Value,
}
/// Captured failure after possible local native resource IO. This grants no IO.
pub struct NativeReplacementStartUncertain {
    attempt: NativeReplacementStartAttempt,
}
pub struct NativeReplacementStarted {
    attempt: NativeReplacementStartAttempt,
    observation: String,
}
/// Immutable captured provenance only. Each resource use needs a fresh reader.
pub struct NativeReplacementMediaOrigin {
    started: Arc<NativeReplacementStarted>,
}
pub struct NativeReplacementMediaRead {
    source: NativeReplacementPreparationRead,
    target: PeerConnectionProjection,
    pub(super) interval: Arc<AuthorityGuardSet>,
}
impl NativeReplacementMediaRead {
    pub fn current(&self) -> bool {
        self.source.current()
    }
    pub fn source(&self) -> &NativeReplacementPreparationRead {
        &self.source
    }
    pub fn target(&self) -> &PeerConnectionProjection {
        &self.target
    }
}
redacted!(
    NativeReplacementStartRead,
    NativeReplacementStartAttempt,
    NativeReplacementStartUncertain,
    NativeReplacementStarted,
    NativeReplacementMediaOrigin,
    NativeReplacementMediaRead
);
impl NativeReplacementStartAttempt {
    pub fn uncertain(self) -> NativeReplacementStartUncertain {
        NativeReplacementStartUncertain { attempt: self }
    }
    pub fn expected_resources(&self) -> &serde_json::Value {
        &self.expected
    }
    pub fn started(self, observation: serde_json::Value) -> Result<NativeReplacementStarted> {
        if observation != self.expected {
            return Err(conflict());
        }
        Ok(NativeReplacementStarted {
            observation: crate::provisioning::canonical_json(&observation)?,
            attempt: self,
        })
    }
}
fn allocated_current(
    store: &Store,
    c: &Connection,
    allocated: &NativeReplacementAllocated,
) -> Result<serde_json::Value> {
    let a = &allocated.attempt;
    let prepared = &a.prepared;
    let saved = current_prepared(store, c, prepared)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.phase='allocate' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json=?4 AND e.observation_json=?5)",params![prepared.record,a.attempt,a.effect,allocated.observation,a.target],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(saved)
}
fn same_target(
    allocated: &NativeReplacementAllocated,
    descriptor: &serde_json::Value,
) -> Result<()> {
    let original: serde_json::Value =
        serde_json::from_str(&allocated.attempt.target).map_err(|_| Error::storage())?;
    let mut normalized = descriptor.clone();
    normalized["intent"]["offer"]["replacement_preparation"]["profile"] =
        serde_json::json!("connect-replacement-preparation-v1");
    if normalized != original {
        return Err(conflict());
    }
    Ok(())
}
fn projection(
    read: &NativeReplacementPreparationRead,
    member: connect_protocol::membership::MemberView,
    saved: &serde_json::Value,
    descriptor: &serde_json::Value,
) -> Result<PeerConnectionProjection> {
    let ctx = read.context();
    let original = read.source();
    let mut session = original.session().clone();
    session.actor = member;
    Ok(PeerConnectionProjection {
        session,
        connection: connect_protocol::connection_control::PreparedConnection {
            connection_id: saved["candidate_connection_id"]
                .as_str()
                .ok_or_else(invalid)?
                .into(),
            conversation_id: ctx.conversation_id.clone(),
            session_id: ctx.session_id.clone(),
            participant_id: ctx.participant_id.clone(),
            membership_epoch: ctx.membership_epoch,
            endpoint_id: ctx.target_endpoint_id.clone(),
            generation: saved["generation"].as_u64().ok_or_else(invalid)?,
            revision: 0,
            state: "preparing".into(),
        },
        runtime_connection_id: saved["candidate_runtime_id"]
            .as_str()
            .ok_or_else(invalid)?
            .into(),
        device_id: ctx.target_device_id.clone(),
        endpoint_address: ctx.target_address.clone(),
        endpoint_transport: ctx.target_transport.clone(),
        endpoint_capabilities: ctx
            .target_capabilities
            .iter()
            .filter(|v| v.as_str() == "audio")
            .cloned()
            .collect(),
        accepted_capabilities: original.accepted_capabilities().clone(),
        presence_role: ctx.role.clone(),
        offer: descriptor["intent"]["offer"].clone(),
        private_consultation: None,
        handoff_preparation: false,
        publication_suspended: true,
    })
}
impl Store {
    /// No effect. Safe to retry transient contention before a once-only start.
    pub fn native_replacement_start_read_with_peer(
        &self,
        allocated: &Arc<NativeReplacementAllocated>,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeReplacementStartRead> {
        let (member, descriptor, saved) = {
            let c = self.try_connection()?;
            let saved = allocated_current(self, &c, allocated)?;
            let (member, descriptor) = target_facts_for_profile(
                self,
                &c,
                &allocated.attempt.prepared,
                credential,
                proof,
                &saved,
                "connect-replacement-media-v1",
            )?;
            same_target(allocated, &descriptor)?;
            (member, descriptor, saved)
        };
        let source = allocated
            .attempt
            .prepared
            .selection
            .read_with_peer(self, proof)?;
        let target = projection(&source, member, &saved, &descriptor)?;
        Ok(NativeReplacementStartRead {
            source,
            target,
            descriptor,
        })
    }
    pub fn begin_native_replacement_start_with_read(
        &self,
        allocated: Arc<NativeReplacementAllocated>,
        credential: &str,
        proof: &PeerAuthorityGuard,
        read: NativeReplacementStartRead,
    ) -> Result<(NativeReplacementStartAttempt, NativeReplacementStartRead)> {
        let prepared = &allocated.attempt.prepared;
        if !read.current() || read.source.selection().origin.record != prepared.record {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let saved = allocated_current(self, &tx, &allocated)?;
        let (member, descriptor) = target_facts_for_profile(
            self,
            &tx,
            prepared,
            credential,
            proof,
            &saved,
            "connect-replacement-media-v1",
        )?;
        same_target(&allocated, &descriptor)?;
        if descriptor != read.descriptor
            || projection(&read.source, member, &saved, &descriptor)?.connection
                != read.target.connection
        {
            return Err(conflict());
        }
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='start')",[&prepared.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement media start was already attempted",
            ));
        }
        let attempt = format!("nrs_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        let target = crate::provisioning::canonical_json(&descriptor)?;
        let streams = descriptor["intent"]["offer"]["streams_offered"]
            .as_array()
            .ok_or_else(invalid)?
            .iter()
            .map(|v| v["id"].clone())
            .collect::<Vec<_>>();
        let expected = serde_json::json!({"candidate":saved["candidate_runtime_id"],"generation":saved["generation"],"stream_revision":2,"streams":streams,"resources_prepared":true,"prepared_muted":true,"media_ready":false});
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-start',?1,'none','prepared',?3)",params![effect,prepared.record,target]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'start',?2,?3)",params![prepared.record,attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='start'",[&prepared.record]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &prepared.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok((
            NativeReplacementStartAttempt {
                allocated,
                request: proof.request.clone(),
                attempt,
                effect,
                target,
                expected,
            },
            read,
        ))
    }
    pub fn record_native_replacement_started_with_peer(
        &self,
        captured: &NativeReplacementStarted,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        self.record_replacement_start(
            &captured.attempt,
            Some(&captured.observation),
            Some(Self::peer_scopes(proof)),
        )
    }
    pub fn record_native_replacement_started(
        &self,
        captured: &NativeReplacementStarted,
    ) -> Result<OperationRecord> {
        self.record_replacement_start(&captured.attempt, Some(&captured.observation), None)
    }
    pub fn native_replacement_start_uncertain_with_peer(
        &self,
        uncertain: &NativeReplacementStartUncertain,
        proof: &PeerAuthorityGuard,
    ) -> Result<()> {
        self.record_replacement_start(&uncertain.attempt, None, Some(Self::peer_scopes(proof)))
            .map(|_| ())
    }
    pub fn native_replacement_start_uncertain(
        &self,
        uncertain: &NativeReplacementStartUncertain,
    ) -> Result<()> {
        self.record_replacement_start(&uncertain.attempt, None, None)
            .map(|_| ())
    }
    fn record_replacement_start(
        &self,
        attempt: &NativeReplacementStartAttempt,
        observation: Option<&str>,
        interval: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<OperationRecord> {
        let prepared = &attempt.allocated.attempt.prepared;
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
        let old:Option<Option<String>>=tx.query_row("SELECT s.observation_json FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='start' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND e.observation_json=?7",params![prepared.record,prepared.runtime,prepared.reservation,prepared.hash,attempt.attempt,attempt.effect,attempt.target],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
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
            tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='start'",params![prepared.record,observation]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&attempt.effect]).map_err(|_|Error::storage())?;
        } else {
            if old.is_some() {
                return Err(conflict());
            }
            let recorded: bool = tx
                .query_row(
                    "SELECT state='unknown' FROM operation_effects WHERE id=?1",
                    [&attempt.effect],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if recorded {
                return Ok(row.record);
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
    pub fn native_replacement_media_origin(
        &self,
        started: Arc<NativeReplacementStarted>,
    ) -> Result<Arc<NativeReplacementMediaOrigin>> {
        let origin = Arc::new(NativeReplacementMediaOrigin { started });
        let c = self.try_connection()?;
        media_facts(self, &c, &origin)?;
        drop(c);
        Ok(origin)
    }
    pub fn try_native_replacement_media_read(
        &self,
        origin: &NativeReplacementMediaOrigin,
    ) -> Result<NativeReplacementMediaRead> {
        // All SQL/fence admissions are nonblocking, including under decoder
        // engine fences. Resolve twice with this one SQL permit and acquire the
        // complete authority set once; never re-enter SQL behind a queued reader.
        let c = self.try_connection()?;
        let (_, _, scopes, expires) = media_facts(self, &c, origin)?;
        let held = self.try_acquire_authority(&scopes)?;
        let (member, saved, scopes, current_expiry) = media_facts(self, &c, origin)?;
        if !self.authority_covers(&held, &scopes) {
            return Err(Error::forbidden());
        }
        let source = origin
            .started
            .attempt
            .allocated
            .attempt
            .prepared
            .selection
            .read_with_connection(self, &c, held.clone(), expires.min(current_expiry))?;
        let descriptor: serde_json::Value =
            serde_json::from_str(&origin.started.attempt.target).map_err(|_| Error::storage())?;
        let target = projection(&source, member, &saved, &descriptor)?;
        Ok(NativeReplacementMediaRead {
            source,
            target,
            interval: held,
        })
    }
}
pub(super) fn media_facts(
    store: &Store,
    c: &Connection,
    origin: &NativeReplacementMediaOrigin,
) -> Result<(
    connect_protocol::membership::MemberView,
    serde_json::Value,
    Vec<AuthorityScope>,
    i64,
)> {
    let start = &origin.started.attempt;
    let prepared = &start.allocated.attempt.prepared;
    let saved = allocated_current(store, c, &start.allocated)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.phase='start' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json=?4 AND e.observation_json=?5)",params![prepared.record,start.attempt,start.effect,origin.started.observation,start.target],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    let (meta, key, mut scopes, expiry) = crate::peer_authority::checked(store, c, &start.request)?;
    let descriptor: serde_json::Value =
        serde_json::from_str(&start.target).map_err(|_| Error::storage())?;
    if descriptor["credential_id"] != meta.actor.credential_id
        || descriptor["key_id"] != start.request.key_id
        || descriptor["public_key"] != serde_json::json!(key.public_key)
        || descriptor["device_id"] != key.device_id
    {
        return Err(Error::forbidden());
    }
    let context = prepared.selection.context(store, c)?;
    let member =
        crate::audience::member(c, &meta.actor.identity_id, &context.conversation_id, true)?;
    if member.participant_id != context.participant_id
        || member.membership_epoch != context.membership_epoch
    {
        return Err(Error::forbidden());
    }
    scopes.extend(prepared.selection.scopes(store, c, false)?);
    Ok((member, saved, AuthorityScope::merged(&scopes)?, expiry))
}

impl Store {
    pub fn replay_native_replacement_start_with_peer(
        &self,
        captured: &NativeReplacementStarted,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let a = &captured.attempt;
        let p = &a.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        actor_from_connection(&c, credential)?.require("connection.offer")?;
        let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let frozen: serde_json::Value =
            serde_json::from_str(&a.target).map_err(|_| Error::storage())?;
        if frozen["intent"] != serde_json::to_value(intent).map_err(|_| Error::storage())?
            || frozen["credential_id"] != credential
            || frozen["key_id"] != proof.key_id()
            || frozen["device_id"] != proof.key.device_id
            || frozen["public_key"] != serde_json::json!(proof.key.public_key)
        {
            return Err(conflict());
        }
        let row = rows::by_record(&c, &p.record)?;
        admission::require_row(self, Self::peer_scopes(proof), &row)?;
        let mut record = row.record;
        record.result = None;
        record.problem = None;
        record.effects.clear();
        Ok(record)
    }
    pub fn record_native_replacement_started_candidate_stopped(
        &self,
        captured: &NativeReplacementStarted,
        absent: &dyn super::super::super::native_offers::NativeOfferRuntimeObservation,
    ) -> Result<OperationRecord> {
        self.replacement_start_stopped(&captured.attempt, Some(&captured.observation), absent)
    }
    /// Only actual absence of this exact owned local native incarnation can
    /// settle uncertain resource preparation. No callback/provider IO is undone.
    pub fn record_native_replacement_uncertain_start_stopped(
        &self,
        uncertain: &NativeReplacementStartUncertain,
        absent: &dyn super::super::super::native_offers::NativeOfferRuntimeObservation,
    ) -> Result<OperationRecord> {
        self.replacement_start_stopped(&uncertain.attempt, None, absent)
    }
    fn replacement_start_stopped(
        &self,
        a: &NativeReplacementStartAttempt,
        observation: Option<&str>,
        absent: &dyn super::super::super::native_offers::NativeOfferRuntimeObservation,
    ) -> Result<OperationRecord> {
        let allocated = &a.allocated.attempt;
        let p = &allocated.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let super::super::super::native_offers::NativeOfferObservation::Retired {
            runtime_connection_id,
            generation,
        } = absent.observe()?
        else {
            return Err(Error::forbidden());
        };
        if serde_json::json!(runtime_connection_id) != a.expected["candidate"]
            || serde_json::json!(generation) != a.expected["generation"]
        {
            return Err(Error::forbidden());
        }
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                admission::row_scopes(&rows::by_record(&c, &p.record)?)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='start' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.observation_json IS ?7 AND e.observation_json=?8 AND s.started=1 AND ((?7 IS NOT NULL AND e.state='confirmed') OR (?7 IS NULL AND e.state='unknown'))) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1)",params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,observation,a.target],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement may have entered movement commit",
            ));
        }
        let cleanup = format!("nrc_{}", a.attempt);
        let already:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE id=?1 AND operation_record_id=?2 AND compensates_effect_id=?3 AND state='confirmed')",params![cleanup,p.record,a.effect],|r|r.get(0)).map_err(|_|Error::storage())?;
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
                format!("nrc_{}", p.attempt),
                &p.effect,
                "native-replacement-prepare",
            ),
            (
                format!("nrc_{}", allocated.attempt),
                &allocated.effect,
                "native-replacement-allocate",
            ),
            (cleanup, &a.effect, "native-replacement-start"),
        ] {
            tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id) VALUES(?1,?2,?3,?1,'none','confirmed',1,1,?4)",params![id,p.record,provider,effect]).map_err(|_|Error::storage())?;
        }
        let steps = {
            let mut q=tx.prepare("SELECT s.attempt_id,s.effect_id,s.phase FROM native_replacement_media_steps s WHERE s.operation_record_id=?1 AND s.started=1").map_err(|_|Error::storage())?;
            let values = q
                .query_map([&p.record], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            values
        };
        for (attempt, effect, phase) in steps {
            tx.execute(
                "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
                [&effect],
            )
            .map_err(|_| Error::storage())?;
            let id = format!("nrc_{}", attempt);
            tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id) VALUES(?1,?2,?3,?1,'none','confirmed',1,1,?4)",params![id,p.record,format!("native-replacement-{}",phase),effect]).map_err(|_|Error::storage())?;
        }
        tx.execute("UPDATE operations SET state=?2,problem_json=?3 WHERE record_id=?1",params![p.record,if row.record.cancel_requested{"cancelled"}else{"failed"},if row.record.cancel_requested{None}else{Some(serde_json::json!({"code":"native-target-stopped","source":"native-replacement"}).to_string())}]).map_err(|_|Error::storage())?;
        workers::invalidate(&tx, &p.record)?;
        native_handoffs::touch(&tx, &p.record, self)?;
        let result = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

/// Entire current native scene under one merged read, including the new target.
pub struct NativeReplacementSceneRead {
    pub(super) media: Arc<NativeReplacementMediaRead>,
    reads: std::collections::BTreeMap<(String, Option<String>), crate::NativeMediaRead>,
    audience_sources: std::collections::BTreeMap<String, audience::Origin>,
    telephone_pairs: std::collections::BTreeMap<String, (bool, bool)>,
}
impl NativeReplacementSceneRead {
    pub fn media(&self) -> &Arc<NativeReplacementMediaRead> {
        &self.media
    }
    pub fn current(&self) -> bool {
        self.media.current() && self.reads.values().all(crate::NativeMediaRead::current)
    }
    pub fn has_current_source(&self, id: &str) -> bool {
        self.current()
            && (self.existing(id, None).is_some()
                || (id == self.media.source.context().source_runtime_connection_id
                    && self.media.source.source().telephone().is_some()))
    }
    pub fn permits_receive(&self, target: &str, source: &str) -> bool {
        if !self.current() {
            return false;
        }
        let phone = &self.media.source.context().source_runtime_connection_id;
        if self.media.source.source().telephone().is_some() {
            if source == phone {
                return self.telephone_pairs.get(target).is_some_and(|v| v.0);
            }
            if target == phone {
                return self.telephone_pairs.get(source).is_some_and(|v| v.1);
            }
        }
        self.existing(target, Some(source))
            .is_some_and(|r| r.permits_subscriptions())
    }
    pub fn existing(&self, target: &str, source: Option<&str>) -> Option<&crate::NativeMediaRead> {
        self.reads.get(&(target.into(), source.map(str::to_owned)))
    }
}
impl Store {
    pub fn try_native_replacement_scene_read(
        &self,
        origin: &NativeReplacementMediaOrigin,
        sources: &[Arc<crate::NativeMediaOrigin>],
    ) -> Result<NativeReplacementSceneRead> {
        let prepared = &origin.started.attempt.allocated.attempt.prepared;
        // Every admission here is immediate. Keeping this one SQL permit avoids
        // repeatedly racing a queued status/watch read between planning and
        // validation. A contended authority fence refuses without waiting under
        // SQL; then the complete facts are resolved again under the merged grant.
        let c = self.try_connection()?;
        let context = prepared.selection.context(self, &c)?;
        if ![
            context.source_runtime_connection_id.as_str(),
            context.retained_runtime_connection_id.as_str(),
        ]
        .iter()
        .all(|id| {
            sources.iter().any(|s| s.runtime_connection_id() == *id)
                || (*id == context.source_runtime_connection_id
                    && matches!(
                        prepared.selection.source(),
                        crate::NativeReplacementSourceOrigin::Telephone(_)
                    ))
        }) {
            return Err(Error::forbidden());
        }
        let mut scopes = media_facts(self, &c, origin)?.2;
        scopes.extend(
            self.native_scene_facts(&c, sources, &context.conversation_id, &context.session_id)?
                .1,
        );
        let held = self.try_acquire_authority(&AuthorityScope::merged(&scopes)?)?;
        let (member, saved, mut required, expiry) = media_facts(self, &c, origin)?;
        let (mut facts, scene_scopes, scene_expiry) =
            self.native_scene_facts(&c, sources, &context.conversation_id, &context.session_id)?;
        required.extend(scene_scopes);
        if !self.authority_covers(&held, &required) {
            return Err(Error::forbidden());
        }
        let expiry = expiry.min(scene_expiry);
        let source = prepared
            .selection
            .read_with_connection(self, &c, held.clone(), expiry)?;
        let descriptor: serde_json::Value =
            serde_json::from_str(&origin.started.attempt.target).map_err(|_| Error::storage())?;
        let target = projection(&source, member, &saved, &descriptor)?;
        self.native_replacement_scene_pairs(
            &c,
            sources,
            &mut facts,
            &context.source_runtime_connection_id,
            source.source().telephone(),
            &target,
            origin
                .started
                .attempt
                .request
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?,
        )?;
        let mut audience_sources = audience::origins(&c, sources, &target, &prepared.record)?;
        let mut telephone_pairs = std::collections::BTreeMap::new();
        if let Some(phone) = source.source().telephone() {
            audience::telephone_origin(&c, phone, &mut audience_sources)?;
            for native in sources {
                let actor = self.native_media_route_actor(&c, native)?;
                let view = &facts
                    .get(&(native.runtime_connection_id().into(), None))
                    .ok_or_else(Error::forbidden)?
                    .0;
                let both = phone
                    .route_actor()
                    .permits_recipient(&view.session.actor.identity_id)
                    && actor.permits_recipient(&phone.member().identity_id);
                telephone_pairs.insert(
                    native.runtime_connection_id().into(),
                    (
                        both && actor.permits("stream.subscribe"),
                        both && phone.route_actor().permits("stream.subscribe"),
                    ),
                );
            }
            let actor = actor_from_connection(
                &c,
                origin
                    .started
                    .attempt
                    .request
                    .credentials
                    .last()
                    .ok_or_else(Error::forbidden)?,
            )?;
            let both = phone.route_actor().permits_recipient(&actor.identity_id)
                && actor.permits_recipient(&phone.member().identity_id);
            telephone_pairs.insert(
                target.runtime_connection_id.clone(),
                (
                    both && actor.permits("stream.subscribe"),
                    both && phone.route_actor().permits("stream.subscribe"),
                ),
            );
        }
        let reads = self.native_scene_reads(facts, held.clone(), expiry)?;
        Ok(NativeReplacementSceneRead {
            media: Arc::new(NativeReplacementMediaRead {
                source,
                target,
                interval: held,
            }),
            reads,
            audience_sources,
            telephone_pairs,
        })
    }
}
