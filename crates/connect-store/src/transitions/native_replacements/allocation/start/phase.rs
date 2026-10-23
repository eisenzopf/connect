//! Once-only local scene attachment, activation and actual readiness observations.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReplacementMediaPhase {
    Attach,
    Activate,
    Readiness,
}
impl NativeReplacementMediaPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Activate => "activate",
            Self::Readiness => "readiness",
        }
    }
    fn previous(self) -> Option<&'static str> {
        match self {
            Self::Attach => None,
            Self::Activate => Some("attach"),
            Self::Readiness => Some("activate"),
        }
    }
}
pub struct NativeReplacementMediaAttempt {
    origin: Arc<NativeReplacementMediaOrigin>,
    phase: NativeReplacementMediaPhase,
    attempt: String,
    effect: String,
    expected: String,
}
pub struct NativeReplacementMediaCapture {
    attempt: NativeReplacementMediaAttempt,
    observation: Option<String>,
    audience: Option<NativeReplacementAudienceCapture>,
}
redacted!(
    NativeReplacementMediaAttempt,
    NativeReplacementMediaCapture,
    NativeReplacementSceneRead
);
impl NativeReplacementMediaAttempt {
    pub fn expected(&self) -> Result<serde_json::Value> {
        serde_json::from_str(&self.expected).map_err(|_| Error::storage())
    }
    pub fn applied(self, observation: serde_json::Value) -> Result<NativeReplacementMediaCapture> {
        if observation != self.expected()? {
            return Err(conflict());
        }
        Ok(NativeReplacementMediaCapture {
            observation: Some(self.expected.clone()),
            audience: None,
            attempt: self,
        })
    }
    pub fn uncertain(self) -> NativeReplacementMediaCapture {
        NativeReplacementMediaCapture {
            attempt: self,
            observation: None,
            audience: None,
        }
    }
}
impl Store {
    pub fn begin_native_replacement_media_phase(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        phase: NativeReplacementMediaPhase,
        read: &NativeReplacementMediaRead,
    ) -> Result<NativeReplacementMediaAttempt> {
        let p = &origin.started.attempt.allocated.attempt.prepared;
        if !read.current() || read.source.selection().origin.record != p.record {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (_, saved, scopes, _) = media_facts(self, &tx, &origin)?;
        if !self.authority_covers(&read.interval, &scopes)
            || read.target.runtime_connection_id
                != saved["candidate_runtime_id"].as_str().ok_or_else(invalid)?
        {
            return Err(Error::forbidden());
        }
        let committing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1)",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if committing {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement may have entered movement commit",
            ));
        }
        if let Some(previous) = phase.previous() {
            let confirmed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.observation_json IS NOT NULL AND s.started=1)",params![p.record,previous],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !confirmed {
                return Err(Error::new(
                    "not-ready",
                    "Previous replacement media phase is not confirmed",
                ));
            }
        }
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase=?2)",params![p.record,phase.name()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement media phase was already attempted",
            ));
        }
        let expected = crate::provisioning::canonical_json(
            &serde_json::json!({"candidate":saved["candidate_runtime_id"],"generation":saved["generation"],"phase":phase.name(),"attached":true,"private":true,"activated_muted":phase!=NativeReplacementMediaPhase::Attach,"media_ready":phase==NativeReplacementMediaPhase::Readiness}),
        )?;
        let attempt = format!("nrm_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,?3,?1,'none','prepared',?4)",params![effect,p.record,format!("native-replacement-{}",phase.name()),expected]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_media_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,?2,?3,?4)",params![p.record,phase.name(),attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_replacement_media_steps SET started=1 WHERE operation_record_id=?1 AND phase=?2",params![p.record,phase.name()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(NativeReplacementMediaAttempt {
            origin,
            phase,
            attempt,
            effect,
            expected,
        })
    }
    pub fn record_native_replacement_media_phase(
        &self,
        captured: &NativeReplacementMediaCapture,
    ) -> Result<OperationRecord> {
        let a = &captured.attempt;
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let row = rows::by_record(&c, &p.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                if captured.audience.is_some() {
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
        let old:Option<(Option<String>,String)>=tx.query_row("SELECT s.observation_json,e.state FROM native_replacements n JOIN native_replacement_media_steps s ON s.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.phase=?5 AND s.attempt_id=?6 AND s.effect_id=?7 AND s.started=1 AND e.observation_json=?8",params![p.record,p.runtime,p.reservation,p.hash,a.phase.name(),a.attempt,a.effect,a.expected],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (old, state) = old.ok_or_else(Error::forbidden)?;
        if old.is_some() {
            if old == captured.observation {
                if let Some(audience) = &captured.audience {
                    audience.require_recorded(&tx, &a.effect)?;
                }
                return Ok(row.record);
            }
            return Err(conflict());
        }
        if captured.observation.is_none() && state == "unknown" {
            return Ok(row.record);
        }
        if let Some(observation) = &captured.observation {
            tx.execute("UPDATE native_replacement_media_steps SET observation_json=?3 WHERE operation_record_id=?1 AND phase=?2",params![p.record,a.phase.name(),observation]).map_err(|_|Error::storage())?;
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
        if let Some(audience) = &captured.audience {
            audience.record(&tx, self, &a.effect)?;
        }
        native_handoffs::touch(&tx, &p.record, self)?;
        let result = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

impl NativeReplacementMediaCapture {
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
        if self.attempt.phase != NativeReplacementMediaPhase::Activate || self.observation.is_none()
        {
            return Err(Error::forbidden());
        }
        audience.require(&p.record, &p.runtime, "activate")?;
        self.audience = Some(audience);
        Ok(self)
    }
}
