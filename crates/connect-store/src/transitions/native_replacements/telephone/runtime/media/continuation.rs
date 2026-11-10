//! One complete current scene plus historical once-only private phase receipts.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TelephoneReplacementPhase {
    Attach,
    Activate,
    Readiness,
}
impl TelephoneReplacementPhase {
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
pub struct TelephoneReplacementSceneRead {
    pub(super) media: Arc<TelephoneReplacementMediaRead>,
    pub(super) resources: Arc<TelephoneReplacementResourcesCapture>,
    pub(super) audio: TelephoneAudioResources,
    existing: BTreeMap<(String, Option<String>), crate::NativeMediaRead>,
    pairs: BTreeSet<(String, String)>,
    revisions: BTreeMap<String, (String, u64, u64)>,
    pub(super) sources: Vec<Arc<crate::NativeMediaOrigin>>,
}
impl TelephoneReplacementSceneRead {
    pub fn expires_at(&self) -> i64 {
        self.media.expires_at()
    }
    pub fn current(&self) -> bool {
        self.media.current() && self.existing.values().all(crate::NativeMediaRead::current)
    }
    pub fn media(&self) -> &Arc<TelephoneReplacementMediaRead> {
        &self.media
    }
    pub fn audio(&self) -> &TelephoneAudioResources {
        &self.audio
    }
    pub fn existing(&self, target: &str, source: Option<&str>) -> Option<&crate::NativeMediaRead> {
        self.existing
            .get(&(target.into(), source.map(str::to_owned)))
    }
    pub fn permits_pair(&self, target: &str, source: &str) -> bool {
        self.current() && self.pairs.contains(&(target.into(), source.into()))
    }
    pub fn capture_activation_revisions(
        &self,
        observed: BTreeMap<String, u64>,
    ) -> Result<TelephoneReplacementAudienceCapture> {
        self.capture_revisions(observed, "activate")
    }
    pub fn capture_commit_revisions(
        &self,
        mut observed: BTreeMap<String, u64>,
    ) -> Result<TelephoneReplacementAudienceCapture> {
        observed.remove(&self.media.source().context().source_runtime_connection_id);
        self.capture_revisions(observed, "commit")
    }
    fn capture_revisions(
        &self,
        observed: BTreeMap<String, u64>,
        phase: &'static str,
    ) -> Result<TelephoneReplacementAudienceCapture> {
        let mut revisions = self.revisions.clone();
        if phase == "commit" {
            revisions.remove(&self.media.source().context().source_runtime_connection_id);
        }
        if observed.len() != revisions.len() {
            return Err(conflict());
        }
        let mut values = BTreeMap::new();
        for (runtime, (connection, generation, before)) in &revisions {
            let after = *observed.get(runtime).ok_or_else(conflict)?;
            if before.checked_add(1) != Some(after) || after > 9_007_199_254_740_991 {
                return Err(conflict());
            }
            values.insert(
                runtime.clone(),
                (connection.clone(), *generation, *before, after),
            );
        }
        let p = &self.resources.attempt.origin.submitted.attempt.prepared;
        Ok(TelephoneReplacementAudienceCapture {
            record: p.record.clone(),
            runtime: p.runtime.clone(),
            phase,
            values,
        })
    }
}
pub struct TelephoneReplacementAudienceCapture {
    pub(super) record: String,
    pub(super) runtime: String,
    pub(super) phase: &'static str,
    pub(super) values: BTreeMap<String, (String, u64, u64, u64)>,
}
pub struct TelephoneReplacementPhaseAttempt {
    resources: Arc<TelephoneReplacementResourcesCapture>,
    phase: TelephoneReplacementPhase,
    attempt: String,
    effect: String,
    expected: String,
    sources: Vec<Arc<crate::NativeMediaOrigin>>,
    expires_at: i64,
}
pub struct TelephoneReplacementPhaseCapture {
    attempt: Arc<TelephoneReplacementPhaseAttempt>,
    observation: Option<String>,
    audience: Option<TelephoneReplacementAudienceCapture>,
}
redacted!(
    TelephoneReplacementSceneRead,
    TelephoneReplacementAudienceCapture,
    TelephoneReplacementPhaseAttempt,
    TelephoneReplacementPhaseCapture
);
impl TelephoneReplacementPhaseAttempt {
    pub fn applied(
        self: &Arc<Self>,
        observation: serde_json::Value,
    ) -> Result<TelephoneReplacementPhaseCapture> {
        if crate::provisioning::canonical_json(&observation)? != self.expected {
            return Err(conflict());
        }
        Ok(TelephoneReplacementPhaseCapture {
            attempt: self.clone(),
            observation: Some(self.expected.clone()),
            audience: None,
        })
    }
    pub fn uncertain(self: &Arc<Self>) -> TelephoneReplacementPhaseCapture {
        TelephoneReplacementPhaseCapture {
            attempt: self.clone(),
            observation: None,
            audience: None,
        }
    }
}
impl TelephoneReplacementPhaseCapture {
    pub fn with_audience(mut self, audience: TelephoneReplacementAudienceCapture) -> Result<Self> {
        let p = &self
            .attempt
            .resources
            .attempt
            .origin
            .submitted
            .attempt
            .prepared;
        if self.attempt.phase != TelephoneReplacementPhase::Activate
            || self.observation.is_none()
            || audience.phase != "activate"
            || audience.record != p.record
            || audience.runtime != p.runtime
        {
            return Err(conflict());
        }
        self.audience = Some(audience);
        Ok(self)
    }
}
pub(super) fn resources_current(
    c: &Connection,
    capture: &TelephoneReplacementResourcesCapture,
) -> Result<TelephoneAudioResources> {
    let a = &capture.attempt;
    let p = &a.origin.submitted.attempt.prepared;
    let raw = capture.observation.as_ref().ok_or_else(Error::forbidden)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_resources r JOIN operation_effects e ON e.id=r.effect_id WHERE r.operation_record_id=?1 AND r.runtime_epoch=?2 AND r.attempt_id=?3 AND r.effect_id=?4 AND r.descriptor_json=?5 AND r.observation_json=?6 AND e.state='confirmed' AND e.observation_json=?5)",params![p.record,p.runtime,a.attempt,a.effect,a.descriptor,raw],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    serde_json::from_str(raw).map_err(|_| Error::storage())
}
impl Store {
    pub fn try_telephone_replacement_private_read(
        &self,
        capture: Arc<TelephoneReplacementResourcesCapture>,
    ) -> Result<TelephoneReplacementMediaRead> {
        let c = self.try_connection()?;
        resources_current(&c, &capture)?;
        let p = &capture.attempt.origin.submitted.attempt.prepared;
        let activated: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_phases p JOIN operation_effects e ON e.id=p.effect_id WHERE p.operation_record_id=?1 AND p.runtime_epoch=?2 AND p.phase='activate' AND p.observation_json IS NOT NULL AND p.audience_json IS NOT NULL AND e.state='confirmed')",params![p.record,p.runtime],|r| r.get(0)).map_err(|_|Error::storage())?;
        if !activated {
            return Err(Error::forbidden());
        }
        let scopes = p.selection.scopes(self, &c, false)?;
        let held = self.try_acquire_authority(&scopes)?;
        self.telephone_media_read_in(&c, capture.attempt.origin.clone(), held, crate::now() + 5)
    }
    pub fn try_telephone_replacement_resources_read(
        &self,
        capture: Arc<TelephoneReplacementResourcesCapture>,
    ) -> Result<TelephoneReplacementMediaRead> {
        let c = self.try_connection()?;
        resources_current(&c, &capture)?;
        let origin = capture.attempt.origin.clone();
        let p = &origin.submitted.attempt.prepared;
        let scopes = p.selection.scopes(self, &c, false)?;
        let held = self.try_acquire_authority(&scopes)?;
        let read = self.telephone_media_read_in(&c, origin, held, crate::now() + 5)?;
        resources_current(&c, &capture)?;
        Ok(read)
    }
    pub(super) fn telephone_replacement_phase_scopes(
        &self,
        c: &Connection,
        capture: &TelephoneReplacementResourcesCapture,
        sources: &[Arc<crate::NativeMediaOrigin>],
        write: bool,
    ) -> Result<Vec<crate::AuthorityScope>> {
        resources_current(c, capture)?;
        let p = &capture.attempt.origin.submitted.attempt.prepared;
        let ctx = p.selection.context(self, c)?;
        if ![
            &ctx.source_runtime_connection_id,
            &ctx.retained_runtime_connection_id,
        ]
        .iter()
        .all(|id| {
            sources
                .iter()
                .any(|s| s.runtime_connection_id() == id.as_str())
        }) {
            return Err(Error::forbidden());
        }
        let mut scopes = p.selection.scopes(self, c, write)?;
        scopes.extend(
            self.native_scene_facts(c, sources, &ctx.conversation_id, &ctx.session_id)?
                .1,
        );
        crate::AuthorityScope::merged(&scopes)
    }
    pub fn try_telephone_replacement_scene_read(
        &self,
        capture: Arc<TelephoneReplacementResourcesCapture>,
        sources: &[Arc<crate::NativeMediaOrigin>],
    ) -> Result<TelephoneReplacementSceneRead> {
        let c = self.try_connection()?;
        let scopes = self.telephone_replacement_phase_scopes(&c, &capture, sources, false)?;
        let held = self.try_acquire_authority(&scopes)?;
        self.telephone_scene_read_in(&c, capture, sources, held, crate::now() + 5)
    }
    pub(super) fn telephone_scene_read_in(
        &self,
        c: &Connection,
        capture: Arc<TelephoneReplacementResourcesCapture>,
        sources: &[Arc<crate::NativeMediaOrigin>],
        held: Arc<crate::AuthorityGuardSet>,
        until: i64,
    ) -> Result<TelephoneReplacementSceneRead> {
        let origin = capture.attempt.origin.clone();
        let p = &origin.submitted.attempt.prepared;
        let ctx = p.selection.context(self, c)?;
        if ![
            &ctx.source_runtime_connection_id,
            &ctx.retained_runtime_connection_id,
        ]
        .iter()
        .all(|id| {
            sources
                .iter()
                .any(|s| s.runtime_connection_id() == id.as_str())
        }) {
            return Err(Error::forbidden());
        }

        let (facts, scene_scopes, expiry) =
            self.native_scene_facts(c, sources, &ctx.conversation_id, &ctx.session_id)?;
        let mut required = p.selection.scopes(self, c, false)?;
        required.extend(scene_scopes);
        if !self.authority_covers(&held, &crate::AuthorityScope::merged(&required)?) {
            return Err(Error::forbidden());
        }
        let media = Arc::new(self.telephone_media_read_in(
            c,
            origin.clone(),
            held.clone(),
            expiry.min(until),
        )?);
        let audio = resources_current(c, &capture)?;
        let mut pairs = BTreeSet::new();
        let old = &ctx.source_runtime_connection_id;
        let candidate = media.candidate()?.to_owned();
        for other in sources {
            let target = other.runtime_connection_id();
            if target == old {
                continue;
            }
            let (view, _, other_may) = facts
                .get(&(target.into(), None))
                .ok_or_else(Error::forbidden)?;
            // Existing exact old/native pairs already enforce both native
            // credential recipient ceilings for this same logical identity.
            // The independent SIP account must additionally cover this peer.
            if !media
                .route_actor()
                .permits_recipient(&view.session.actor.identity_id)
            {
                continue;
            }
            if media.route_actor().permits("stream.subscribe")
                && facts.contains_key(&(old.clone(), Some(target.into())))
            {
                pairs.insert((candidate.clone(), target.into()));
            }
            if *other_may && facts.contains_key(&(target.into(), Some(old.clone()))) {
                pairs.insert((target.into(), candidate.clone()));
            }
        }
        let mut revisions = BTreeMap::new();
        for source in sources {
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![source.connection_id(),source.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
            revisions.insert(
                source.runtime_connection_id().into(),
                (source.connection_id().into(), source.generation(), revision),
            );
        }
        let activated:Option<String>=c.query_row("SELECT audience_json FROM telephone_replacement_phases WHERE operation_record_id=?1 AND phase='activate' AND observation_json IS NOT NULL",[&p.record],|r|r.get(0)).optional().map_err(|_|Error::storage())?.flatten();
        let revision = if let Some(raw) = activated {
            serde_json::from_str::<serde_json::Value>(&raw).map_err(|_| Error::storage())?
                [&candidate][3]
                .as_u64()
                .ok_or_else(Error::storage)?
        } else {
            audio.stream_revision
        };
        revisions.insert(
            candidate,
            (
                media.plan["candidate_connection_id"]
                    .as_str()
                    .ok_or_else(invalid)?
                    .into(),
                audio.generation,
                revision,
            ),
        );
        let existing = self.native_scene_reads(facts, held, media.expires_at())?;
        Ok(TelephoneReplacementSceneRead {
            media,
            resources: capture,
            audio,
            existing,
            pairs,
            revisions,
            sources: sources.to_vec(),
        })
    }
    pub fn begin_telephone_replacement_phase(
        &self,
        read: TelephoneReplacementSceneRead,
        phase: TelephoneReplacementPhase,
    ) -> Result<Arc<TelephoneReplacementPhaseAttempt>> {
        let until = read.expires_at();
        self.begin_telephone_replacement_phase_before(read, phase, until)
    }
    pub fn begin_telephone_replacement_phase_before(
        &self,
        read: TelephoneReplacementSceneRead,
        phase: TelephoneReplacementPhase,
        original_until: i64,
    ) -> Result<Arc<TelephoneReplacementPhaseAttempt>> {
        if !read.current() || !self.authority_covers(&read.media.interval, &[]) {
            return Err(Error::forbidden());
        }
        // Routing hints only: copy this original complete interval's IDs, then
        // drop every Read before fresh exclusive admission. Full current scene
        // facts are rederived and covered inside the transaction below.
        let mutation_scopes = read.media.interval.mutation_scopes();
        let resources = read.resources.clone();
        let sources = read.sources.clone();
        let revisions = read.revisions.clone();
        let pairs = read.pairs.clone();
        let original_descriptor = read.media.read.descriptor.clone();
        let until = read.expires_at().min(original_until);
        if crate::now() >= until {
            return Err(Error::forbidden());
        }
        drop(read);
        let p = &resources.attempt.origin.submitted.attempt.prepared;
        // Do not spend a five-ms file-drain budget repeatedly deriving the
        // O(N²) scene before admission. The copied IDs authorize nothing; a
        // new/moved dependency fails current coverage before any DML.
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(5));
        let held = self.acquire_authority(&mutation_scopes, &wait)?;
        let mut c = self.try_connection()?;
        c.bind_available_mutation_authority(self, &held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let read =
            self.telephone_scene_read_in(&tx, resources.clone(), &sources, held.clone(), until)?;
        if read.revisions != revisions
            || read.pairs != pairs
            || read.media.read.descriptor != original_descriptor
        {
            return Err(conflict());
        }
        resources_current(&tx, &resources)?;
        if !self.authority_covers(&read.media.interval, &p.selection.scopes(self, &tx, false)?) {
            return Err(Error::forbidden());
        }

        let (_, _, descriptor) = facts(
            self,
            &tx,
            p,
            read.media.source(),
            &resources.attempt.origin.submitted.attempt.route_credential,
            &resources.attempt.origin.submitted.attempt.policy,
        )?;
        if descriptor != read.media.read.descriptor {
            return Err(conflict());
        }
        if let Some(previous) = phase.previous() {
            let confirmed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_phases s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.observation_json IS NOT NULL)",params![p.record,previous],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !confirmed {
                return Err(Error::new(
                    "not-ready",
                    "Previous telephone phase is not confirmed",
                ));
            }
        }
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_phases WHERE operation_record_id=?1 AND phase=?2)",params![p.record,phase.name()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone phase already attempted",
            ));
        }
        let committing:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1)",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if committing {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone movement may have started",
            ));
        }
        let expected = crate::provisioning::canonical_json(
            &serde_json::json!({"candidate":read.audio.candidate,"generation":read.audio.generation,"phase":phase.name(),"attached":true,"private":true,"activated_muted":phase!=TelephoneReplacementPhase::Attach,"media_ready":phase==TelephoneReplacementPhase::Readiness}),
        )?;
        let attempt = format!("tpc_{}", uuid::Uuid::new_v4());
        let effect = format!("tpe_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,?3,?1,'none','prepared',?4)",params![effect,p.record,format!("telephone-replacement-{}",phase.name()),expected]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO telephone_replacement_phases(operation_record_id,phase,runtime_epoch,attempt_id,effect_id) VALUES(?1,?2,?3,?4,?5)",params![p.record,phase.name(),p.runtime,attempt,effect]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        let expires_at = read.expires_at();
        drop(read);
        drop(c);
        drop(held);
        Ok(Arc::new(TelephoneReplacementPhaseAttempt {
            resources: resources.clone(),
            phase,
            attempt,
            effect,
            expected,
            sources,
            expires_at,
        }))
    }
    pub fn try_telephone_replacement_phase_attempt_read(
        &self,
        attempt: Arc<TelephoneReplacementPhaseAttempt>,
    ) -> Result<TelephoneReplacementSceneRead> {
        if crate::now() >= attempt.expires_at {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        let p = &attempt.resources.attempt.origin.submitted.attempt.prepared;
        let scopes = self.telephone_replacement_phase_scopes(
            &c,
            &attempt.resources,
            &attempt.sources,
            false,
        )?;
        let held = self.try_acquire_authority(&scopes)?;
        let read = self.telephone_scene_read_in(
            &c,
            attempt.resources.clone(),
            &attempt.sources,
            held,
            attempt.expires_at,
        )?;
        let exact: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_phases s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.runtime_epoch=?3 AND s.attempt_id=?4 AND s.effect_id=?5 AND s.observation_json IS NULL AND e.state='submitting' AND e.observation_json=?6 AND e.attempt=1 AND e.submission_generation=1)", params![p.record,attempt.phase.name(),p.runtime,attempt.attempt,attempt.effect,attempt.expected], |r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
    pub fn record_telephone_replacement_phase(
        &self,
        capture: &TelephoneReplacementPhaseCapture,
    ) -> Result<OperationRecord> {
        let a = &capture.attempt;
        let p = &a.resources.attempt.origin.submitted.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let (held, mut c) = self.telephone_state_change_sql(|c| {
            let row = rows::by_record(c, &p.record)?;
            let mut scopes = admission::mutation_row_scopes(&row)?;
            if capture.audience.is_some() {
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &row.record.conversation_id.ok_or_else(invalid)?,
                    crate::AuthorityMode::Write,
                )?);
            }
            crate::AuthorityScope::merged(&scopes)
        })?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let old:Option<(Option<String>,Option<String>,String)>=tx.query_row("SELECT s.observation_json,s.audience_json,e.state FROM telephone_replacement_phases s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.runtime_epoch=?3 AND s.attempt_id=?4 AND s.effect_id=?5 AND e.observation_json=?6",params![p.record,a.phase.name(),p.runtime,a.attempt,a.effect,a.expected],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        let (old, audience, state) = old.ok_or_else(Error::forbidden)?;
        let captured_audience = capture
            .audience
            .as_ref()
            .map(|v| crate::provisioning::canonical_json(&serde_json::json!(v.values)))
            .transpose()?;
        if let Some(value) = &capture.observation {
            if let Some(old) = old {
                if old != *value || state != "confirmed" || audience != captured_audience {
                    return Err(conflict());
                }
                return Ok(row.record);
            }
            if !matches!(state.as_str(), "submitting" | "unknown")
                || ((a.phase == TelephoneReplacementPhase::Activate) != capture.audience.is_some())
            {
                return Err(conflict());
            }
            tx.execute("UPDATE telephone_replacement_phases SET observation_json=?3,audience_json=?4 WHERE operation_record_id=?1 AND phase=?2",params![p.record,a.phase.name(),value,captured_audience]).map_err(|_|Error::storage())?;
            tx.execute(
                "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
                [&a.effect],
            )
            .map_err(|_| Error::storage())?;
            if let Some(audience) = &capture.audience {
                for (connection, generation, before, after) in audience.values.values() {
                    tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0)",params![connection,generation,p.runtime,after,before]).map_err(|_|Error::storage())?;
                }
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
