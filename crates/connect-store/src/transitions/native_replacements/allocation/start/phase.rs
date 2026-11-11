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
    candidate_only: bool,
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
        self.begin_native_replacement_media_phase_inner(origin, phase, read, None, false)
    }
    /// Complete scene custody may reserve only gain-only Attach/Readiness
    /// metadata. A naked packet read remains insufficient for a durable phase;
    /// general Activate and every uncertain/general audience projection retain Write.
    pub fn begin_native_replacement_scene_media_phase(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        phase: NativeReplacementMediaPhase,
        scene: &NativeReplacementSceneRead,
    ) -> Result<NativeReplacementMediaAttempt> {
        self.begin_native_replacement_scene_phase(origin, phase, scene, false)
    }
    /// Only the specialized Core muted activation may consume this attempt.
    /// Its complete current scene pins the exact unchanged outside rules.
    pub fn begin_native_replacement_candidate_activation(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        scene: &NativeReplacementSceneRead,
    ) -> Result<NativeReplacementMediaAttempt> {
        self.begin_native_replacement_scene_phase(
            origin,
            NativeReplacementMediaPhase::Activate,
            scene,
            true,
        )
    }
    fn begin_native_replacement_scene_phase(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        phase: NativeReplacementMediaPhase,
        scene: &NativeReplacementSceneRead,
        candidate_only: bool,
    ) -> Result<NativeReplacementMediaAttempt> {
        if !scene.current() {
            return Err(Error::forbidden());
        }
        for source in scene.reads.values() {
            if !scene
                .media
                .interval
                .covers_original_interval(source.interval())
            {
                return Err(Error::forbidden());
            }
        }
        self.begin_native_replacement_media_phase_inner(
            origin,
            phase,
            scene.media(),
            Some(scene),
            candidate_only,
        )
    }
    fn begin_native_replacement_media_phase_inner(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        phase: NativeReplacementMediaPhase,
        read: &NativeReplacementMediaRead,
        scene: Option<&NativeReplacementSceneRead>,
        candidate_only: bool,
    ) -> Result<NativeReplacementMediaAttempt> {
        if candidate_only && phase != NativeReplacementMediaPhase::Activate {
            return Err(Error::forbidden());
        }
        let positive = scene.is_some()
            && (candidate_only
                || matches!(
                    phase,
                    NativeReplacementMediaPhase::Attach | NativeReplacementMediaPhase::Readiness
                ));
        let current = || scene.map_or_else(|| read.current(), NativeReplacementSceneRead::current);
        let p = &origin.started.attempt.allocated.attempt.prepared;
        if !current() || read.source.selection().origin.record != p.record {
            return Err(Error::forbidden());
        }
        if !positive && !self.authority_covers(&read.interval, &read.interval.mutation_scopes()) {
            return Err(Error::forbidden());
        }
        let mut c = self.connection_with_authority(&read.interval)?;
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
        let mut observation = serde_json::json!({"candidate":saved["candidate_runtime_id"],"generation":saved["generation"],"phase":phase.name(),"attached":true,"private":true,"activated_muted":phase!=NativeReplacementMediaPhase::Attach,"media_ready":phase==NativeReplacementMediaPhase::Readiness});
        if candidate_only {
            observation["candidate_only"] = serde_json::json!(true);
        }
        let expected = crate::provisioning::canonical_json(&observation)?;
        let attempt = format!("nrm_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        if !current() {
            return Err(Error::forbidden());
        }
        let permit = if candidate_only {
            Some(
                self.authority_writer_boundary
                    .native_candidate_activation_ledger(
                        self,
                        &tx,
                        &read.interval,
                        ledger_key(
                            p,
                            phase,
                            &attempt,
                            &effect,
                            &expected,
                            crate::authority_writer_boundary::phase_ledger::Stage::Begin,
                        ),
                        None,
                    )?,
            )
        } else if positive {
            Some(self.authority_writer_boundary.native_phase_ledger(
                self,
                &tx,
                &read.interval,
                ledger_key(
                    p,
                    phase,
                    &attempt,
                    &effect,
                    &expected,
                    crate::authority_writer_boundary::phase_ledger::Stage::Begin,
                ),
            )?)
        } else {
            None
        };
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,?3,?1,'none','prepared',?4)",params![effect,p.record,format!("native-replacement-{}",phase.name()),expected]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO native_replacement_media_steps(operation_record_id,phase,attempt_id,effect_id) VALUES(?1,?2,?3,?4)",params![p.record,phase.name(),attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE native_replacement_media_steps SET started=1 WHERE operation_record_id=?1 AND phase=?2",params![p.record,phase.name()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !current() {
            return Err(Error::forbidden());
        }
        drop(permit);
        tx.commit().map_err(|_| Error::storage())?;
        Ok(NativeReplacementMediaAttempt {
            origin,
            phase,
            candidate_only,
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
        let (held, mut c) = self.try_replacement_capture_projection(None, |c| {
            let row = rows::by_record(c, &p.record)?;
            let mut scopes = if positive_capture(c, captured)? {
                admission::row_scopes(&row)?
            } else {
                admission::mutation_row_scopes(&row)?
            };
            if captured
                .audience
                .as_ref()
                .is_some_and(|audience| !audience.is_candidate_only())
            {
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &row.record.conversation_id.clone().ok_or_else(invalid)?,
                    crate::AuthorityMode::Write,
                )?);
            }
            AuthorityScope::merged(&scopes)
        })?;
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
        // Revalidate eligibility under the same original SQL transaction. A
        // state change to Unknown/cancel/audience cannot upgrade this reader.
        let positive = positive_capture(&tx, captured)?;
        let required = if positive {
            admission::row_scopes(&row)?
        } else {
            admission::mutation_row_scopes(&row)?
        };
        if !self.authority_covers(&held, &required) {
            return Err(Error::new(
                "authority-busy",
                "Original phase capture now requires a writer",
            ));
        }
        let permit = if positive && a.candidate_only {
            Some(
                self.authority_writer_boundary
                    .native_candidate_activation_ledger(
                        self,
                        &tx,
                        &held,
                        ledger_key(
                            p,
                            a.phase,
                            &a.attempt,
                            &a.effect,
                            &a.expected,
                            crate::authority_writer_boundary::phase_ledger::Stage::Confirm,
                        ),
                        captured.audience.as_ref(),
                    )?,
            )
        } else if positive {
            Some(self.authority_writer_boundary.native_phase_ledger(
                self,
                &tx,
                &held,
                ledger_key(
                    p,
                    a.phase,
                    &a.attempt,
                    &a.effect,
                    &a.expected,
                    crate::authority_writer_boundary::phase_ledger::Stage::Confirm,
                ),
            )?)
        } else {
            None
        };
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
        drop(permit);
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
        if self.attempt.candidate_only != audience.is_candidate_only() {
            return Err(Error::forbidden());
        }
        self.audience = Some(audience);
        Ok(self)
    }
}

fn ledger_key(
    p: &NativeReplacementPrepared,
    phase: NativeReplacementMediaPhase,
    attempt: &str,
    effect: &str,
    expected: &str,
    stage: crate::authority_writer_boundary::phase_ledger::Stage,
) -> crate::authority_writer_boundary::phase_ledger::Key {
    crate::authority_writer_boundary::phase_ledger::Key {
        record: p.record.clone(),
        phase: phase.name(),
        attempt: attempt.into(),
        effect: effect.into(),
        expected: expected.into(),
        runtime: p.runtime.clone(),
        reservation: p.reservation.clone(),
        digest: p.hash.clone(),
        stage,
    }
}
fn positive_capture(c: &Connection, captured: &NativeReplacementMediaCapture) -> Result<bool> {
    let a = &captured.attempt;
    let eligible = if a.candidate_only {
        a.phase == NativeReplacementMediaPhase::Activate
            && captured
                .audience
                .as_ref()
                .is_some_and(NativeReplacementAudienceCapture::is_candidate_only)
    } else {
        matches!(
            a.phase,
            NativeReplacementMediaPhase::Attach | NativeReplacementMediaPhase::Readiness
        ) && captured.audience.is_none()
    };
    if !eligible || captured.observation.as_deref() != Some(a.expected.as_str()) {
        return Ok(false);
    }
    let p = &a.origin.started.attempt.allocated.attempt.prepared;
    let current:Option<(Option<String>,String,bool)>=c.query_row(
        "SELECT s.observation_json,e.state,(o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND NOT EXISTS(SELECT 1 FROM native_replacement_steps commit_step WHERE commit_step.operation_record_id=o.record_id AND commit_step.phase='commit' AND commit_step.started=1)) FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_media_steps s ON s.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.phase=?5 AND s.attempt_id=?6 AND s.effect_id=?7 AND s.started=1 AND e.provider='native-replacement-'||s.phase AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.observation_json=?8 AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL",
        params![p.record,p.runtime,p.reservation,p.hash,a.phase.name(),a.attempt,a.effect,a.expected],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    Ok(current.is_some_and(|(old, state, preparing)| {
        (old.as_deref() == Some(a.expected.as_str()) && state == "confirmed")
            || (old.is_none() && state == "submitting" && preparing)
    }))
}
