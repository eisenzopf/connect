//! No-grant telephone preparation and read-only same-started-attempt admission.
use super::*;
pub struct TelephoneReplacementCommitPreparation {
    pub(super) resources: Arc<TelephoneReplacementResourcesCapture>,
    pub(super) sources: Vec<Arc<crate::NativeMediaOrigin>>,
    pub(super) attempt: String,
    pub(super) effect: String,
    pub(super) original_until: i64,
    pub(super) original_scene: String,
    descriptor: String,
    snapshot: TelephoneConnectionSnapshot,
}
redacted!(TelephoneReplacementCommitPreparation);
fn scene_stamp(read: &TelephoneReplacementSceneRead) -> Result<String> {
    crate::provisioning::canonical_json(&serde_json::json!(read
        .sources
        .iter()
        .map(|s| (s.connection_id(), s.runtime_connection_id(), s.generation()))
        .collect::<Vec<_>>()))
}
impl TelephoneReplacementCommitPreparation {
    pub fn attempt_binding(&self) -> (&str, &str, &str) {
        (
            &self
                .resources
                .attempt
                .origin
                .submitted
                .attempt
                .prepared
                .record,
            &self.attempt,
            &self.effect,
        )
    }

    pub fn snapshot(&self) -> &TelephoneConnectionSnapshot {
        &self.snapshot
    }
    pub fn expires_at(&self) -> i64 {
        self.original_until
    }
    pub(super) fn new(
        read: &TelephoneReplacementSceneRead,
        snapshot: TelephoneConnectionSnapshot,
        descriptor: String,
    ) -> Result<Self> {
        Ok(Self {
            resources: read.resources.clone(),
            sources: read.sources.clone(),
            attempt: format!("tcm_{}", uuid::Uuid::new_v4()),
            effect: format!("tre_{}", uuid::Uuid::new_v4()),
            original_until: read.expires_at(),
            original_scene: scene_stamp(read)?,
            descriptor,
            snapshot,
        })
    }
    pub(super) fn require_read(&self, read: &TelephoneReplacementSceneRead) -> Result<()> {
        if !read.current()
            || crate::now() >= self.original_until
            || !Arc::ptr_eq(&self.resources, &read.resources)
            || self.sources.len() != read.sources.len()
            || !self
                .sources
                .iter()
                .zip(&read.sources)
                .all(|(a, b)| Arc::ptr_eq(a, b))
            || self.original_scene != scene_stamp(read)?
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(super) fn require_snapshot(
        &self,
        read: &TelephoneReplacementSceneRead,
        descriptor: &str,
    ) -> Result<()> {
        self.require_read(read)?;
        if self.descriptor != descriptor {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl Store {
    pub fn prepare_telephone_replacement_commit(
        &self,
        read: &TelephoneReplacementSceneRead,
    ) -> Result<TelephoneReplacementCommitPreparation> {
        let c = self.try_connection()?;
        let (snapshot, descriptor, _) = self.telephone_commit_preparation_facts(&c, read, None)?;
        TelephoneReplacementCommitPreparation::new(read, snapshot, descriptor)
    }
    pub fn revalidate_prepared_telephone_replacement_commit(
        &self,
        preparation: &TelephoneReplacementCommitPreparation,
        read: &TelephoneReplacementSceneRead,
    ) -> Result<()> {
        let c = self.try_connection()?;
        let (_, descriptor, _) = self.telephone_commit_preparation_facts(&c, read, None)?;
        preparation.require_snapshot(read, &descriptor)
    }
    pub(super) fn telephone_commit_preparation_facts(
        &self,
        c: &Connection,
        read: &TelephoneReplacementSceneRead,
        purpose: Option<&TelephoneReplacementCommitAttempt>,
    ) -> Result<(TelephoneConnectionSnapshot, String, String)> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let resources = &read.resources;
        let a = &resources.attempt.origin.submitted.attempt;
        let p = &a.prepared;
        let ctx = read.media.source().context();
        if !self.authority_covers(&read.media.interval, &p.selection.scopes(self, c, false)?)
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
            c,
            p,
            read.media.source(),
            &a.route_credential,
            &a.policy,
        )?;
        if descriptor != read.media.read.descriptor {
            return Err(conflict());
        }
        if let Some(attempt) = purpose {
            self.require_unpublished_telephone_commit_attempt(c, attempt)?;
            if !Arc::ptr_eq(&read.resources, &attempt.resources) {
                return Err(Error::forbidden());
            }
        }
        let exists:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists && purpose.is_none() {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone movement was already attempted",
            ));
        }
        let confirmed:u32=c.query_row("SELECT count(*) FROM telephone_replacement_phases s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=?1 AND s.runtime_epoch=?2 AND s.observation_json IS NOT NULL",params![p.record,p.runtime],|r|r.get(0)).map_err(|_|Error::storage())?;
        if confirmed != 3 {
            return Err(Error::new(
                "not-ready",
                "Actual telephone confirmation is not recorded",
            ));
        }
        let plan = allocation::current_prepared(self, c, p)?;
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
                c,
                &ctx.conversation_id,
                connect_protocol::membership::ContentKind::Session,
                &ctx.session_id,
            )?,
        };
        let descriptor = crate::provisioning::canonical_json(
            &serde_json::to_value(&snapshot).map_err(|_| Error::storage())?,
        )?;
        let expected = crate::provisioning::canonical_json(&snapshot.expected_movement)?;
        Ok((snapshot, descriptor, expected))
    }
    fn require_unpublished_telephone_commit_attempt(
        &self,
        c: &Connection,
        attempt: &TelephoneReplacementCommitAttempt,
    ) -> Result<()> {
        let p = &attempt.resources.attempt.origin.submitted.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let expected = crate::provisioning::canonical_json(&attempt.snapshot.expected_movement)?;
        let exact: bool=c.query_row(
            "SELECT EXISTS(SELECT 1 FROM telephone_replacement_commits t JOIN native_replacements n ON n.operation_record_id=t.operation_record_id JOIN operations o ON o.record_id=t.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=t.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=t.operation_record_id WHERE t.operation_record_id=?1 AND t.runtime_epoch=?2 AND t.attempt_id=?3 AND t.effect_id=?4 AND t.descriptor_json=?5 AND n.runtime_epoch=?2 AND n.reservation_id=?6 AND n.plan_digest=?7 AND s.attempt_id=t.attempt_id AND s.effect_id=t.effect_id AND s.started=1 AND s.observation_json IS NULL AND e.provider='native-replacement-commit' AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.state='submitting' AND e.provider_ref IS NULL AND e.attempt=1 AND e.submission_generation=1 AND e.observation_json=?8 AND o.state='preparing' AND o.cancel_requested=0 AND o.worker_runtime=?9 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND o.expires_at>?10 AND n.expires_at>?10)",
            params![p.record,p.runtime,attempt.attempt,attempt.effect,attempt.descriptor,p.reservation,p.hash,expected,self.transition_runtime,crate::now()],|row|row.get(0)
        ).map_err(|_|Error::storage())?;
        if !exact || crate::now() >= attempt.original_until {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    /// Exact purpose read for an already owned attempt. No new SQL step/effect,
    /// lease renewal, telephone leg or permission to re-execute Core is created.
    pub fn try_telephone_replacement_commit_attempt_scene_write(
        &self,
        attempt: &TelephoneReplacementCommitAttempt,
        wait: &crate::AuthorityWait,
    ) -> Result<TelephoneReplacementSceneRead> {
        let until = attempt.original_until;
        let wait = wait.bounded(std::time::Duration::from_secs(
            (until - crate::now()).max(0) as u64,
        ));
        let derive = |c: &Connection| {
            self.require_unpublished_telephone_commit_attempt(c, attempt)?;
            self.telephone_replacement_phase_scopes(c, &attempt.resources, &attempt.sources, true)
        };
        let Some((held, c)) = self.telephone_mutation_admission(Some(&wait), derive)? else {
            return Err(Error::new(
                "authority-busy",
                "Same telephone attempt final admission is busy",
            ));
        };
        self.require_unpublished_telephone_commit_attempt(&c, attempt)?;
        if !self.authority_covers(
            &held,
            &self.telephone_replacement_phase_scopes(
                &c,
                &attempt.resources,
                &attempt.sources,
                true,
            )?,
        ) {
            return Err(Error::new(
                "authority-busy",
                "Same telephone attempt scope changed",
            ));
        }
        let read = self.telephone_scene_read_in(
            &c,
            attempt.resources.clone(),
            &attempt.sources,
            held,
            until,
        )?;
        let (_, descriptor, _) =
            self.telephone_commit_preparation_facts(&c, &read, Some(attempt))?;
        if descriptor != attempt.descriptor
            || scene_stamp(&read)? != attempt.original_scene
            || !read.current()
        {
            return Err(Error::forbidden());
        }
        wait.check()?;
        Ok(read)
    }
}

impl TelephoneReplacementCommitAttempt {
    pub fn attempt_binding(&self) -> (&str, &str, &str) {
        (
            &self
                .resources
                .attempt
                .origin
                .submitted
                .attempt
                .prepared
                .record,
            &self.attempt,
            &self.effect,
        )
    }
}
