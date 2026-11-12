//! Once-only movement custody. A captured commit is historical metadata, not a
//! packet grant or a completed public projection. The owner must retain it
//! before awaiting any metadata write and must never retry the physical commit.
use super::*;
pub(super) mod not_submitted;
mod recovery;
mod retired_source;
pub use not_submitted::NativeReplacementCommitNotSubmitted;
use recovery::{Projection, ProjectionFacts};

// Invocation-local test observation only. It grants no authority, supplies no
// error, and is absent from shipping. The actual old entry/SQL body still runs.
#[cfg(test)]
thread_local! {
    static COMMIT_ENTRY_PROBE: std::cell::RefCell<Option<(&'static str, Box<dyn FnOnce()>)>> =
        std::cell::RefCell::new(None);
}
#[cfg(test)]
pub(super) fn observe_commit_entry_probe(stage: &'static str) {
    let probe = COMMIT_ENTRY_PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|(expected, _)| *expected == stage)
        {
            slot.take().map(|(_, probe)| probe)
        } else {
            None
        }
    });
    if let Some(probe) = probe {
        probe();
    }
}
#[cfg(test)]
impl Store {
    pub(crate) fn with_native_replacement_commit_entry_probe_for_test<T>(
        stage: &'static str,
        probe: impl FnOnce() + 'static,
        call: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                COMMIT_ENTRY_PROBE.with(|slot| slot.borrow_mut().take());
            }
        }
        COMMIT_ENTRY_PROBE.with(|slot| {
            assert!(slot.borrow().is_none(), "nested Commit entry probe");
            *slot.borrow_mut() = Some((stage, Box::new(probe)));
        });
        let _reset = Reset;
        call()
    }
}

pub struct NativeReplacementCommitAttempt {
    origin: Arc<NativeReplacementMediaOrigin>,
    attempt: String,
    effect: String,
    expected: String,
    target: PeerConnectionProjection,
    resources: String,
    source_expires_at: i64,
    activation: Arc<crate::NativeReplacementActivation>,
    private_successor:
        Option<crate::connection_control::private_consultation::replacement_lineage::Successor>,
}
pub struct NativeReplacementCommitted {
    attempt: NativeReplacementCommitAttempt,
    observation: String,
    audience: Option<NativeReplacementAudienceCapture>,
    capture_diagnostics: super::projection::AdmissionCounters,
    activation_diagnostics: super::projection::AdmissionCounters,
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
    /// Installation metadata for this exact once-only reservation, never IO authority.
    pub fn activation(&self) -> &Arc<crate::NativeReplacementActivation> {
        &self.activation
    }

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
            || ready.private_consultation.as_deref()
                != self
                    .private_successor
                    .as_ref()
                    .map(|lineage| lineage.consultation_record_id.as_str())
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
            if !unique.insert(stream.as_str())
                || !offered.iter().any(|s| s["id"].as_str() == Some(stream))
            {
                return Err(conflict());
            }
        }
        // Preserve every identity captured at the once-only native resource
        // start. Omitting quarantined video must not erase its durable binding.
        let resources: serde_json::Value =
            serde_json::from_str(&self.origin.started.observation).map_err(|_| Error::storage())?;
        let expected_streams = resources["streams"]
            .as_array()
            .ok_or_else(Error::storage)?
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<std::collections::BTreeSet<_>>>()
            .ok_or_else(Error::storage)?;
        if unique != expected_streams {
            return Err(conflict());
        }
        let observation = crate::provisioning::canonical_json(&serde_json::json!({
            "movement":movement,
            "projection":ProjectionFacts::from_attempt(&self),
            "readiness":{"streams":ready.streams,"stream_revision":ready.stream_revision,
                         "participant_revision":ready.participant_revision}
        }))?;
        Ok(NativeReplacementCommitted {
            attempt: self,
            observation,
            audience: None,
            capture_diagnostics: Default::default(),
            activation_diagnostics: Default::default(),
        })
    }
    pub fn uncertain(self) -> NativeReplacementCommitUncertain {
        NativeReplacementCommitUncertain { attempt: self }
    }
}
impl NativeReplacementCommitted {
    pub(crate) fn commit_ledger_key(&self) -> crate::authority_writer_boundary::phase_ledger::Key {
        let a = &self.attempt;
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        crate::authority_writer_boundary::phase_ledger::Key {
            record: p.record.clone(),
            phase: "commit",
            attempt: a.attempt.clone(),
            effect: a.effect.clone(),
            expected: self.observation.clone(),
            runtime: p.runtime.clone(),
            reservation: p.reservation.clone(),
            digest: p.hash.clone(),
            stage: crate::authority_writer_boundary::phase_ledger::Stage::Confirm,
        }
    }
    pub(crate) fn commit_ledger_descriptor(&self) -> &str {
        &self.attempt.expected
    }
    pub(crate) fn commit_ledger_audience(&self) -> Option<&NativeReplacementAudienceCapture> {
        self.audience.as_ref()
    }
    pub(crate) fn commit_ledger_authority_decision(
        &self,
        store: &Store,
        c: &Connection,
        held: Option<&Arc<AuthorityGuardSet>>,
    ) -> Result<(Vec<AuthorityScope>, i64, &'static str)> {
        let p = &self
            .attempt
            .origin
            .started
            .attempt
            .allocated
            .attempt
            .prepared;
        let (_, _, mut scopes, until) = match media_facts(store, c, &self.attempt.origin) {
            Ok(facts) => facts,
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "forbidden" | "unauthorized" | "not-ready" | "conflict"
                ) =>
            {
                return match retired_source::authority(self, store, c, held) {
                    Ok((scopes, until)) => Ok((scopes, until, "exact-retired-original")),
                    Err(e)
                        if matches!(
                            e.code.as_str(),
                            "forbidden" | "unauthorized" | "not-ready" | "conflict"
                        ) =>
                    {
                        Err(error)
                    }
                    Err(e) => Err(e),
                };
            }
            Err(error) => return Err(error),
        };
        scopes.extend(crate::authority_admission::conversation_scopes(
            c,
            &p.selection.origin.request().conversation_id,
            crate::AuthorityMode::Read,
        )?);
        let until = if let Some(held) = held {
            p.selection
                .read_with_connection(store, c, held.clone(), until)?
                .expires_at()
        } else {
            until
                .min(p.selection.physical_expiry())
                .min(p.selection.origin.request().expires_at)
        };
        Ok((AuthorityScope::merged(&scopes)?, until, "live-original"))
    }
    /// Attach the Core-sealed packetization boundary exactly once before this
    /// original immutable observation is recorded. No DB or physical mutation.
    pub fn capture_native_activation(
        &mut self,
        audience: connect_protocol::replacement_activation::NativeActivationAudience,
    ) -> Result<()> {
        use connect_protocol::replacement_activation::{
            NativeActivationAudience as A, NativeActivationBoundary, PROFILE,
        };
        let value: serde_json::Value =
            serde_json::from_str(&self.observation).map_err(|_| Error::storage())?;
        if value.get("native_activation").is_some() {
            return Err(conflict());
        }
        let p = &self
            .attempt
            .origin
            .started
            .attempt
            .allocated
            .attempt
            .prepared;
        let original = p.selection.origin.request();
        let private = self
            .attempt
            .private_successor
            .as_ref()
            .map(|l| l.consultation_record_id.as_str());
        validate_activation_substrate(&self.attempt.target.offer, &audience)?;
        match (&audience, private) {
            (A::PrivateConsultation { original_operation }, Some(parent))
                if original_operation == parent => {}
            (A::Public { streams }, None) => {
                // The Core outcome may name only selected receiving audio and
                // explicitly inherited receiver-video mappings, never Data or
                // omitted/quarantined auxiliary media.
                let offered = self.attempt.target.offer["streams_offered"]
                    .as_array()
                    .ok_or_else(invalid)?;
                let video = match &original.intent {
                    connect_protocol::transitions::TransitionIntent::Replace { video, .. } => {
                        video.as_ref()
                    }
                    _ => return Err(conflict()),
                };
                let mut expected = std::collections::BTreeSet::new();
                for offer in offered {
                    if offer["kind"] == "audio" {
                        if offer["direction"] != "sendonly" {
                            expected.insert(offer["id"].as_str().ok_or_else(invalid)?.to_owned());
                        }
                    }
                }
                if let Some(video) = video {
                    for mapping in &video.receivers {
                        expected.insert(mapping.replacement.clone());
                    }
                }
                let actual = streams
                    .iter()
                    .map(|s| s.stream_id.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                if actual != expected {
                    return Err(conflict());
                }
            }
            (A::Unsupported { reason }, None)
                if reason == "non-native-adapter"
                    && self.attempt.target.offer["substrate"] == "websocket+webrtc" => {}
            _ => return Err(conflict()),
        }
        let boundary = NativeActivationBoundary {
            profile: PROFILE.into(),
            operation_record_id: p.record.clone(),
            operation_id: original.operation_id.clone(),
            source_connection_id: original.connection_id.clone().ok_or_else(invalid)?,
            target_connection_id: self.attempt.target.connection.connection_id.clone(),
            target_generation: self.attempt.target.connection.generation,
            stream_revision: value["readiness"]["stream_revision"]
                .as_u64()
                .ok_or_else(invalid)?,
            audience,
        };
        attach_activation_observation(&mut self.observation, &boundary)
    }
}

fn validate_activation_substrate(
    saved_offer: &serde_json::Value,
    audience: &connect_protocol::replacement_activation::NativeActivationAudience,
) -> Result<()> {
    use connect_protocol::replacement_activation::NativeActivationAudience as A;
    match (saved_offer["substrate"].as_str(), audience) {
        (Some("quic" | "webtransport"), A::Public { .. } | A::PrivateConsultation { .. }) => Ok(()),
        (Some("websocket+webrtc"), A::Unsupported { reason }) if reason == "non-native-adapter" => {
            Ok(())
        }
        (Some("websocket+webrtc"), A::PrivateConsultation { .. }) => Ok(()),
        _ => Err(conflict()),
    }
}
fn attach_activation_observation(
    observation: &mut String,
    boundary: &connect_protocol::replacement_activation::NativeActivationBoundary,
) -> Result<()> {
    // This helper deliberately borrows the captured effect. Every failure
    // preserves its exact bytes for Known custody and stop/reconciliation.
    let mut value: serde_json::Value =
        serde_json::from_str(observation).map_err(|_| Error::storage())?;
    if value.get("native_activation").is_some() {
        return Err(conflict());
    }
    boundary.validate()?;
    value["native_activation"] = serde_json::to_value(boundary).map_err(|_| Error::storage())?;
    let captured = crate::provisioning::canonical_json(&value)?;
    *observation = captured;
    Ok(())
}
#[cfg(test)]
mod activation_capture_tests {
    use super::*;
    use connect_protocol::replacement_activation::{
        NativeActivationAudience as A, NativeActivationBoundary as B,
    };
    #[test]
    fn exact_saved_native_and_webrtc_offer_profiles_cannot_be_interchanged() {
        let unsupported = A::Unsupported {
            reason: "non-native-adapter".into(),
        };
        let public = A::Public { streams: vec![] };
        let private = A::PrivateConsultation {
            original_operation: "consult".into(),
        };
        let rtc = serde_json::json!({"substrate":"websocket+webrtc"});
        assert!(validate_activation_substrate(&rtc, &unsupported).is_ok());
        assert!(validate_activation_substrate(&rtc, &public).is_err());
        assert!(validate_activation_substrate(&rtc, &private).is_ok());
        for substrate in ["quic", "webtransport"] {
            let offer = serde_json::json!({"substrate":substrate});
            assert!(validate_activation_substrate(&offer, &public).is_ok());
            assert!(validate_activation_substrate(&offer, &unsupported).is_err());
        }
        for substrate in ["webrtc", "websocket", "other", ""] {
            let offer = serde_json::json!({"substrate":substrate});
            assert!(validate_activation_substrate(&offer, &public).is_err());
            assert!(validate_activation_substrate(&offer, &unsupported).is_err());
        }
    }
    #[test]
    fn failed_activation_attachment_preserves_exact_captured_effect_for_known_custody() {
        let original =
            r#"{"movement":{"original":"already-committed"},"readiness":{"stream_revision":3}}"#
                .to_owned();
        let mut observation = original.clone();
        let mut boundary = B {
            profile: connect_protocol::replacement_activation::PROFILE.into(),
            operation_record_id: "record".into(),
            operation_id: "move".into(),
            source_connection_id: "old".into(),
            target_connection_id: "new".into(),
            target_generation: 2,
            stream_revision: 2,
            audience: A::Unsupported {
                reason: "non-native-adapter".into(),
            },
        };
        assert!(attach_activation_observation(&mut observation, &boundary).is_err());
        assert_eq!(observation, original);
        boundary.stream_revision = 3;
        attach_activation_observation(&mut observation, &boundary).unwrap();
        let sealed = observation.clone();
        assert!(attach_activation_observation(&mut observation, &boundary).is_err());
        assert_eq!(observation, sealed);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&observation).unwrap()["movement"]
                ["original"],
            "already-committed"
        );
    }
}

impl Store {
    /// Acquire the complete current scene Writer for one original Commit.
    /// It covers the independent operator, old/retained owners and target peer.
    /// An effect-free SQL availability preview belongs AFTER deriving and
    /// dropping the complete provisional read, BEFORE any WriterNotice. The
    /// preview grants nothing; actual begin still owns its SAME fresh Unit.
    pub fn try_native_replacement_commit_scene_write(
        &self,
        origin: &NativeReplacementMediaOrigin,
        sources: &[Arc<crate::NativeMediaOrigin>],
    ) -> Result<NativeReplacementSceneRead> {
        let provisional = self.try_native_replacement_scene_read(origin, sources)?;
        let scopes = provisional.media.interval.mutation_scopes();
        let expiry = provisional.media.source.expires_at();
        drop(provisional);
        #[cfg(test)]
        observe_commit_entry_probe("scene-derived");
        // No SQL or provisional reader survives into waited file admission.
        drop(self.try_connection()?);
        let wait = crate::authority_admission::passive_projection_wait();
        let writer = self.acquire_authority(&scopes, &wait)?;
        self.native_replacement_scene_with_interval(origin, sources, None, Some((&writer, expiry)))
    }
    /// Persist the single commit attempt before entering Core while retaining
    /// the full current Writer through the physical movement fence.
    /// failure to enter primary SQL is a definite pre-effect Busy, never a wait.
    pub fn begin_native_replacement_commit(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        read: &NativeReplacementSceneRead,
    ) -> Result<NativeReplacementCommitAttempt> {
        self.try_begin_native_replacement_commit(origin, read)?
            .ok_or_else(|| {
                Error::new(
                    "authority-busy",
                    "Commit metadata is busy before its original attempt",
                )
            })
    }
    /// None means ONLY the primary Unit could not be entered before any step,
    /// effect, transaction or Core action. All later errors remain errors and
    /// must never be converted into a pre-effect retry by their error code.
    pub fn try_begin_native_replacement_commit(
        &self,
        origin: Arc<NativeReplacementMediaOrigin>,
        read: &NativeReplacementSceneRead,
    ) -> Result<Option<NativeReplacementCommitAttempt>> {
        let p = &origin.started.attempt.allocated.attempt.prepared;
        if !read.current()
            || read.media.source.selection().origin.record != p.record
            || !self.authority_covers(&read.media.interval, &read.media.interval.mutation_scopes())
        {
            return Err(Error::forbidden());
        }
        #[cfg(test)]
        observe_commit_entry_probe("begin-sql");
        let mut c = match self.try_connection() {
            Ok(connection) => connection,
            Err(error) if error.code == "authority-busy" => return Ok(None),
            Err(error) => return Err(error),
        };
        c.bind_available_mutation_authority(self, &read.media.interval)?;
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
        let descriptor: serde_json::Value =
            serde_json::from_str(&origin.started.attempt.target).map_err(|_| Error::storage())?;
        let mut intent: connect_protocol::connection_control::OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
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
        let private_successor =
            crate::connection_control::private_consultation::replacement_lineage::capture(
                self,
                &tx,
                &ctx.source_connection_id,
                &ctx.retained_connection_id,
                &read.media.target,
                &descriptor,
                source_expires_at,
                &read.media.interval,
            )?;
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
        let activation = self.replacement_activation_metadata(
            crate::peer_authority::ReplacementActivationInput {
                record: p.record.clone(),
                attempt: attempt.clone(),
                effect: effect.clone(),
                runtime: p.runtime.clone(),
                target: read.media.target.clone(),
                binding: origin.started.attempt.request.clone(),
                descriptor,
                expires_at: source_expires_at,
                original: p.selection.origin.clone(),
            },
        )?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(NativeReplacementCommitAttempt {
            origin,
            attempt,
            effect,
            expected,
            target: read.media.target.clone(),
            resources,
            source_expires_at,
            activation,
            private_successor,
        }))
    }
    /// Persist a known actual outcome after all transient engine grants have
    /// been dropped. This does not reauthorize or reexecute movement.
    pub fn record_native_replacement_committed(
        &self,
        captured: &NativeReplacementCommitted,
    ) -> Result<OperationRecord> {
        // Re-observing an already recorded immutable outcome is a fresh read,
        // not a current-state mutation. In particular, the preparation watcher
        // may arrive after public activation; reserving a Writer here would
        // revoke unrelated queued media even though no SQL row changes.
        if let Some(record) = self.read_recorded_replacement_commit(captured)? {
            return Ok(record);
        }
        self.record_replacement_commit(
            &captured.attempt,
            Some(&captured.observation),
            captured.audience.as_ref(),
            Some(&captured.capture_diagnostics),
            Some(captured),
        )
    }
    fn read_recorded_replacement_commit(
        &self,
        captured: &NativeReplacementCommitted,
    ) -> Result<Option<OperationRecord>> {
        let a = &captured.attempt;
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let (held, c) = self.try_replacement_capture_projection(None, |c| {
            let row = rows::by_record(c, &p.record)?;
            let mut scopes = admission::row_scopes(&row)?;
            if captured.audience.is_some() {
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &row.record.conversation_id.clone().ok_or_else(invalid)?,
                    crate::AuthorityMode::Read,
                )?);
            }
            AuthorityScope::merged(&scopes)
        })?;
        let row = rows::by_record(&c, &p.record)?;
        admission::require_row(self, &held, &row)?;
        // Exact original runtime, reservation, plan, attempt, effect and expected
        // result remain required. Equal operation IDs alone cannot use this path.
        let old: Option<Option<String>> = c.query_row(
            "SELECT s.observation_json FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND e.observation_json=?7",
            params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,a.expected],
            |r| r.get(0),
        ).optional().map_err(|_| Error::storage())?;
        let Some(old) = old.ok_or_else(Error::forbidden)? else {
            // No outcome yet: this scope releases SQL and every Read before
            // the existing Writer path rechecks everything and records once.
            return Ok(None);
        };
        if old != captured.observation {
            return Err(conflict());
        }
        if let Some(audience) = &captured.audience {
            audience.require_recorded(&c, &a.effect)?;
        }
        Ok(Some(row.record))
    }
    pub fn native_replacement_commit_uncertain(
        &self,
        captured: &NativeReplacementCommitUncertain,
    ) -> Result<OperationRecord> {
        self.record_replacement_commit(&captured.attempt, None, None, None, None)
    }
    fn record_replacement_commit(
        &self,
        a: &NativeReplacementCommitAttempt,
        observation: Option<&str>,
        audience: Option<&NativeReplacementAudienceCapture>,
        diagnostics: Option<&super::projection::AdmissionCounters>,
        positive: Option<&NativeReplacementCommitted>,
    ) -> Result<OperationRecord> {
        let p = &a.origin.started.attempt.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        // This effect-free preview chooses only a provisional footprint. Its
        // full policy derivation must not consume the shared five-millisecond
        // SQL/file admission window. The immediate SQL try adds no wait, and
        // its guard drops here before that original admission window starts.
        let preview = if let Some(capture) = positive {
            let c = self.try_connection()?;
            crate::authority_writer_boundary::phase_ledger::commit::decision(self, &c, capture)?
        } else {
            crate::authority_writer_boundary::phase_ledger::commit::Decision {
                reader: false,
                reason: "non-positive-observation",
            }
        };
        let (held, mut c) = self.try_replacement_capture_projection(diagnostics, |c| {
            let row = rows::by_record(c, &p.record)?;
            // This hint grants nothing. The actual transaction below repeats
            // complete current validation and refuses a stale Read footprint.
            let eligible = preview.reader;
            let mut scopes = if eligible {
                admission::row_scopes(&row)?
            } else {
                admission::mutation_row_scopes(&row)?
            };
            if audience.is_some() {
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &row.record.conversation_id.clone().ok_or_else(invalid)?,
                    if eligible {
                        crate::AuthorityMode::Read
                    } else {
                        crate::AuthorityMode::Write
                    },
                )?);
            }
            AuthorityScope::merged(&scopes)
        })?;
        let mut timing = super::projection::TransactionTiming::new(diagnostics);
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        timing.body();
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let old:Option<(Option<String>,String)>=tx.query_row("SELECT s.observation_json,e.state FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND e.observation_json=?7",params![p.record,p.runtime,p.reservation,p.hash,a.attempt,a.effect,a.expected],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (old, state) = old.ok_or_else(Error::forbidden)?;
        if let Some(old) = old {
            if Some(old.as_str()) == observation {
                if let Some(audience) = audience {
                    audience.require_recorded(&tx, &a.effect)?;
                }
                drop(tx);
                timing.unchanged();
                return Ok(row.record);
            }
            return Err(conflict());
        }
        if observation.is_none() && state == "unknown" {
            drop(tx);
            timing.unchanged();
            return Ok(row.record);
        }
        // Eligibility may have crossed an expiry/cancel boundary after scope
        // derivation. Never upgrade a retained Reader behind itself.
        let actual = positive
            .map(|capture| {
                crate::authority_writer_boundary::phase_ledger::commit::decision(self, &tx, capture)
            })
            .transpose()?
            .unwrap_or(
                crate::authority_writer_boundary::phase_ledger::commit::Decision {
                    reader: false,
                    reason: "non-positive-observation",
                },
            );
        let eligible = actual.reader;
        let required = if eligible {
            admission::row_scopes(&row)?
        } else {
            admission::mutation_row_scopes(&row)?
        };
        if !self.authority_covers(&held, &required) {
            return Err(Error::new(
                "authority-busy",
                "Commit capture now requires original writer custody",
            ));
        }
        let permit = if eligible {
            Some(
                self.authority_writer_boundary
                    .native_commit_capture_ledger(
                        self,
                        &tx,
                        &held,
                        positive.ok_or_else(Error::forbidden)?,
                    )?,
            )
        } else {
            None
        };
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
            if eligible {
                audience.record_positive_commit(&tx, self, &a.effect)?;
            } else {
                audience.record(&tx, self, &a.effect)?;
            }
        }
        if let Some(permit) = &permit {
            native_handoffs::touch_at(&tx, &p.record, self, permit.commit_capture_recorded_at()?)?;
        } else {
            native_handoffs::touch(&tx, &p.record, self)?;
        }
        let record = rows::by_record(&tx, &p.record)?.record;
        if let Some(permit) = &permit {
            local_owner(self, &p.owner, &p.runtime)?;
            permit.require_commit_capture_current()?;
        }
        drop(permit);
        timing.committing();
        tx.commit().map_err(|_| Error::storage())?;
        timing.committed();
        tracing::debug!(target:"connect_store::native_commit_capture",record=%p.record,
            mode=if eligible {"positive-reader"} else {"writer-fallback"},
            preview_reason=preview.reason, transaction_reason=actual.reason,
            "Historical replacement Commit capture recorded");
        Ok(record)
    }
}

enum ActivationObservation<'a> {
    Installed(&'a dyn crate::NativeReplacementInstallationObservation),
    Stopped(&'a dyn crate::NativeOfferRuntimeObservation),
}
impl ActivationObservation<'_> {
    fn stopped(&self) -> bool {
        matches!(self, Self::Stopped(_))
    }
    fn require(
        &self,
        activation: Option<&crate::NativeReplacementActivation>,
        core: &str,
        generation: u64,
    ) -> Result<Option<Box<dyn crate::NativeReplacementInstallationGuard + '_>>> {
        match self {
            Self::Installed(observation) => {
                return observation
                    .hold_installed(activation.ok_or_else(Error::forbidden)?)?
                    .map(Some)
                    .ok_or_else(|| {
                        Error::new("authority-busy", "Exact replacement entry is not installed")
                    });
            }
            Self::Stopped(observation) => {
                let expected = crate::NativeOfferObservation::Retired {
                    runtime_connection_id: core.into(),
                    generation,
                };
                if observation.observe()? != expected {
                    return Err(Error::forbidden());
                }
            }
        }
        Ok(None)
    }
}

impl Store {
    /// Project a captured physical commit atomically. Retries write metadata
    /// only, even after the old route's cleanup, cancellation or revocation.
    pub fn project_native_replacement_committed(
        &self,
        captured: &NativeReplacementCommitted,
        installed: &dyn crate::NativeReplacementInstallationObservation,
    ) -> Result<Arc<crate::NativeMediaOrigin>> {
        self.project_replacement_activation(captured, ActivationObservation::Installed(installed))
    }

    /// A known physical move whose new route stopped before activation is still
    /// recorded exactly once. Emit retirement, never false public readiness.
    pub fn project_native_replacement_stopped(
        &self,
        captured: &NativeReplacementCommitted,
        absent: &dyn crate::NativeOfferRuntimeObservation,
    ) -> Result<Arc<crate::NativeMediaOrigin>> {
        self.project_replacement_activation(captured, ActivationObservation::Stopped(absent))
    }

    fn project_replacement_activation(
        &self,
        captured: &NativeReplacementCommitted,
        observation: ActivationObservation<'_>,
    ) -> Result<Arc<crate::NativeMediaOrigin>> {
        let p = &captured
            .attempt
            .origin
            .started
            .attempt
            .allocated
            .attempt
            .prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let projection = Projection::from_capture(captured)?;
        self.project_replacement_metadata(&projection, Some(captured), observation)?
            .ok_or_else(Error::forbidden)
    }

    // A successor can settle immutable historical facts only as stopped. It
    // cannot construct the sealed activation context or return a media origin.
    fn project_replacement_metadata(
        &self,
        p: &Projection,
        live: Option<&NativeReplacementCommitted>,
        observation: ActivationObservation<'_>,
    ) -> Result<Option<Arc<crate::NativeMediaOrigin>>> {
        let owner = self.require_runtime_owner()?;
        let stopped = observation.stopped();
        if live.is_none() && (!stopped || p.runtime == self.runtime_epoch()) {
            return Err(Error::forbidden());
        }
        let activation = live.map(NativeReplacementCommitted::activation);
        let target = &p.facts.target;
        drop(observation.require(
            activation.map(AsRef::as_ref),
            &target.runtime_connection_id,
            target.connection.generation,
        )?);
        let (held, mut c) = self.try_replacement_publication_projection(
            live.map(|capture| &capture.activation_diagnostics),
            |c| {
                let row = rows::by_record(c, &p.record)?;
                let mut scopes = admission::row_scopes(&row)?;
                scopes.extend(crate::authority_admission::conversation_scopes(
                    c,
                    &target.connection.conversation_id,
                    crate::AuthorityMode::Write,
                )?);
                AuthorityScope::merged(&scopes)
            },
        )?;
        let mut timing = super::projection::TransactionTiming::new(
            live.map(|capture| &capture.activation_diagnostics),
        );
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        timing.body();
        let row = rows::by_record(&tx, &p.record)?;
        admission::require_row(self, &held, &row)?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7 AND e.observation_json=?8)",params![p.record,p.runtime,p.reservation,p.hash,p.attempt,p.effect,p.observation,p.expected],|r|r.get(0)).map_err(|_| Error::storage())?;
        if !exact {
            return Err(Error::forbidden());
        }
        let descriptor: serde_json::Value =
            serde_json::from_str(&p.descriptor).map_err(|_| Error::storage())?;
        let intent: connect_protocol::connection_control::OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
        let public_key = serde_json::from_value(descriptor["public_key"].clone())
            .map_err(|_| Error::storage())?;
        if row.record.state == OperationState::Completed {
            if row
                .record
                .result
                .as_ref()
                .and_then(|r| r["activation_state"].as_str())
                != Some(if stopped { "stopped" } else { "ready" })
            {
                return Err(conflict());
            }
            if stopped {
                drop(tx);
                timing.unchanged();
                return Ok(live.map(|capture| capture.activation().origin().clone()));
            }
            let origin = self.replacement_established_origin(
                &tx,
                &p.record,
                live.ok_or_else(Error::forbidden)?
                    .attempt
                    .origin
                    .started
                    .attempt
                    .request
                    .clone(),
                intent,
                public_key,
            )?;
            drop(tx);
            timing.unchanged();
            return Ok(Some(origin));
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        if !stopped {
            self.require_native_replacement_activation(
                &tx,
                activation.ok_or_else(Error::forbidden)?,
            )?;
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
        // The immutable lifetime triggers bind the actual captured admission
        // before its retirement fence. These intermediate rows stay inside this
        // transaction, behind its complete authority writer: the stopped path
        // retires them before journal emission or commit, so no reader sees Ready.
        let state = "ready";
        let generation = target.connection.generation;
        let observed_runtime = Some(p.runtime.as_str());
        let observed_at = Some(crate::now());
        let ready: serde_json::Value =
            serde_json::from_str(&p.observation).map_err(|_| Error::storage())?;
        let readiness = Some(ready["readiness"].to_string());
        let mut offer = intent.offer.clone();
        offer
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("replacement_preparation");
        let recipients: Vec<String> =
            serde_json::from_str(&p.facts.resources).map_err(|_| Error::storage())?;
        if !crate::conversations::capacity(&tx, 1 + 2 * recipients.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Replacement lifecycle journal capacity exhausted",
            ));
        }
        let claim: Option<String> = tx
            .query_row(
                "SELECT claim_json FROM native_replacement_candidates WHERE operation_record_id=?1",
                [&p.record],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some(claim) = claim {
            let claim: serde_json::Value =
                serde_json::from_str(&claim).map_err(|_| Error::storage())?;
            let changed=tx.execute("UPDATE connections SET generation=?2,replaces=?3,state=?13,revision=revision+1,observed_runtime_id=?4,observed_at=?5,readiness_json=?6 WHERE id=?1 AND session_id=?7 AND participant_id=?8 AND membership_epoch=?9 AND identity_id=?10 AND endpoint_id=?11 AND state='retired' AND generation=?12+1 AND observed_runtime_id IS NULL AND readiness_json IS NULL",params![target.connection.connection_id,generation,source,observed_runtime,observed_at,readiness,target.connection.session_id,target.connection.participant_id,target.connection.membership_epoch,target.identity_id,target.connection.endpoint_id,claim["generation"].as_u64().ok_or_else(invalid)?,state]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
            let changed=tx.execute("UPDATE connection_offers SET runtime_connection_id=?2,operation_id=?3,credential_id=?4,offer_json=?5,created_at=?6 WHERE connection_id=?1 AND runtime_connection_id=?7 AND operation_id=?8 AND credential_id=?9",params![target.connection.connection_id,target.runtime_connection_id,intent.operation_id,descriptor["credential_id"].as_str().ok_or_else(invalid)?,offer.to_string(),crate::now(),claim["core"].as_str().ok_or_else(invalid)?,claim["operation"].as_str().ok_or_else(invalid)?,claim["credential"].as_str().ok_or_else(invalid)?]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
        } else {
            // Successor startup first fences the old epoch and then confirms
            // retirement, so its terminal source can advance more than once.
            // Only the exact original retired lifetime and unchanged physical
            // mapping can supply historical lineage; a newer live source cannot.
            tx.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,replaces,state,revision,observed_runtime_id,observed_at,readiness_json) SELECT ?1,?2,?3,?4,?5,tenant_id,?6,?7,id,?13,1,?8,?9,?10 FROM connections WHERE id=?11 AND (generation BETWEEN ?12 AND ?12+1 OR (?14 AND state='retired' AND generation>?12 AND observed_runtime_id IS NULL AND readiness_json IS NULL AND EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN connection_offers m ON m.connection_id=l.connection_id WHERE l.connection_id=connections.id AND l.generation=?12 AND l.runtime_epoch=?15 AND l.retired=1 AND m.runtime_connection_id=?16)))",params![target.connection.connection_id,target.connection.session_id,target.connection.participant_id,target.connection.membership_epoch,target.identity_id,target.connection.endpoint_id,generation,observed_runtime,observed_at,readiness,source,saved["source_generation"].as_u64().ok_or_else(invalid)?,state,live.is_none() && stopped,p.runtime,ready["movement"]["source"].as_str().ok_or_else(invalid)?]).map_err(|_| Error::storage())?;
            tx.execute("INSERT INTO connection_offers(connection_id,runtime_connection_id,operation_id,credential_id,offer_json,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![target.connection.connection_id,target.runtime_connection_id,intent.operation_id,descriptor["credential_id"].as_str().ok_or_else(invalid)?,offer.to_string(),crate::now()]).map_err(|_| Error::storage())?;
        }
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
        let retained_connection_id = rows::request(&row)?.intent.retained().to_owned();
        let retired = if stopped {
            vec![source.clone(), target.connection.connection_id.clone()]
        } else {
            vec![source.clone()]
        };
        let result = serde_json::json!({"activation_state":if stopped { "stopped" } else { "ready" },"retained_connection_ids":[retained_connection_id],"admitted_connection_ids":[target.connection.connection_id],"retired_connection_ids":retired,"conversation_revision":null,"connection_id":target.connection.connection_id,"replaces":source,"generation":target.connection.generation,"participant_id":target.connection.participant_id,"membership_epoch":target.connection.membership_epoch,"endpoint_id":target.connection.endpoint_id,"device_id":target.device_id,"readiness":if stopped { serde_json::Value::Null } else { ready["readiness"].clone() },"physical_readiness":ready["readiness"],"native_activation":ready["native_activation"],"session_revision":topology_revision});
        tx.execute(
            "UPDATE operations SET state='completed',result_json=?2 WHERE record_id=?1",
            params![p.record, result.to_string()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO native_replacement_lifetimes(operation_record_id,credential_id,key_id,device_id,endpoint_id,participant_id,membership_epoch,generation,resources_json,source_expires_at,attempt_runtime,attempt_id,effect_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![p.record,descriptor["credential_id"].as_str().ok_or_else(invalid)?,descriptor["key_id"].as_str().ok_or_else(invalid)?,target.device_id,target.connection.endpoint_id,target.connection.participant_id,target.connection.membership_epoch,target.connection.generation,p.facts.resources,p.facts.source_expires_at,p.runtime,p.attempt,p.effect]).map_err(|_| Error::storage())?;
        tx.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation,retired) VALUES(?1,?2,?3,?4,?5)",params![p.record,target.connection.connection_id,p.runtime,target.connection.generation,stopped]).map_err(|_| Error::storage())?;
        if !stopped {
            if let Some(lineage) = &p.facts.private_successor {
                // New rows remain invisible under this full writer until the
                // installed exact Core owner publishes the same transaction.
                // Recheck the new physical private pair, never the retired
                // predecessor or its historical execution credential.
                crate::connection_control::private_consultation::replacement_lineage::require_published(self,&tx,lineage,&held)?;
            }
        }
        if stopped {
            let changed = tx.execute(
                "UPDATE connections SET state='retired',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE id=?1 AND generation=?2 AND state='ready'",
                params![target.connection.connection_id, generation],
            ).map_err(|_| Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
        }
        workers::invalidate(&tx, &p.record)?;
        native_handoffs::touch(&tx, &p.record, self)?;
        for connection in [source.as_str(), target.connection.connection_id.as_str()] {
            if connection == source && old_retired {
                continue;
            }
            let value=tx.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces,n.readiness_json,n.observed_runtime_id FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[connection],|r|Ok(connect_protocol::events::ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?,stream_revision:crate::events::stream_revision(r.get(8)?,r.get(10)?,r.get(11)?)?})).map_err(|_| Error::storage())?;
            for recipient in &recipients {
                let mut data = serde_json::json!({"source_kind":"session","source_id":value.session_id,"session_id":value.session_id,"connection_id":value.connection_id,"participant_id":value.participant_id,"identity_id":target.identity_id,"connection":value});
                if recipient == &row.owner {
                    data["operation_id"] = serde_json::json!(row.record.operation_id);
                }
                tx.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,row.record.conversation_id,if connection==source || stopped {"connection_retired"}else{"connection_ready"},p.record,value.revision,data.to_string(),serde_json::json!({"kind":"connect-native-runtime","correlation_id":value.connection_id}).to_string()]).map_err(|_| Error::storage())?;
            }
        }
        let origin = if stopped {
            live.map(|capture| capture.activation().origin().clone())
        } else {
            Some(
                self.replacement_established_origin(
                    &tx,
                    &p.record,
                    live.ok_or_else(Error::forbidden)?
                        .attempt
                        .origin
                        .started
                        .attempt
                        .request
                        .clone(),
                    intent,
                    public_key,
                )?,
            )
        };
        if stopped {
            crate::session_participant_lifecycle::reconcile(
                &tx,
                self,
                &target.connection.conversation_id,
                &target.connection.session_id,
                None,
            )?;
        }
        // Acquire only after Store authority/SQL work. Keep this local entry
        // custody through commit; no await or authority acquisition follows it.
        let installation = observation.require(
            activation.map(AsRef::as_ref),
            &target.runtime_connection_id,
            target.connection.generation,
        )?;
        let commit: Box<dyn FnOnce() -> Result<()> + '_> = Box::new(|| {
            if !owner.current() {
                return Err(Error::forbidden());
            }
            timing.committing();
            tx.commit().map_err(|_| Error::storage())
        });
        if let Some(installation) = installation.as_ref() {
            installation.publish(commit)?;
        } else {
            commit()?;
        }
        timing.committed();
        drop(installation);
        Ok(origin)
    }
}

impl NativeReplacementCommitted {
    /// Best-effort fixed-size diagnostics, not readiness or current authority.
    /// Read only after Store work returns; concurrent snapshots are not atomic.
    pub fn projection_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"format":"connect/replacement-projection-diagnostics/1",
            "capture":self.capture_diagnostics.snapshot(),
            "activation":self.activation_diagnostics.snapshot()})
    }

    /// The identical metadata survives physical capture; it confers no permission.
    pub fn activation(&self) -> &Arc<crate::NativeReplacementActivation> {
        &self.attempt.activation
    }

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
