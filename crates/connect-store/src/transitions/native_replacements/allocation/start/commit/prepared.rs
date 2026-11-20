//! Immutable preparation and exact same-attempt final admission. No preparation owns a grant.
use super::*;

/// Facts for cold local installation, never a scene read or an execution grant.
/// IDs are chosen once and inserted only by final begin under current Writer.
pub struct NativeReplacementCommitPreparation {
    pub(super) origin: Arc<NativeReplacementMediaOrigin>,
    pub(super) attempt: String,
    pub(super) effect: String,
    pub(super) activation: Arc<crate::NativeReplacementActivation>,
    pub(super) original_until: i64,
    pub(super) original_scene: String,
    expected: String,
    resources: String,
    source_expires_at: i64,
    target: serde_json::Value,
}
redacted!(NativeReplacementCommitPreparation);

/// Cloneable immutable routing for a fresh proof of ONE already-owned attempt.
/// This owns no authority and cannot capture an outcome or begin another attempt.
#[derive(Clone)]
pub struct NativeReplacementCommitAdmission {
    origin: Arc<NativeReplacementMediaOrigin>,
    attempt: String,
    effect: String,
    expected: String,
    target: PeerConnectionProjection,
    original_until: i64,
    original_scene: String,
    source_expires_at: i64,
    resources: String,
}
redacted!(NativeReplacementCommitAdmission);
impl NativeReplacementCommitAttempt {
    /// Exact attribution only, never current authority or permission to retry Core.
    pub fn attempt_binding(&self) -> (&str, &str, &str) {
        (
            &self
                .origin
                .started
                .attempt
                .allocated
                .attempt
                .prepared
                .record,
            &self.attempt,
            &self.effect,
        )
    }

    pub fn admission(&self) -> NativeReplacementCommitAdmission {
        NativeReplacementCommitAdmission {
            origin: self.origin.clone(),
            attempt: self.attempt.clone(),
            effect: self.effect.clone(),
            expected: self.expected.clone(),
            target: self.target.clone(),
            original_until: self.original_until,
            original_scene: self.original_scene.clone(),
            source_expires_at: self.source_expires_at,
            resources: self.resources.clone(),
        }
    }
}
pub(super) struct NativeCommitFacts {
    pub expected: String,
    pub resources: String,
    pub source_expires_at: i64,
    pub descriptor: serde_json::Value,
    pub private_successor:
        Option<crate::connection_control::private_consultation::replacement_lineage::Successor>,
}
fn target_stamp(view: &PeerConnectionProjection) -> serde_json::Value {
    serde_json::json!({"session":{"view":view.session.session,"actor":view.session.actor,
        "conversation_revision":view.session.conversation_revision,"present":view.session.present},
        "connection":view.connection,"runtime":view.runtime_connection_id,"device":view.device_id,
        "endpoint_address":view.endpoint_address,"endpoint_transport":view.endpoint_transport,
        "endpoint_capabilities":view.endpoint_capabilities,"accepted_capabilities":view.accepted_capabilities,
        "presence_role":view.presence_role,"offer":view.offer,"private_consultation":view.private_consultation,
        "handoff_preparation":view.handoff_preparation,"publication_suspended":view.publication_suspended})
}
fn scene_stamp(read: &NativeReplacementSceneRead) -> Result<String> {
    crate::provisioning::canonical_json(&serde_json::json!(read
        .reads
        .iter()
        .map(|(key, value)| {
            let v = value.target();
            (
                key,
                &v.runtime_connection_id,
                &v.connection.participant_id,
                v.connection.membership_epoch,
                v.connection.generation,
                v.connection.revision,
                &v.connection.state,
            )
        })
        .collect::<Vec<_>>()))
}
impl NativeReplacementCommitPreparation {
    pub fn attempt_binding(&self) -> (&str, &str, &str) {
        (
            &self
                .origin
                .started
                .attempt
                .allocated
                .attempt
                .prepared
                .record,
            &self.attempt,
            &self.effect,
        )
    }

    pub fn activation(&self) -> &Arc<crate::NativeReplacementActivation> {
        &self.activation
    }
    pub fn expires_at(&self) -> i64 {
        self.original_until
    }
    pub(super) fn new(
        store: &Store,
        origin: Arc<NativeReplacementMediaOrigin>,
        read: &NativeReplacementSceneRead,
        facts: &NativeCommitFacts,
    ) -> Result<Self> {
        let p = &origin.started.attempt.allocated.attempt.prepared;
        let attempt = format!("nmc_{}", uuid::Uuid::new_v4());
        let effect = format!("nre_{}", uuid::Uuid::new_v4());
        let activation = store.replacement_activation_metadata(
            crate::peer_authority::ReplacementActivationInput {
                record: p.record.clone(),
                attempt: attempt.clone(),
                effect: effect.clone(),
                runtime: p.runtime.clone(),
                target: read.media.target.clone(),
                binding: origin.started.attempt.request.clone(),
                descriptor: facts.descriptor.clone(),
                expires_at: facts.source_expires_at,
                original: p.selection.origin.clone(),
            },
        )?;
        Ok(Self {
            origin,
            attempt,
            effect,
            activation,
            original_until: read.expires_at(),
            original_scene: scene_stamp(read)?,
            expected: facts.expected.clone(),
            resources: facts.resources.clone(),
            source_expires_at: facts.source_expires_at,
            target: target_stamp(&read.media.target),
        })
    }
    pub(super) fn require_current(
        &self,
        read: &NativeReplacementSceneRead,
        facts: &NativeCommitFacts,
    ) -> Result<()> {
        if !read.current()
            || crate::now() >= self.original_until
            || self.expected != facts.expected
            || self.resources != facts.resources
            || facts.source_expires_at != self.source_expires_at
            || self.original_scene != scene_stamp(read)?
            || self.target != target_stamp(&read.media.target)
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl Store {
    /// Short read-only SQL projection. Caller drops its original planning read
    /// before codec, adoption, transport or Core metadata preparation can wait.
    pub fn prepare_native_replacement_commit(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        read: &NativeReplacementSceneRead,
    ) -> Result<NativeReplacementCommitPreparation> {
        let c = self.try_connection()?;
        let facts = self.native_commit_preparation_facts(&c, &origin, read, false, None)?;
        NativeReplacementCommitPreparation::new(self, origin, read, &facts)
    }
    /// Bind cold facts to the caller's once-chosen absolute phase cutoff.
    /// This can only shorten the real planning read, never mint a new grant.
    pub fn prepare_native_replacement_commit_until(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        read: &NativeReplacementSceneRead,
        original_until: i64,
    ) -> Result<NativeReplacementCommitPreparation> {
        let mut prepared = self.prepare_native_replacement_commit(origin, read)?;
        prepared.original_until = prepared.original_until.min(original_until);
        if crate::now() >= prepared.original_until {
            return Err(Error::forbidden());
        }
        Ok(prepared)
    }
    /// A new short planning read can only validate the same grant-free facts;
    /// this never begins or reauthorizes a possibly submitted movement.
    pub fn revalidate_prepared_native_replacement_commit(
        &self,
        preparation: &NativeReplacementCommitPreparation,
        read: &NativeReplacementSceneRead,
    ) -> Result<()> {
        let c = self.try_connection()?;
        let facts =
            self.native_commit_preparation_facts(&c, &preparation.origin, read, false, None)?;
        preparation.require_current(read, &facts)
    }
    pub(super) fn native_commit_preparation_facts(
        &self,
        c: &Connection,
        origin: &NativeReplacementMediaOrigin,
        read: &NativeReplacementSceneRead,
        capture_successor: bool,
        purpose: Option<&NativeReplacementCommitAdmission>,
    ) -> Result<NativeCommitFacts> {
        let p = &origin.started.attempt.allocated.attempt.prepared;
        if !read.current() || read.media.source.selection().origin.record != p.record {
            return Err(Error::forbidden());
        }
        if let Some(attempt) = purpose {
            self.require_unpublished_native_commit_attempt(c, attempt)?;
            if attempt
                .origin
                .started
                .attempt
                .allocated
                .attempt
                .prepared
                .record
                != p.record
            {
                return Err(Error::forbidden());
            }
        }
        let exists: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit')",
            [&p.record], |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if exists && purpose.is_none() {
            return Err(Error::new(
                "reconciliation-required",
                "Replacement movement was already attempted",
            ));
        }
        let (_, saved, scopes, _) = media_facts(self, c, &origin)?;
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
        let confirmed: u32 = c.query_row(
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
            crate::peer_authority::checked(self, c, &binding)?;
        if !self.authority_covers(&read.media.interval, &target_scopes) {
            return Err(Error::forbidden());
        }
        let descriptor: serde_json::Value =
            serde_json::from_str(&origin.started.attempt.target).map_err(|_| Error::storage())?;
        let mut intent: connect_protocol::connection_control::OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
        intent
            .offer
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("replacement_preparation");
        let recipients = crate::connection_control::intent_resources(c, &meta.actor, &intent)?;
        if recipients.iter().any(|r| !meta.actor.permits_recipient(r)) {
            return Err(Error::forbidden());
        }
        let resources = crate::provisioning::canonical_json(&serde_json::json!(recipients))?;
        let private_successor = if capture_successor {
            let value =
                crate::connection_control::private_consultation::replacement_lineage::capture(
                    self,
                    c,
                    &ctx.source_connection_id,
                    &ctx.retained_connection_id,
                    &read.media.target,
                    &descriptor,
                    source_expires_at,
                    &read.media.interval,
                )?;
            value
        } else {
            None
        };
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
        Ok(NativeCommitFacts {
            expected,
            resources,
            source_expires_at,
            descriptor,
            private_successor,
        })
    }

    fn require_unpublished_native_commit_attempt(
        &self,
        c: &Connection,
        attempt: &NativeReplacementCommitAdmission,
    ) -> Result<()> {
        let p = &attempt.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let exact: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json IS NULL AND e.provider='native-replacement-commit' AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.state='submitting' AND e.provider_ref IS NULL AND e.attempt=1 AND e.submission_generation=1 AND e.observation_json=?7 AND o.state='preparing' AND o.cancel_requested=0 AND o.worker_runtime=?8 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND o.expires_at>?9 AND n.expires_at>?9)",
            params![p.record,p.runtime,p.reservation,p.hash,attempt.attempt,attempt.effect,attempt.expected,self.transition_runtime,crate::now()], |row| row.get(0)
        ).map_err(|_| Error::storage())?;
        if !exact || crate::now() >= attempt.original_until {
            return Err(Error::forbidden());
        }
        Ok(())
    }

    /// Re-admit ONLY this owned started/submitting attempt; never reserve an
    /// attempt, renew a lease, record an outcome or infer permission to retry Core.
    /// Service must separately own the SAME linear Core StillUnpublished token.
    pub fn try_native_replacement_commit_attempt_scene_write(
        &self,
        attempt: &NativeReplacementCommitAdmission,
        sources: &[Arc<crate::NativeMediaOrigin>],
        wait: &crate::AuthorityWait,
    ) -> Result<NativeReplacementSceneRead> {
        {
            let c = self.try_connection_with_wait(wait)?;
            self.require_unpublished_native_commit_attempt(&c, attempt)?;
        }
        let provisional = self.try_native_replacement_scene_read(&attempt.origin, sources)?;
        if scene_stamp(&provisional)? != attempt.original_scene {
            return Err(Error::forbidden());
        }
        let scopes = provisional.scoped_commit_scopes()?;
        let until = provisional.expires_at().min(attempt.original_until);
        drop(provisional);
        let wait = wait.bounded(std::time::Duration::from_secs(
            (until - crate::now()).max(0) as u64,
        ));
        let writer = self.try_acquire_authority(&scopes)?;
        {
            let c = self.try_connection()?;
            self.require_unpublished_native_commit_attempt(&c, attempt)?;
        }
        let read = self.native_replacement_scene_with_interval(
            &attempt.origin,
            sources,
            None,
            Some((&writer, until)),
        )?;
        if !read.current()
            || !self.authority_covers(&writer, &read.scoped_commit_scopes()?)
            || scene_stamp(&read)? != attempt.original_scene
            || target_stamp(&read.media.target) != target_stamp(&attempt.target)
        {
            return Err(Error::forbidden());
        }
        let facts = {
            let connection = self.try_connection()?;
            self.native_commit_preparation_facts(
                &connection,
                &attempt.origin,
                &read,
                false,
                Some(attempt),
            )?
        };
        if facts.expected != attempt.expected
            || facts.resources != attempt.resources
            || facts.source_expires_at != attempt.source_expires_at
        {
            return Err(Error::forbidden());
        }
        wait.check()?;
        Ok(read)
    }
}
