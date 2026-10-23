//! Once-only telephone movement custody. Captures grant no future IO.
use super::continuation::{
    resources_current, TelephoneReplacementAudienceCapture, TelephoneReplacementSceneRead,
};
use super::*;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelephoneConnectionSnapshot {
    pub connection_id: String,
    pub runtime_connection_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub identity_id: String,
    pub endpoint_id: String,
    pub device_id: String,
    pub generation: u64,
    pub source_connection_id: String,
    pub source_generation: u64,
    pub route_credential_id: String,
    pub route_owner_id: String,
    pub route_reference: String,
    pub destination: String,
    pub endpoint_address: String,
    pub role: String,
    pub media: TelephoneAudioResources,
    pub expected_movement: serde_json::Value,
    pub recipients: Vec<String>,
}
pub struct TelephoneReplacementCommitAttempt {
    pub(super) resources: Arc<TelephoneReplacementResourcesCapture>,
    pub(super) attempt: String,
    pub(super) effect: String,
    pub(super) descriptor: String,
    pub(super) snapshot: TelephoneConnectionSnapshot,
}
pub struct TelephoneReplacementCommitted {
    pub(super) attempt: Arc<TelephoneReplacementCommitAttempt>,
    pub(super) observation: String,
    pub(super) audience: TelephoneReplacementAudienceCapture,
}
pub struct TelephoneReplacementCommitUncertain {
    attempt: Arc<TelephoneReplacementCommitAttempt>,
}
redacted!(
    TelephoneConnectionSnapshot,
    TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitted,
    TelephoneReplacementCommitUncertain
);
impl TelephoneReplacementCommitAttempt {
    pub fn snapshot(&self) -> &TelephoneConnectionSnapshot {
        &self.snapshot
    }

    pub fn committed(
        self: &Arc<Self>,
        movement: serde_json::Value,
        stream: &str,
        stream_revision: u64,
        audience: TelephoneReplacementAudienceCapture,
    ) -> Result<TelephoneReplacementCommitted> {
        let p = &self.resources.attempt.origin.submitted.attempt.prepared;
        if movement != self.snapshot.expected_movement
            || stream != self.snapshot.media.stream_id
            || audience.record != p.record
            || audience.runtime != p.runtime
            || audience.phase != "commit"
            || audience
                .values
                .get(&self.snapshot.runtime_connection_id)
                .map(|v| v.3)
                != Some(stream_revision)
        {
            return Err(conflict());
        }
        let observation = crate::provisioning::canonical_json(
            &serde_json::json!({"movement":movement,"readiness":{"streams":[stream],"stream_revision":stream_revision,"participant_revision":movement["participant_revision"]}}),
        )?;
        Ok(TelephoneReplacementCommitted {
            attempt: self.clone(),
            observation,
            audience,
        })
    }
    pub fn uncertain(self: &Arc<Self>) -> TelephoneReplacementCommitUncertain {
        TelephoneReplacementCommitUncertain {
            attempt: self.clone(),
        }
    }
}
impl Store {
    pub fn begin_telephone_replacement_commit(
        &self,
        read: &TelephoneReplacementSceneRead,
    ) -> Result<Arc<TelephoneReplacementCommitAttempt>> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let resources = &read.resources;
        let a = &resources.attempt.origin.submitted.attempt;
        let p = &a.prepared;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        resources_current(&tx, resources)?;
        let ctx = read.media.source().context();
        if !self.authority_covers(&read.media.interval, &p.selection.scopes(self, &tx, false)?)
            || read
                .existing(&ctx.source_runtime_connection_id, None)
                .is_none()
            || read
                .existing(&ctx.retained_runtime_connection_id, None)
                .is_none()
        {
            return Err(Error::forbidden());
        }
        let (_, _, descriptor) = facts(
            self,
            &tx,
            p,
            read.media.source(),
            &a.route_credential,
            &a.policy,
        )?;
        if descriptor != read.media.read.descriptor {
            return Err(conflict());
        }
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone movement was already attempted",
            ));
        }
        let confirmed:u32=tx.query_row("SELECT count(*) FROM telephone_replacement_phases s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.runtime_epoch=?2 AND s.observation_json IS NOT NULL",params![p.record,p.runtime],|r|r.get(0)).map_err(|_|Error::storage())?;
        if confirmed != 3 {
            return Err(Error::new(
                "not-ready",
                "Actual telephone confirmation is not recorded",
            ));
        }
        let plan = allocation::current_prepared(self, &tx, p)?;
        let revision = plan["expected_preparation"]["participant_revision"]
            .as_u64()
            .and_then(|v| v.checked_add(1))
            .filter(|v| *v <= 9_007_199_254_740_991)
            .ok_or_else(invalid)?;
        let movement = serde_json::json!({"operation_id":ctx.operation_record_id,"session":ctx.session_id,"participant":ctx.participant_id,"membership_epoch":ctx.membership_epoch,"source":ctx.source_runtime_connection_id,"candidate":plan["candidate_runtime_id"],"retained":ctx.retained_runtime_connection_id,"endpoint":ctx.target_endpoint_id,"device":ctx.target_device_id,"generation":plan["generation"],"participant_revision":revision,"role":ctx.role,"confirmation":plan["confirmation"]});
        let snapshot = TelephoneConnectionSnapshot {
            connection_id: plan["candidate_connection_id"]
                .as_str()
                .ok_or_else(invalid)?
                .into(),
            runtime_connection_id: read.audio.candidate.clone(),
            conversation_id: ctx.conversation_id.clone(),
            session_id: ctx.session_id.clone(),
            participant_id: ctx.participant_id.clone(),
            membership_epoch: ctx.membership_epoch,
            identity_id: ctx.identity_id.clone(),
            endpoint_id: ctx.target_endpoint_id.clone(),
            device_id: ctx.target_device_id.clone(),
            generation: read.audio.generation,
            source_connection_id: p
                .selection
                .origin
                .request()
                .connection_id
                .clone()
                .ok_or_else(invalid)?,
            source_generation: plan["source_generation"].as_u64().ok_or_else(invalid)?,
            route_credential_id: a.route_credential.clone(),
            route_owner_id: read.media.route_actor().identity_id.clone(),
            route_reference: a.policy.route_reference.clone(),
            destination: read.media.destination()?,
            endpoint_address: ctx.target_address.clone(),
            role: ctx.role.clone(),
            media: read.audio.clone(),
            expected_movement: movement,
            recipients: crate::audience::source_members(
                &tx,
                &ctx.conversation_id,
                connect_protocol::membership::ContentKind::Session,
                &ctx.session_id,
            )?,
        };
        let descriptor = crate::provisioning::canonical_json(
            &serde_json::to_value(&snapshot).map_err(|_| Error::storage())?,
        )?;
        let expected = crate::provisioning::canonical_json(&snapshot.expected_movement)?;
        let attempt = format!("tcm_{}", uuid::Uuid::new_v4());
        let effect = format!("tre_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'native-replacement-commit',?1,'none','prepared',?3)",params![effect,p.record,expected]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,'commit',?2,?3)",params![p.record,attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_replacement_steps SET started=1 WHERE operation_record_id=?1 AND phase='commit'",[&p.record]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO telephone_replacement_commits(operation_record_id,runtime_epoch,attempt_id,effect_id,descriptor_json) VALUES(?1,?2,?3,?4,?5)",params![p.record,p.runtime,attempt,effect,descriptor]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Arc::new(TelephoneReplacementCommitAttempt {
            resources: resources.clone(),
            attempt,
            effect,
            descriptor,
            snapshot,
        }))
    }
    pub fn record_telephone_replacement_committed(
        &self,
        capture: &TelephoneReplacementCommitted,
    ) -> Result<OperationRecord> {
        self.record_telephone_commit(
            &capture.attempt,
            Some((&capture.observation, &capture.audience)),
        )
    }
    pub fn record_telephone_replacement_commit_uncertain(
        &self,
        capture: &TelephoneReplacementCommitUncertain,
    ) -> Result<OperationRecord> {
        self.record_telephone_commit(&capture.attempt, None)
    }
    fn record_telephone_commit(
        &self,
        a: &TelephoneReplacementCommitAttempt,
        observation: Option<(&str, &TelephoneReplacementAudienceCapture)>,
    ) -> Result<OperationRecord> {
        let p = &a.resources.attempt.origin.submitted.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &p.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                if observation.is_some() {
                    scopes.extend(crate::authority_admission::conversation_scopes(
                        &c,
                        &a.snapshot.conversation_id,
                        crate::AuthorityMode::Write,
                    )?);
                }
                crate::AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let saved:Option<(Option<String>,Option<String>,String)>=tx.query_row("SELECT t.observation_json,t.audience_json,e.state FROM telephone_replacement_commits t JOIN native_replacement_steps s ON s.operation_record_id=t.operation_record_id AND s.phase='commit' AND s.attempt_id=t.attempt_id AND s.effect_id=t.effect_id AND s.started=1 JOIN operation_effects e ON e.id=t.effect_id WHERE t.operation_record_id=?1 AND t.runtime_epoch=?2 AND t.attempt_id=?3 AND t.effect_id=?4 AND t.descriptor_json=?5",params![p.record,p.runtime,a.attempt,a.effect,a.descriptor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let (old, audience, state) = saved.ok_or_else(Error::forbidden)?;
        if let Some((value, capture)) = observation {
            let raw = crate::provisioning::canonical_json(&serde_json::json!(capture.values))?;
            if let Some(old) = old {
                if old == value && audience.as_deref() == Some(&raw) && state == "confirmed" {
                    return Ok(row.record);
                }
                return Err(conflict());
            }
            if !matches!(state.as_str(), "submitting" | "unknown") {
                return Err(conflict());
            }
            tx.execute("UPDATE telephone_replacement_commits SET observation_json=?2,audience_json=?3 WHERE operation_record_id=?1",params![p.record,value,raw]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",params![p.record,value]).map_err(|_|Error::storage())?;
            tx.execute(
                "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
                [&a.effect],
            )
            .map_err(|_| Error::storage())?;
            for (connection, generation, before, after) in capture.values.values() {
                tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0)",params![connection,generation,p.runtime,after,before]).map_err(|_|Error::storage())?;
            }
        } else {
            if old.is_some() {
                return Err(conflict());
            }
            if state == "unknown" {
                return Ok(row.record);
            }
            if state != "submitting" {
                return Err(conflict());
            }
            tx.execute(
                "UPDATE operation_effects SET state='unknown' WHERE id=?1",
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
        native_handoffs::touch(&tx, &p.record, self)?;
        let record = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}
