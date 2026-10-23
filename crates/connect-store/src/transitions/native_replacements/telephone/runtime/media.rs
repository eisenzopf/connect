//! Actual SIP activation provenance, with independently current route authority.
//! Neither a native peer projection nor an activation receipt grants media IO.
use super::*;
mod commit;
mod lifetime;
mod pair;
pub use lifetime::{
    TelephoneConnectionRuntimeObservation, TelephoneMediaOrigin, TelephoneMediaRead,
};
pub use pair::TelephoneNativeMediaRead;
mod continuation;
pub use commit::{
    TelephoneConnectionSnapshot, TelephoneReplacementCommitAttempt,
    TelephoneReplacementCommitUncertain, TelephoneReplacementCommitted,
};
pub use continuation::{
    TelephoneReplacementAudienceCapture, TelephoneReplacementPhase,
    TelephoneReplacementPhaseAttempt, TelephoneReplacementPhaseCapture,
    TelephoneReplacementSceneRead,
};

pub struct TelephoneReplacementMediaOrigin {
    submitted: Arc<TelephoneReplacementSubmitted>,
}
pub struct TelephoneReplacementMediaRead {
    read: TelephoneReplacementRead,
    origin: Arc<TelephoneReplacementMediaOrigin>,
    plan: serde_json::Value,
    interval: Arc<crate::AuthorityGuardSet>,
}
redacted!(
    TelephoneReplacementMediaOrigin,
    TelephoneReplacementMediaRead
);
impl TelephoneReplacementMediaRead {
    pub fn current(&self) -> bool {
        self.read.current()
    }
    pub fn source(&self) -> &NativeReplacementPreparationRead {
        self.read.source()
    }
    pub fn route_actor(&self) -> &Actor {
        self.read.route_actor()
    }
    pub fn target_member(&self) -> &connect_protocol::membership::MemberView {
        self.read.target_member()
    }
    pub fn destination(&self) -> Result<String> {
        self.read.destination()
    }
    pub fn call_id(&self) -> &str {
        self.origin
            .submitted
            .attempt
            .allocated
            .as_ref()
            .expect("validated SIP allocation")
            .call_id()
    }
    pub fn expires_at(&self) -> i64 {
        self.read.expires_at()
    }
    pub fn candidate(&self) -> Result<&str> {
        self.plan["candidate_runtime_id"]
            .as_str()
            .ok_or_else(invalid)
    }
    pub fn generation(&self) -> Result<u64> {
        self.plan["generation"].as_u64().ok_or_else(invalid)
    }
    /// The logical member's accepted Session and the new verified Endpoint
    /// constrain actual SIP negotiation. Native offer/codec fields are not used.
    pub fn allows_audio(&self, codec: &str) -> bool {
        let view = self.source().source();
        let equivalent = |name: &str| {
            let normalize = |n: &str| match n.to_ascii_lowercase().as_str() {
                "pcmu" | "g.711-mu" | "g711-mu" | "g711-u" => "g.711-mu".to_owned(),
                "pcma" | "g.711-a" | "g711-a" => "g.711-a".to_owned(),
                other => other.to_owned(),
            };
            normalize(name) == normalize(codec)
        };
        let contains = |v: &serde_json::Value| {
            v.as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str().is_some_and(equivalent)))
        };
        self.current()
            && self
                .source()
                .context()
                .target_capabilities
                .iter()
                .any(|v| v == "audio")
            && view.session().session.media.iter().any(|v| v == "audio")
            && contains(&view.accepted_capabilities()["audio"])
            && view
                .session()
                .session
                .capabilities_offer
                .as_ref()
                .is_some_and(|v| contains(&v["audio"]))
    }
}
fn submitted_current(c: &Connection, submitted: &TelephoneReplacementSubmitted) -> Result<()> {
    let a = &submitted.attempt;
    if a.phase != Phase::Submit {
        return Err(Error::forbidden());
    }
    allocated_current_from_submission(c, submitted)?;
    let exact: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json=?4 AND e.state='confirmed' AND e.observation_json=?5 AND e.correlation_key=?6 AND e.attempt=1 AND e.submission_generation=1)",
        params![a.prepared.record,a.attempt,a.effect,submitted.observation,a.descriptor,a.allocated.as_ref().ok_or_else(Error::forbidden)?.call_id()], |r|r.get(0),
    ).map_err(|_| Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
fn allocated_current_from_submission(
    c: &Connection,
    submitted: &TelephoneReplacementSubmitted,
) -> Result<()> {
    let allocated = submitted
        .attempt
        .allocated
        .as_ref()
        .ok_or_else(Error::forbidden)?;
    let a = &allocated.attempt;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='allocate' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.observation_json=?4 AND e.state='confirmed' AND e.observation_json=?5)",params![a.prepared.record,a.attempt,a.effect,allocated.observation,a.descriptor],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    /// A returned actual Core activation must already be durable. This origin
    /// carries immutable metadata only; every resource/packet use needs a read.
    pub fn telephone_replacement_media_origin(
        &self,
        submitted: Arc<TelephoneReplacementSubmitted>,
    ) -> Result<Arc<TelephoneReplacementMediaOrigin>> {
        let origin = Arc::new(TelephoneReplacementMediaOrigin { submitted });
        self.try_telephone_replacement_media_read(origin.clone())?;
        Ok(origin)
    }
    pub fn try_telephone_replacement_media_read(
        &self,
        origin: Arc<TelephoneReplacementMediaOrigin>,
    ) -> Result<TelephoneReplacementMediaRead> {
        let a = &origin.submitted.attempt;
        let p = &a.prepared;
        // Resolve the complete source/retained/operator/route footprint under
        // the same immediate SQL permit. Never wait under media engine fences.
        let c = self.try_connection()?;
        local_owner(self, &p.owner, &p.runtime)?;
        let scopes = p.selection.scopes(self, &c, false)?;
        let held = self.try_acquire_authority(&scopes)?;
        self.telephone_media_read_in(&c, origin.clone(), held, crate::now() + 5)
    }
    fn telephone_media_read_in(
        &self,
        c: &Connection,
        origin: Arc<TelephoneReplacementMediaOrigin>,
        held: Arc<crate::AuthorityGuardSet>,
        until: i64,
    ) -> Result<TelephoneReplacementMediaRead> {
        let a = &origin.submitted.attempt;
        let p = &a.prepared;
        let expires: i64 = c
            .query_row(
                "SELECT expires_at FROM native_replacements WHERE operation_record_id=?1",
                [&p.record],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let mut source =
            p.selection
                .read_with_connection(self, c, held.clone(), expires.min(until))?;
        let (route_actor, target, descriptor) =
            facts(self, c, p, &source, &a.route_credential, &a.policy)?;
        if descriptor != a.descriptor {
            return Err(conflict());
        }
        source.limit(route_actor.expires_at)?;
        submitted_current(c, &origin.submitted)?;
        let plan = allocation::current_prepared(self, c, p)?;
        let read = TelephoneReplacementRead {
            prepared: p.clone(),
            source,
            route_actor,
            target,
            policy: a.policy.clone(),
            descriptor,
            phase: Phase::Submit,
            allocated: a.allocated.clone(),
        };
        Ok(TelephoneReplacementMediaRead {
            read,
            origin,
            plan,
            interval: held,
        })
    }
}

/// Actual negotiated SIP resources, still muted and not duplex/confirmation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelephoneAudioResources {
    pub candidate: String,
    pub generation: u64,
    pub call_id: String,
    pub stream_id: String,
    pub stream_local_id: u32,
    pub stream_generation: u64,
    pub stream_revision: u64,
    pub codec: String,
    pub clock_rate_hz: u32,
    pub channels: u8,
    pub fmtp: Option<String>,
    pub payload_type: u8,
    pub prepared_muted: bool,
    pub media_ready: bool,
}
pub struct TelephoneReplacementResourcesAttempt {
    origin: Arc<TelephoneReplacementMediaOrigin>,
    attempt: String,
    effect: String,
    descriptor: String,
    candidate: String,
    generation: u64,
    accepted: serde_json::Value,
    offered: serde_json::Value,
}
pub struct TelephoneReplacementResourcesCapture {
    attempt: Arc<TelephoneReplacementResourcesAttempt>,
    observation: Option<String>,
}
redacted!(
    TelephoneAudioResources,
    TelephoneReplacementResourcesAttempt,
    TelephoneReplacementResourcesCapture
);
impl TelephoneReplacementResourcesAttempt {
    pub fn prepared(
        self: &Arc<Self>,
        observation: TelephoneAudioResources,
    ) -> Result<TelephoneReplacementResourcesCapture> {
        id(&observation.stream_id)?;
        if observation.candidate != self.candidate
            || observation.generation != self.generation
            || observation.call_id
                != self
                    .origin
                    .submitted
                    .attempt
                    .allocated
                    .as_ref()
                    .ok_or_else(invalid)?
                    .call_id()
            || observation.stream_revision != 2
            || observation.stream_generation == 0
            || observation.stream_generation > 9_007_199_254_740_991
            || observation.stream_local_id == 0
            || !(1..=192000).contains(&observation.clock_rate_hz)
            || !(1..=8).contains(&observation.channels)
            || observation.payload_type > 127
            || observation.codec.is_empty()
            || observation.codec.len() > 64
            || !observation
                .codec
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
            || observation
                .fmtp
                .as_ref()
                .is_some_and(|s| s.len() > 2048 || s.chars().any(char::is_control))
            || !observation.prepared_muted
            || observation.media_ready
            || !audio_contains(&self.accepted["audio"], &observation.codec)
            || !audio_contains(&self.offered["audio"], &observation.codec)
        {
            return Err(conflict());
        }
        Ok(TelephoneReplacementResourcesCapture {
            attempt: self.clone(),
            observation: Some(crate::provisioning::canonical_json(
                &serde_json::to_value(observation).map_err(|_| invalid())?,
            )?),
        })
    }
    pub fn uncertain(self: &Arc<Self>) -> TelephoneReplacementResourcesCapture {
        TelephoneReplacementResourcesCapture {
            attempt: self.clone(),
            observation: None,
        }
    }
}
fn audio_contains(value: &serde_json::Value, codec: &str) -> bool {
    let normalize = |n: &str| match n.to_ascii_lowercase().as_str() {
        "pcmu" | "g.711-mu" | "g711-mu" | "g711-u" => "g.711-mu".into(),
        "pcma" | "g.711-a" | "g711-a" => "g.711-a".into(),
        other => other.to_owned(),
    };
    value.as_array().is_some_and(|a| {
        a.iter()
            .any(|v| v.as_str().is_some_and(|s| normalize(s) == normalize(codec)))
    })
}
impl TelephoneReplacementMediaRead {
    /// After this exact attempt is durable, a held Core resource invocation may
    /// use its current read. This does not authorize output or preparation IO.
    pub fn requires_resources_attempt(
        &self,
        attempt: &TelephoneReplacementResourcesAttempt,
    ) -> Result<()> {
        if !self.current()
            || !Arc::ptr_eq(&self.origin, &attempt.origin)
            || self.candidate()? != attempt.candidate
            || self.generation()? != attempt.generation
            || self.read.descriptor != attempt.descriptor
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl Store {
    pub fn begin_telephone_replacement_resources(
        &self,
        read: &TelephoneReplacementMediaRead,
    ) -> Result<Arc<TelephoneReplacementResourcesAttempt>> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let candidate = read.candidate()?.to_owned();
        let generation = read.generation()?;
        let view = read.source().source();
        let accepted = view.accepted_capabilities().clone();
        let offered = view
            .session()
            .session
            .capabilities_offer
            .clone()
            .ok_or_else(invalid)?;
        let origin = &read.origin;
        let p = &origin.submitted.attempt.prepared;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (_, _, descriptor) = facts(
            self,
            &tx,
            p,
            &read.read.source,
            &origin.submitted.attempt.route_credential,
            &origin.submitted.attempt.policy,
        )?;
        if descriptor != read.read.descriptor {
            return Err(conflict());
        }
        submitted_current(&tx, &origin.submitted)?;
        let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM telephone_replacement_resources WHERE operation_record_id=?1)",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if exists {
            return Err(Error::new(
                "reconciliation-required",
                "Telephone resources already attempted",
            ));
        }
        let attempt = format!("trm_{}", uuid::Uuid::new_v4());
        let effect = format!("tre_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,observation_json) VALUES(?1,?2,'telephone-replacement-resources',?3,'none','prepared',?4)",params![effect,p.record,read.call_id(),descriptor]).map_err(|_|Error::storage())?;
        tx.execute("INSERT INTO telephone_replacement_resources(operation_record_id,runtime_epoch,attempt_id,effect_id,descriptor_json) VALUES(?1,?2,?3,?4,?5)",params![p.record,p.runtime,attempt,effect,descriptor]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='submitting',attempt=1,submission_generation=1 WHERE id=?1",[&effect]).map_err(|_|Error::storage())?;
        native_handoffs::touch(&tx, &p.record, self)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Arc::new(TelephoneReplacementResourcesAttempt {
            origin: origin.clone(),
            attempt,
            effect,
            descriptor,
            candidate,
            generation,
            accepted,
            offered,
        }))
    }
    pub fn record_telephone_replacement_resources(
        &self,
        capture: &TelephoneReplacementResourcesCapture,
    ) -> Result<OperationRecord> {
        let a = &capture.attempt;
        let p = &a.origin.submitted.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
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
        let old:Option<(Option<String>,String)>=tx.query_row("SELECT r.observation_json,e.state FROM telephone_replacement_resources r JOIN operation_effects e ON e.id=r.effect_id WHERE r.operation_record_id=?1 AND r.runtime_epoch=?2 AND r.attempt_id=?3 AND r.effect_id=?4 AND r.descriptor_json=?5 AND e.observation_json=?5 AND e.attempt=1 AND e.submission_generation=1",params![p.record,p.runtime,a.attempt,a.effect,a.descriptor],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let (old, state) = old.ok_or_else(Error::forbidden)?;
        match &capture.observation {
            Some(value) => {
                if let Some(old) = old {
                    if old != *value || state != "confirmed" {
                        return Err(conflict());
                    }
                    return Ok(row.record);
                }
                if !matches!(state.as_str(), "submitting" | "unknown") {
                    return Err(conflict());
                }
                tx.execute("UPDATE telephone_replacement_resources SET observation_json=?2 WHERE operation_record_id=?1",params![p.record,value]).map_err(|_|Error::storage())?;
                tx.execute(
                    "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
                    [&a.effect],
                )
                .map_err(|_| Error::storage())?;
            }
            None => {
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
                tx.execute("UPDATE operations SET state='unknown' WHERE record_id=?1 AND state='preparing'",[&p.record]).map_err(|_|Error::storage())?;
                workers::invalidate(&tx, &p.record)?;
            }
        }
        native_handoffs::touch(&tx, &p.record, self)?;
        let record = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}

impl Store {
    /// Retire the host's continuation of this already known SIP attempt. This
    /// asserts no remote absence or failed call: uncertainty remains explicit,
    /// the exact call reference and confirmed resource facts remain retained,
    /// and replay can never allocate or dial again. No current IO is granted.
    pub fn abandon_telephone_replacement(
        &self,
        submitted: &TelephoneReplacementSubmitted,
    ) -> Result<OperationRecord> {
        let p = &submitted.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
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
        submitted_current(&tx, submitted)?;
        if row.record.state == OperationState::Preparing {
            tx.execute(
                "UPDATE operations SET state='unknown' WHERE record_id=?1",
                [&p.record],
            )
            .map_err(|_| Error::storage())?;
            workers::invalidate(&tx, &p.record)?;
            native_handoffs::touch(&tx, &p.record, self)?;
        }
        let record = rows::by_record(&tx, &p.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(record)
    }
}
