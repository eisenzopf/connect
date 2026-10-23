//! Once-only movement custody. A captured commit is historical metadata, not a
//! packet grant or a completed public projection. The owner must retain it
//! before awaiting any metadata write and must never retry the physical commit.
use super::*;

pub struct NativeReplacementCommitAttempt {
    origin: Arc<NativeReplacementMediaOrigin>,
    attempt: String,
    effect: String,
    expected: String,
    target: PeerConnectionProjection,
    resources: String,
    source_expires_at: i64,
}
pub struct NativeReplacementCommitted {
    attempt: NativeReplacementCommitAttempt,
    observation: String,
    audience: Option<NativeReplacementAudienceCapture>,
}
pub struct NativeReplacementCommitUncertain {
    attempt: NativeReplacementCommitAttempt,
}
redacted!(
    NativeReplacementCommitAttempt,
    NativeReplacementCommitted,
    NativeReplacementCommitUncertain
);
impl NativeReplacementCommitAttempt {
    /// Exact managed participant observation expected from the sealed Core
    /// movement result. This cannot be supplied by a signed wire ready flag.
    pub fn expected(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.expected).map_err(|_| Error::storage())
    }
    pub fn committed(
        self,
        movement: serde_json::Value,
        ready: &crate::NativeReadyObservation,
    ) -> Result<NativeReplacementCommitted> {
        let target = &self.target;
        if movement != self.expected()?
            || ready.private_consultation.is_some()
            || ready.handoff_preparation
            || ready.conversation_id != target.connection.conversation_id
            || ready.session_id != target.connection.session_id
            || ready.runtime_connection_id != target.runtime_connection_id
            || ready.participant_id != target.connection.participant_id
            || ready.membership_epoch != target.connection.membership_epoch
            || ready.endpoint_id != target.connection.endpoint_id
            || ready.device_id != target.device_id
            || ready.generation != target.connection.generation
            || !(3..=9_007_199_254_740_991).contains(&ready.stream_revision)
            || Some(ready.participant_revision) != movement["participant_revision"].as_u64()
            || ready.streams.is_empty()
            || ready.streams.len() > 32
        {
            return Err(conflict());
        }
        let offered = target.offer["streams_offered"]
            .as_array()
            .ok_or_else(invalid)?;
        let mut unique = std::collections::BTreeSet::new();
        for stream in &ready.streams {
            id(stream)?;
            if !unique.insert(stream) || !offered.iter().any(|s| s["id"].as_str() == Some(stream)) {
                return Err(conflict());
            }
        }
        let observation = crate::provisioning::canonical_json(&serde_json::json!({
            "movement":movement,
            "readiness":{"streams":ready.streams,"stream_revision":ready.stream_revision,
                         "participant_revision":ready.participant_revision}
        }))?;
        Ok(NativeReplacementCommitted {
            attempt: self,
            observation,
            audience: None,
        })
    }
    pub fn uncertain(self) -> NativeReplacementCommitUncertain {
        NativeReplacementCommitUncertain { attempt: self }
    }
}
impl Store {
    /// Persist the single commit attempt before entering Core. The complete
    /// scene grant includes the independent operator, old/retained owners and
    /// target peer. Callers must retain it through the physical movement fence.
    pub fn begin_native_replacement_commit(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        read: &NativeReplacementSceneRead,
    ) -> Result<NativeReplacementCommitAttempt> {
        let p = &origin.started.attempt.allocated.attempt.prepared;
        if !read.current() || read.media.source.selection().origin.record != p.record {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')",
            [&p.record], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement movement was already attempted",
            ));
        }
        let (_, saved, scopes, _) = media_facts(self, &tx, &origin)?;
        let ctx = read.media.source.context();
        if !self.authority_covers(&read.media.interval, &scopes)
            || read.media.target.runtime_connection_id
                != saved["candidate_runtime_id"].as_str().ok_or_else(invalid)?
            || !read.has_current_source(&ctx.source_runtime_connection_id)
            || read
                .existing(&ctx.retained_runtime_connection_id, None)
                .is_none()
        {
            return Err(Error::forbidden());
        }
        let confirmed: u32 = tx.query_row(
            "SELECT count(*) FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.started=1 AND s.observation_json IS NOT NULL",
            [&p.record], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if confirmed != 3 {
            return Err(Error::new(
                "not-ready",
                "Actual private replacement readiness is not confirmed",
            ));
        }
        // Capture the target's own lifetime, without the old preparation
        // deadline. Its recipient scope must cover the established programme.
        let mut binding = origin.started.attempt.request.clone();
        binding.command = crate::PeerCommand::Binding;
        let (meta, _, target_scopes, source_expires_at) =
            crate::peer_authority::checked(self, &tx, &binding)?;
        if !self.authority_covers(&read.media.interval, &target_scopes) {
            return Err(Error::forbidden());
        }
        let mut intent: connect_protocol::connection_control::OfferIntent = serde_json::from_value(
            serde_json::from_str::<serde_json::Value>(&origin.started.attempt.target)
                .map_err(|_| Error::storage())?["intent"]
                .clone(),
        )
        .map_err(|_| Error::storage())?;
        intent
            .offer
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("replacement_preparation");
        let recipients = crate::connection_control::intent_resources(&tx, &meta.actor, &intent)?;
        if recipients.iter().any(|r| !meta.actor.permits_recipient(r)) {
            return Err(Error::forbidden());
        }
        let resources = crate::provisioning::canonical_json(&serde_json::json!(recipients))?;
        let revision = saved["expected_preparation"]["participant_revision"]
            .as_u64()
            .and_then(|v| v.checked_add(1))
            .filter(|v| *v <= 9_007_199_254_740_991)
            .ok_or_else(invalid)?;
        let expected = crate::provisioning::canonical_json(&serde_json::json!({
            "operation_id":ctx.operation_record_id,"session":ctx.session_id,
            "participant":ctx.participant_id,"membership_epoch":ctx.membership_epoch,
            "source":ctx.source_runtime_connection_id,"candidate":saved["candidate_runtime_id"],
            "retained":ctx.retained_runtime_connection_id,"endpoint":ctx.target_endpoint_id,
            "device":ctx.target_device_id,"generation":saved["generation"],
            "participant_revision":revision,"role":ctx.role,"confirmation":saved["confirmation"]
        }))?;
        let attempt = format!("nmc_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',?3)",params![effect,p.record,expected]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'commit',?2,?3)",params![p.record,attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='commit'",[&p.record]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(NativeReplacementCommitAttempt {
            origin,
            attempt,
            effect,
            expected,
            target: read.media.target.clone(),
            resources,
            source_expires_at,
        })
    }
    /// Persist a known actual outcome after all transient engine grants have
    /// been dropped. This does not reauthorize or reexecute movement.
    pub fn record_native_replacement_committed(
        &self,
        captured: &NativeReplacementCommitted,
    ) -> Result<OperationRecord> {
        self.record_replacement_commit(
            &captured.attempt,
            Some(&captured.observation),
            captured.audience.as_ref(),
        )
    }
    pub fn native_replacement_commit_uncertain(
        &self,
        captured: &NativeReplacementCommitUncertain,
    ) -> Result<OperationRecord> {
        self.record_replacement_commit(&captured.attempt, None, None)
    }
    fn record_replacement_commit(
        &self,
        a: &NativeReplacementCommitAttempt,
        observation: Option<&str>,
        audience: Option<&NativeReplacementAudienceCapture>,
    ) -> Result<OperationRecord> {
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &p.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                if audience.is_some() {
                    scopes.extend(crate::authority_admission::conversation_scopes(
                        &c,
                        &row.record.conversation_id.clone().ok_or_else(invalid)?,
                        crate::AuthorityMode::Write,
                    )?);
                }
                AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let old:Option<(Option<String>,String)>=tx.query_row("SELECT s.observation_json,e.state FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND e.observation_json=?7",params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,a.expected],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (old, state) = old.ok_or_else(Error::forbidden)?;
        if let Some(old) = old {
            if Some(old.as_str()) == observation {
                if let Some(audience) = audience {
                    audience.require_recorded(&tx, &a.effect)?;
                }
                return Ok(row.record);
            }
            return Err(conflict());
        }
        if observation.is_none() && state == "unknown" {
            return Ok(row.record);
        }
        if let Some(observation) = observation {
            tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",params![p.record,observation]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&a.effect]).map_err(|_|Error::storage())?;
        } else {
            tx.execute(
                "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
                [&a.effect],
            )
            .map_err(|_| Error::storage())?;
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1 AND state='preparing'",
                [&p.record],
            )
            .map_err(|_| Error::storage())?;
            workers::invalidate(&tx, &p.record)?;
        }
        if let Some(audience) = audience {
            audience.record(&tx, self, &a.effect)?;
        }
        native_handoffs::touch(&tx, &p.record, self)?;
        let record = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}

impl Store {
    /// Project a captured physical commit atomically. Retries write metadata
    /// only, even after the old route's cleanup, cancellation or revocation.
    pub fn project_native_replacement_committed(
        &self,
        captured: &NativeReplacementCommitted,
    ) -> Result<Arc<crate::NativeMediaOrigin>> {
        let a = &captured.attempt;
        let start = &a.origin.started.attempt;
        let p = &start.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &p.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                scopes.extend(crate::authority_admission::conversation_scopes(
                    &c,
                    &a.target.connection.conversation_id,
                    crate::AuthorityMode::Write,
                )?);
                AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7 AND e.observation_json=?8)",params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,captured.observation,a.expected],|r|r.get(0)).map_err(|_| Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let descriptor: serde_json::Value =
            serde_json::from_str(&start.target).map_err(|_| Error::storage())?;
        let intent: connect_protocol::connection_control::OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
        let public_key = serde_json::from_value(descriptor["public_key"].clone())
            .map_err(|_| Error::storage())?;
        if row.record.state == OperationState::Completed {
            return self.replacement_established_origin(
                &tx,
                &p.record,
                start.request.clone(),
                intent,
                public_key,
            );
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let saved: serde_json::Value = tx
            .query_row(
                "SELECT plan_json FROM native_replacements WHERE operation_record_id=?1",
                [&p.record],
                |r| r.get::<_, String>(0),
            )
            .map_err(|_| Error::storage())
            .and_then(|v| serde_json::from_str(&v).map_err(|_| Error::storage()))?;
        let source = row.record.connection_id.as_ref().ok_or_else(invalid)?;
        let target = &a.target;
        let ready: serde_json::Value =
            serde_json::from_str(&captured.observation).map_err(|_| Error::storage())?;
        let mut offer = intent.offer.clone();
        offer
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("replacement_preparation");
        let recipients: Vec<String> =
            serde_json::from_str(&a.resources).map_err(|_| Error::storage())?;
        if !crate::conversations::capacity(&tx, 1 + 2 * recipients.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Replacement lifecycle journal capacity exhausted",
            ));
        }
        tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,replaces,state,revision,observed_runtime_id,observed_at,readiness_json) SELECT ?1,?2,?3,?4,?5,tenant_id,?6,?7,id,'ready',1,?8,?9,?10 FROM connections WHERE id=?11 AND generation BETWEEN ?12 AND ?12+1",params![target.connection.connection_id,target.connection.session_id,target.connection.participant_id,target.connection.membership_epoch,target.session.actor.identity_id,target.connection.endpoint_id,target.connection.generation,p.runtime,crate::now(),ready["readiness"].to_string(),source,saved["source_generation"].as_u64().ok_or_else(invalid)?]).map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO connection_offers(connection_id,runtime_connection_id,operation_id,credential_id,offer_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![target.connection.connection_id,target.runtime_connection_id,intent.operation_id,descriptor["credential_id"].as_str().ok_or_else(invalid)?,offer.to_string(),crate::now()]).map_err(|_| Error::storage())?;
        let old_retired: bool = tx
            .query_row(
                "SELECT state='retired' FROM connections WHERE id=?1",
                [source],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        // Never touch the retained peer. Old cleanup may already have recorded
        // this exact retirement while the captured result awaited projection.
        tx.execute(
            "UPDATE native_connection_lifetimes SET retired=1 WHERE connection_id=?1",
            [source],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1 AND generation=?2 AND state!='retired'",params![source,saved["source_generation"].as_u64().ok_or_else(invalid)?]).map_err(|_| Error::storage())?;
        // The physical effect already happened under its original revision
        // fence. Later metadata mutations cannot erase it or cause replay.
        let current_revision: u64 = tx
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id=?1",
                [&target.connection.session_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let topology_revision = current_revision
            .checked_add(1)
            .filter(|v| *v < 9_007_199_254_740_991)
            .ok_or_else(conflict)?;
        let changed=tx.execute("UPDATE sessions SET topology_revision=?2,updated_at=?3 WHERE id=?1 AND topology_revision=?4",params![target.connection.session_id,topology_revision,crate::now(),current_revision]).map_err(|_|Error::storage())?;
        if changed != 1 {
            return Err(conflict());
        }
        if row.record.state == OperationState::Preparing {
            tx.execute(
                "UPDATE operations SET state='committing' WHERE record_id=?1",
                [&p.record],
            )
            .map_err(|_| Error::storage())?;
        }
        let result = serde_json::json!({"connection_id":target.connection.connection_id,"replaces":source,"generation":target.connection.generation,"participant_id":target.connection.participant_id,"membership_epoch":target.connection.membership_epoch,"endpoint_id":target.connection.endpoint_id,"device_id":target.device_id,"readiness":ready["readiness"],"session_revision":topology_revision});
        tx.execute(
            "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
            params![p.record, result.to_string()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO native_replacement_lifetimes(operation_record_id,credential_id,key_id,device_id,endpoint_id,participant_id,membership_epoch,generation,resources_json,source_expires_at,attempt_runtime,attempt_id,effect_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![p.record,descriptor["credential_id"].as_str().ok_or_else(invalid)?,descriptor["key_id"].as_str().ok_or_else(invalid)?,target.device_id,target.connection.endpoint_id,target.connection.participant_id,target.connection.membership_epoch,target.connection.generation,a.resources,a.source_expires_at,p.runtime,a.attempt,a.effect]).map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![p.record,target.connection.connection_id,p.runtime,target.connection.generation]).map_err(|_| Error::storage())?;
        workers::invalidate(&tx, &p.record)?;
        native_handoffs::touch(&tx, &p.record, self)?;
        for connection in [source.as_str(), target.connection.connection_id.as_str()] {
            if connection == source && old_retired {
                continue;
            }
            let value=tx.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[connection],|r|Ok(connect_protocol::events::ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?})).map_err(|_| Error::storage())?;
            for recipient in &recipients {
                let mut data = serde_json::json!({"source_kind":"session","source_id":value.session_id,"session_id":value.session_id,"connection_id":value.connection_id,"participant_id":value.participant_id,"identity_id":target.session.actor.identity_id,"connection":value});
                if recipient == &row.owner {
                    data["operation_id"] = serde_json::json!(row.record.operation_id);
                }
                tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,row.record.conversation_id,if connection==source{"connection_retired"}else{"connection_ready"},p.record,value.revision,data.to_string(),serde_json::json!({"kind":"connect-native-runtime","correlation_id":value.connection_id}).to_string()]).map_err(|_| Error::storage())?;
            }
        }
        let origin = self.replacement_established_origin(
            &tx,
            &p.record,
            start.request.clone(),
            intent,
            public_key,
        )?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(origin)
    }
}

impl NativeReplacementCommitted {
    pub fn with_audience_revisions(
        mut self,
        audience: NativeReplacementAudienceCapture,
    ) -> Result<Self> {
        let p = &self
            .attempt
            .origin
            .started
            .attempt
            .allocated
            .attempt
            .prepared;
        audience.require(&p.record, &p.runtime, "commit")?;
        let data: serde_json::Value =
            serde_json::from_str(&self.observation).map_err(|_| Error::storage())?;
        if audience.after(&self.attempt.target.runtime_connection_id)
            != data["readiness"]["stream_revision"].as_u64()
        {
            return Err(conflict());
        }
        self.audience = Some(audience);
        Ok(self)
    }
}
