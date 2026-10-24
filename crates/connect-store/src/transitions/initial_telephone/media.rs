//! Real answer facts, own-identity acceptance and fresh guarded media facts.
use super::*;
mod private_admission;
mod private_answer;
mod private_facts;
mod private_phase;
pub use private_phase::PrivateInitialTelephonePhase;
mod routing;
use serde_json::{json, Value};
macro_rules! media_redacted {
    ($($name:ident),+ $(,)?) => {$(impl std::fmt::Debug for $name {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(concat!(stringify!($name), "(redacted)"))
        }
    })+};
}
mod signal;
pub use signal::{
    InitialTelephoneNativeSignalRead, InitialTelephoneSignalDirection, InitialTelephoneSignalRead,
};

pub struct InitialTelephoneAnswerCapture {
    signal: Arc<InitialTelephoneSignaling>,
    codec: String,
    observation: String,
}
#[derive(Clone)]
pub struct InitialTelephoneMediaOrigin {
    signal: Arc<InitialTelephoneSignaling>,
    codec: String,
    observation: String,
}
pub struct InitialTelephoneAnswerRecorded {
    pub media: Option<Arc<InitialTelephoneMediaOrigin>>,
    pub problem: Option<String>,
}
pub struct InitialTelephoneMediaRead {
    origin: Arc<InitialTelephoneMediaOrigin>,
    owner: Arc<InstallationOwnership>,
    _interval: Arc<AuthorityGuardSet>,
    endpoint_actor: Actor,
    route_actor: Actor,
    member: MemberView,
    session: session_control::SessionView,
    role: String,
    accepted: Value,
    conversation_revision: u64,
    expires: i64,
    connected: bool,
    purpose: ProjectionPurpose,
}
struct InitialTelephoneProjection {
    endpoint_actor: Actor,
    route_actor: Actor,
    member: MemberView,
    session: session_control::SessionView,
    role: String,
    accepted: Value,
    conversation_revision: u64,
    expires: i64,
    connected: bool,
}
#[derive(Clone, Copy)]
enum ProjectionPurpose {
    Media,
    Signal,
    PrivateAdmission,
    PrivateSelection,
    PrivateCurrent,
    PrivateReturn,
}
media_redacted!(
    InitialTelephoneAnswerCapture,
    InitialTelephoneMediaOrigin,
    InitialTelephoneAnswerRecorded,
    InitialTelephoneMediaRead
);
fn codec_name(s: &str) -> String {
    match s.to_ascii_lowercase().as_str() {
        "pcmu" => "g.711-mu".into(),
        "pcma" => "g.711-a".into(),
        s => s.to_owned(),
    }
}
impl InitialTelephoneSignaling {
    /// Trusted installed adapter composition only, after checking its opaque
    /// actual answer against this exact route. Capturing grants no new IO.
    pub fn capture_answer(
        self: &Arc<Self>,
        call_id: &str,
        codec: &str,
    ) -> Result<InitialTelephoneAnswerCapture> {
        if call_id != self.sip_call_id()
            || codec.is_empty()
            || codec.len() > 64
            || codec.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(Error::forbidden());
        }
        let codec = codec_name(codec);
        let observation = crate::provisioning::canonical_json(
            &json!({"kind":"answered","runtime_connection_id":self.runtime_connection_id(),"call_id":call_id,"codec":codec}),
        )?;
        Ok(InitialTelephoneAnswerCapture {
            signal: self.clone(),
            codec,
            observation,
        })
    }
}
impl InitialTelephoneMediaOrigin {
    pub fn source(&self) -> &InitialTelephoneSource {
        self.signal.origin().source()
    }
    pub fn record_id(&self) -> &str {
        self.signal.origin().record_id()
    }
    pub fn connection_id(&self) -> &str {
        self.signal.origin().connection_id()
    }
    pub fn runtime_connection_id(&self) -> &str {
        self.signal.runtime_connection_id()
    }
    pub fn sip_call_id(&self) -> &str {
        self.signal.sip_call_id()
    }
    pub fn codec(&self) -> &str {
        &self.codec
    }
}
impl InitialTelephoneMediaRead {
    pub fn origin(&self) -> &InitialTelephoneMediaOrigin {
        &self.origin
    }
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires
    }
    pub fn source(&self) -> &InitialTelephoneSource {
        self.origin.source()
    }
    pub fn endpoint_actor(&self) -> &Actor {
        &self.endpoint_actor
    }
    pub fn route_actor(&self) -> &Actor {
        &self.route_actor
    }
    pub fn member(&self) -> &MemberView {
        &self.member
    }
    pub fn session(&self) -> &session_control::SessionView {
        &self.session
    }
    pub fn role(&self) -> &str {
        &self.role
    }
    pub fn conversation_revision(&self) -> u64 {
        self.conversation_revision
    }
    pub fn accepted_capabilities(&self) -> &Value {
        &self.accepted
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub fn is_connected(&self) -> bool {
        self.connected
    }
    pub fn is_private_admission(&self) -> bool {
        matches!(self.purpose, ProjectionPurpose::PrivateAdmission)
    }
    pub fn is_private_audio(&self) -> bool {
        self.origin.source().is_private_candidate()
    }
    pub fn may_receive(&self) -> bool {
        !matches!(self.purpose, ProjectionPurpose::PrivateReturn)
            && self.endpoint_actor.permits("stream.subscribe")
    }
}
fn current_scopes(
    c: &Connection,
    origin: &InitialTelephoneOrigin,
    write: bool,
) -> Result<Vec<AuthorityScope>> {
    let mut result = crate::authority_admission::conversation_scopes(
        c,
        &origin.source.conversation_id,
        if write {
            AuthorityMode::Write
        } else {
            AuthorityMode::Read
        },
    )?;
    result.push(AuthorityScope::identity(
        &origin.source.route_owner_id,
        AuthorityMode::Read,
    )?);
    result.push(AuthorityScope::identity(
        &origin.source.caller.identity_id,
        if write {
            AuthorityMode::Write
        } else {
            AuthorityMode::Read
        },
    )?);
    AuthorityScope::merged(&result)
}
fn historical(
    store: &Store,
    c: &Connection,
    signal: &InitialTelephoneSignaling,
) -> Result<rows::Saved> {
    historical_lifetime(store, c, signal)?;
    rows::by_record(c, &signal.origin().record)
}
// Packet validation needs exact lifetime and current state, never operation
// display metadata or its complete visible-journal/causal-event projection.
fn historical_lifetime(
    store: &Store,
    c: &Connection,
    signal: &InitialTelephoneSignaling,
) -> Result<()> {
    let origin = signal.origin();
    origin.lifetime(store)?;
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM initial_telephone_operations n WHERE n.operation_record_id=?1 AND n.signaling_started=1 AND n.attempt_runtime=?2 AND n.runtime_connection_id=?3 AND n.sip_call_id=?4)",params![origin.record,origin.runtime,signal.runtime_connection_id(),signal.sip_call_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    /// Persist an actual captured answer even if its old command authority was
    /// withdrawn after IO. Current own-identity acceptance is a separate result.
    pub fn record_initial_telephone_answer(
        &self,
        capture: &InitialTelephoneAnswerCapture,
    ) -> Result<InitialTelephoneAnswerRecorded> {
        self.record_initial_telephone_answer_with_mode(capture, false)
    }
    /// Persist an actual answer and its own current consent for the original
    /// private SIP reservation. This does not accept public Session presence,
    /// change its codecs, publish media, or assert private readiness.
    pub fn record_private_initial_telephone_answer(
        &self,
        capture: &InitialTelephoneAnswerCapture,
    ) -> Result<InitialTelephoneAnswerRecorded> {
        if !capture.signal.origin().source.is_private_candidate() {
            return Err(Error::forbidden());
        }
        self.record_initial_telephone_answer_with_mode(capture, true)
    }
    fn record_initial_telephone_answer_with_mode(
        &self,
        capture: &InitialTelephoneAnswerCapture,
        private: bool,
    ) -> Result<InitialTelephoneAnswerRecorded> {
        let signal = &capture.signal;
        let origin = signal.origin();
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                historical(self, &c, signal)?;
                current_scopes(&c, origin, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = historical(self, &tx, signal)?;
        if !self.authority_covers(&held, &current_scopes(&tx, origin, true)?) {
            return Err(Error::forbidden());
        }
        let old:Option<(String,bool)>=tx.query_row("SELECT observation_json,accepted FROM initial_telephone_answers WHERE operation_record_id=?1",[&origin.record],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        let media = Arc::new(InitialTelephoneMediaOrigin {
            signal: signal.clone(),
            codec: capture.codec.clone(),
            observation: capture.observation.clone(),
        });
        if let Some((saved, accepted)) = old {
            if saved != capture.observation {
                return Err(conflict());
            }
            if origin.source.is_private_candidate() && !private {
                return Ok(InitialTelephoneAnswerRecorded {
                    media: None,
                    problem: Some("private-context-required".into()),
                });
            }
            if private && accepted {
                private_answer::require(self, &tx, capture, &held)?;
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(InitialTelephoneAnswerRecorded {
                media: accepted.then_some(media),
                problem: (!accepted).then(|| {
                    if origin.source.is_private_candidate() {
                        "private-context-required".into()
                    } else {
                        "authority-unavailable".into()
                    }
                }),
            });
        }
        tx.execute("INSERT INTO initial_telephone_answers(operation_record_id,observation_json,codec,observed_at) VALUES(?1,?2,?3,?4)",params![origin.record,capture.observation,capture.codec,crate::now()]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE operation_effects SET state='confirmed',observation_json=?2 WHERE id=(SELECT effect_id FROM initial_telephone_operations WHERE operation_record_id=?1) AND state IN ('submitting','unknown')",params![origin.record,capture.observation]).map_err(|_|Error::storage())?;
        if origin.source.is_private_candidate() && !private {
            // Preserve the actual answer, but this public own-acceptance path
            // cannot publish private presence or manufacture a media grant.
            if !row.record.state.terminal() && row.record.state != OperationState::Unknown {
                rows::change(&tx, &row, OperationState::Unknown, None, self)?;
            }
            tx.execute("UPDATE connections SET state='unknown',revision=revision+1 WHERE id=?1 AND state='preparing'",[&origin.connection]).map_err(|_|Error::storage())?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(InitialTelephoneAnswerRecorded {
                media: None,
                problem: Some("private-context-required".into()),
            });
        }
        // The remote credential acts only for its OWN invited Participant.
        // The protected observed answer supplies consent; no operator accept or
        // metadata-only readiness is substituted for the real media commit.
        tx.execute_batch("SAVEPOINT initial_answer_accept")
            .map_err(|_| Error::storage())?;
        let accepted = (|| {
            if private {
                return private_answer::accept(self, &tx, capture, &held);
            }
            let (endpoint, _, _) = origin.current(self, &tx)?;
            endpoint.require("session.accept")?;
            let session = crate::session_control::view(
                &tx,
                &origin.source.conversation_id,
                &origin.source.session_id,
            )?;
            let request = session_control::Request {
                operation_id: format!("answer_{}", origin.record),
                conversation_id: origin.source.conversation_id.clone(),
                session_id: origin.source.session_id.clone(),
                by_participant: Some(origin.source.target.participant_id.clone()),
                expected_revision: Some(session.topology_revision),
                action: session_control::Action::Accept {
                    capabilities: json!({"audio":[capture.codec]}),
                },
            };
            crate::operations::execute(
                &tx,
                &endpoint,
                &Operation::SessionControl { request },
                &crate::DiscoveryRuntime::default(),
                self,
                Some(&held),
            )?
            .outcome?;
            Ok::<(), Error>(())
        })();
        let problem = if let Err(error) = accepted {
            tx.execute_batch("ROLLBACK TO initial_answer_accept; RELEASE initial_answer_accept")
                .map_err(|_| Error::storage())?;
            if !row.record.state.terminal() && row.record.state != OperationState::Unknown {
                rows::change(&tx, &row, OperationState::Unknown, None, self)?;
            }
            tx.execute("UPDATE connections SET state='unknown',revision=revision+1 WHERE id=?1 AND state='preparing'",[&origin.connection]).map_err(|_|Error::storage())?;
            Some(error.code)
        } else {
            tx.execute_batch("RELEASE initial_answer_accept")
                .map_err(|_| Error::storage())?;
            let (acceptance, kind) = if private {
                (origin.record.clone(), "private-connection")
            } else {
                let acceptance:String=tx.query_row("SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id=?2 AND state='completed'",params![origin.source.target.identity_id,format!("answer_{}",origin.record)],|r|r.get(0)).map_err(|_|Error::storage())?;
                (acceptance, "public-session")
            };
            tx.execute(
                "UPDATE initial_telephone_answers SET accepted=1,acceptance_record_id=?2,acceptance_kind=?3 WHERE operation_record_id=?1",
                params![origin.record, acceptance, kind],
            )
            .map_err(|_| Error::storage())?;
            super::super::effects::touch(&tx, &origin.record, self)?;
            None
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(InitialTelephoneAnswerRecorded {
            media: problem.is_none().then_some(media),
            problem,
        })
    }
    pub fn try_initial_telephone_media_read(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
    ) -> Result<InitialTelephoneMediaRead> {
        let required = {
            let c = self.try_connection()?;
            historical_lifetime(self, &c, &origin.signal)?;
            current_scopes(&c, origin.signal.origin(), false)?
        };
        let held = self.try_acquire_authority(&required)?;
        let c = self.try_connection()?;
        self.initial_telephone_media_in(&c, origin, held, crate::now() + 30)
    }
    /// Only the original candidate's bounded, quarantined admission phase.
    /// Established private routing uses its separately sealed scene authority.
    pub fn try_private_initial_telephone_admission_read(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
    ) -> Result<InitialTelephoneMediaRead> {
        let required = {
            let c = self.try_connection()?;
            private_admission::scopes(self, &c, &origin)?
        };
        let held = self.try_acquire_authority(&required)?;
        let c = self.try_connection()?;
        self.initial_telephone_read_with_purpose(
            &c,
            origin,
            held,
            crate::now() + 30,
            ProjectionPurpose::PrivateAdmission,
        )
    }
    /// Current metadata for an unchanged initial SIP outsider in a native
    /// audience transaction. It grants no private receipt or native identity.
    pub(crate) fn initial_telephone_audience_facts(
        &self,
        c: &Connection,
        origin: &InitialTelephoneMediaOrigin,
    ) -> Result<(Vec<AuthorityScope>, Value, i64)> {
        self.initial_telephone_audience_facts_for(
            c,
            origin,
            crate::connection_control::private_consultation::Purpose::Media,
        )
    }
    pub(crate) fn initial_telephone_audience_facts_for(
        &self,
        c: &Connection,
        origin: &InitialTelephoneMediaOrigin,
        purpose: crate::connection_control::private_consultation::Purpose,
    ) -> Result<(Vec<AuthorityScope>, Value, i64)> {
        if origin.source().is_private_candidate() {
            return private_facts::facts_for(self, c, origin, purpose);
        }
        historical_lifetime(self, c, &origin.signal)?;
        let scopes = current_scopes(c, origin.signal.origin(), false)?;
        let p = self.initial_telephone_projection(
            c,
            origin,
            crate::now() + 5,
            if purpose.media() {
                ProjectionPurpose::Media
            } else {
                ProjectionPurpose::Signal
            },
        )?;
        let current: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND generation=1 AND observed_runtime_id=?2 AND state IN ('ready','active') AND readiness_json IS NOT NULL)",params![origin.connection_id(),self.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !p.connected || !current {
            return Err(Error::forbidden());
        }
        let source = origin.source();
        let value = json!({
            "source_kind":"initial-sip", "record_id":origin.record_id(),
            "connection_id":origin.connection_id(), "runtime_connection_id":origin.runtime_connection_id(),
            "sip_call_id":origin.sip_call_id(), "generation":1,
            "participant_id":p.member.participant_id, "membership_epoch":p.member.membership_epoch,
            "identity_id":p.member.identity_id, "endpoint_id":source.endpoint_id,
            "endpoint_revision":source.endpoint_revision, "device_id":source.device_id,
            "endpoint_credential_id":p.endpoint_actor.credential_id,
            "route_credential_id":p.route_actor.credential_id, "route_owner_id":source.route_owner_id,
            "physical_expires_at":p.endpoint_actor.expires_at.min(p.route_actor.expires_at)
        });
        Ok((scopes, value, p.expires))
    }
    /// Captured Core outcome only. Historical custody never reauthorizes IO,
    /// revives a retired call, or replaces a newer observed stream revision.
    pub(crate) fn record_initial_telephone_audience_revision(
        &self,
        c: &Connection,
        binding: &Value,
        runtime: &str,
        previous: u64,
        revision: u64,
    ) -> Result<()> {
        if !Self::initial_sip_snapshot_kind_matches(c, binding)?
            || binding["generation"] != 1
            || previous.checked_add(1) != Some(revision)
            || revision >= 9_007_199_254_740_991
        {
            return Err(Error::storage());
        }
        let id = binding["connection_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        let parent = binding["record_id"].as_str().ok_or_else(Error::storage)?;
        let core_id = binding["runtime_connection_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        let call = binding["sip_call_id"].as_str().ok_or_else(Error::storage)?;
        c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?5) WHERE id=?1 AND generation=1 AND observed_runtime_id=?2 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?6 AND EXISTS(SELECT 1 FROM initial_telephone_operations p JOIN operations o ON o.record_id=p.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=p.operation_record_id WHERE p.operation_record_id=?3 AND o.connection_id=connections.id AND p.attempt_runtime=?2 AND p.runtime_connection_id=?4 AND p.sip_call_id=?7 AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL)", params![id,runtime,parent,core_id,revision,previous,call]).map_err(|_|Error::storage())?;
        Ok(())
    }
    pub(crate) fn initial_telephone_media_in(
        &self,
        c: &Connection,
        origin: Arc<InitialTelephoneMediaOrigin>,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<InitialTelephoneMediaRead> {
        self.initial_telephone_read_with_purpose(
            c,
            origin,
            interval,
            until,
            ProjectionPurpose::Media,
        )
    }
    fn initial_telephone_read_with_purpose(
        &self,
        c: &Connection,
        origin: Arc<InitialTelephoneMediaOrigin>,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
        purpose: ProjectionPurpose,
    ) -> Result<InitialTelephoneMediaRead> {
        let scopes = if matches!(purpose, ProjectionPurpose::PrivateCurrent) {
            private_facts::current_authority(self, c, &origin)?.0
        } else if matches!(purpose, ProjectionPurpose::PrivateAdmission) {
            private_admission::scopes(self, c, &origin)?
        } else {
            current_scopes(c, origin.signal.origin(), false)?
        };
        if !self.authority_covers(&interval, &scopes) {
            return Err(Error::forbidden());
        }
        let p = self.initial_telephone_projection(c, &origin, until, purpose)?;
        let owner = self.require_runtime_owner()?;
        if p.connected && matches!(purpose, ProjectionPurpose::Media) {
            self.remember_session_ready(
                c,
                crate::session_lifecycle::ReadyOrigin::InitialTelephone(origin.clone()),
            )?;
        }
        Ok(InitialTelephoneMediaRead {
            origin,
            owner,
            _interval: interval,
            endpoint_actor: p.endpoint_actor,
            route_actor: p.route_actor,
            member: p.member,
            session: p.session,
            role: p.role,
            accepted: p.accepted,
            conversation_revision: p.conversation_revision,
            expires: p.expires,
            connected: p.connected,
            purpose,
        })
    }
    fn initial_telephone_projection(
        &self,
        c: &Connection,
        origin: &InitialTelephoneMediaOrigin,
        until: i64,
        purpose: ProjectionPurpose,
    ) -> Result<InitialTelephoneProjection> {
        historical_lifetime(self, c, &origin.signal)?;
        let original = origin.signal.origin();
        let s = &original.source;
        if matches!(purpose, ProjectionPurpose::PrivateAdmission) {
            return private_admission::projection(self, c, origin, until);
        }
        if matches!(purpose, ProjectionPurpose::PrivateCurrent) {
            return private_facts::current_projection(self, c, origin, until);
        }
        if matches!(purpose, ProjectionPurpose::PrivateReturn) {
            return private_facts::return_projection(self, c, origin, until);
        }
        if matches!(purpose, ProjectionPurpose::PrivateSelection) {
            return private_facts::selection_projection(self, c, origin, until);
        }
        if s.is_private_candidate() {
            return Err(Error::forbidden());
        }
        let (state, cancel_requested): (String, bool) = c
            .query_row(
                "SELECT state,cancel_requested FROM operations WHERE record_id=?1",
                [&original.record],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| Error::storage())?
            .ok_or_else(Error::forbidden)?;
        let (saved,accepted,connected,retired):(String,bool,bool,bool)=c.query_row("SELECT observation_json,accepted,connected_at IS NOT NULL,retired_at IS NOT NULL FROM initial_telephone_answers WHERE operation_record_id=?1",[&original.record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::forbidden())?;
        if saved != origin.observation
            || !accepted
            || retired
            || cancel_requested
            || (!connected && crate::now() >= original.expires_at)
            || !matches!(state.as_str(), "preparing" | "completed")
        {
            return Err(Error::forbidden());
        }
        let endpoint = actor_from_connection(c, &original.policy.endpoint_credential_id)?;
        let route = actor_from_connection(c, &original.policy.route_credential_id)?;
        if matches!(purpose, ProjectionPurpose::Media) {
            endpoint.require("connection.offer")?;
        }
        route.require("sip_dial")?;
        if endpoint.identity_id != s.target.identity_id || route.identity_id != s.route_owner_id {
            return Err(Error::forbidden());
        }
        let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections n JOIN endpoints e ON e.id=n.endpoint_id JOIN devices d ON d.id=e.device_id AND d.revoked=0 WHERE n.id=?1 AND n.endpoint_id=?2 AND n.generation=1 AND n.state IN ('preparing','ready','active') AND e.revision=?3 AND e.verification_state='verified' AND e.secret_ref=?4 AND e.route_owner_id=?5 AND d.identity_id=e.identity_id)",params![original.connection,s.endpoint_id,s.endpoint_revision,original.policy.route_reference,s.route_owner_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !valid {
            return Err(Error::forbidden());
        }
        let member = crate::audience::member(c, &endpoint.identity_id, &s.conversation_id, true)?;
        if member.participant_id != s.target.participant_id
            || member.membership_epoch != s.target.membership_epoch
        {
            return Err(Error::forbidden());
        }
        crate::audience::require_original(
            c,
            &endpoint,
            &s.conversation_id,
            connect_protocol::membership::ContentKind::Session,
            &s.session_id,
        )?;
        let session = crate::session_control::view(c, &s.conversation_id, &s.session_id)?;
        if !matches!(session.state.as_str(), "accepted" | "ready" | "active") {
            return Err(Error::forbidden());
        }
        let (role,raw,conversation_revision):(String,String,u64)=c.query_row("SELECT p.role,cap.capabilities_json,v.revision FROM session_participants p JOIN session_presence_capabilities cap ON cap.session_id=p.session_id AND cap.participant_id=p.participant_id AND cap.membership_epoch=p.membership_epoch JOIN conversations v ON v.id=p.conversation_id WHERE p.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND p.state='present' AND v.closed=0",params![s.session_id,s.target.participant_id,s.target.membership_epoch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::forbidden())?;
        let accepted: Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        let allows = |v: &Value| {
            v["audio"].as_array().is_some_and(|v| {
                v.iter()
                    .any(|n| n.as_str().is_some_and(|n| codec_name(n) == origin.codec))
            })
        };
        if matches!(purpose, ProjectionPurpose::Media)
            && (!allows(&accepted) || !session.capabilities_offer.as_ref().is_some_and(allows))
        {
            return Err(Error::forbidden());
        }
        for identity in &if matches!(purpose, ProjectionPurpose::Media) {
            crate::audience::source_members(
                c,
                &s.conversation_id,
                connect_protocol::membership::ContentKind::Session,
                &s.session_id,
            )?
        } else {
            Vec::new()
        } {
            if !endpoint.permits_recipient(identity) || !route.permits_recipient(identity) {
                return Err(Error::forbidden());
            }
        }
        let expires = until
            .min(endpoint.expires_at)
            .min(route.expires_at)
            .min(if connected {
                i64::MAX
            } else {
                original.expires_at
            });
        if expires <= crate::now() {
            return Err(Error::forbidden());
        }
        Ok(InitialTelephoneProjection {
            endpoint_actor: endpoint,
            route_actor: route,
            member,
            session,
            role,
            accepted,
            conversation_revision,
            expires,
            connected,
        })
    }
    pub(crate) fn initial_telephone_session_readiness(
        &self,
        c: &Connection,
        origin: &InitialTelephoneMediaOrigin,
    ) -> Result<crate::session_lifecycle::ReadyConnection> {
        let p = self.initial_telephone_projection(c, origin, i64::MAX, ProjectionPurpose::Media)?;
        if !p.connected {
            return Err(Error::forbidden());
        }
        let observed:i64=c.query_row("SELECT a.connected_at FROM initial_telephone_answers a JOIN operations o ON o.record_id=a.operation_record_id JOIN connections n ON n.id=o.connection_id WHERE a.operation_record_id=?1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL AND n.id=?2 AND n.generation=1 AND n.state IN ('ready','active') AND n.observed_runtime_id=?3 AND n.readiness_json IS NOT NULL",params![origin.record_id(),origin.connection_id(),self.runtime_epoch()],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        Ok(crate::session_lifecycle::ReadyConnection {
            connection: connect_protocol::events::SessionActiveConnection {
                connid: origin.connection_id().into(),
                participant_id: p.member.participant_id,
                transport: "sip-interop".into(),
            },
            epoch: p.member.membership_epoch,
            observed_at_millis: observed * 1000,
            expires_at: p.expires,
            capabilities: json!({"audio_codecs":[{"name":origin.codec,"params":{"sample_rate":8000,"channels":1,"fmtp":null}}]}),
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct InitialTelephoneReadyObservation {
    pub runtime_connection_id: String,
    pub call_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub generation: u64,
    pub stream_revision: u64,
    pub participant_revision: u64,
    pub streams: Vec<String>,
}
/// Only the installed service derives this from its actual committed Core
/// binding and guarded controller. It must not enter Store or perform IO.
pub trait InitialTelephoneRuntimeObservation: Send + Sync {
    fn observe(&self) -> Result<InitialTelephoneReadyObservation>;
}
/// Installed service composition only: attest actual private Core admission,
/// committed guarded ports and the original muted controller at revision two.
/// This observation supplies no continuing private-scene authority.
pub trait PrivateInitialTelephoneRuntimeObservation: Send + Sync {
    fn observe_private(&self) -> Result<InitialTelephoneReadyObservation>;
}
media_redacted!(InitialTelephoneReadyObservation);
impl Store {
    pub fn record_initial_telephone_connected(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
        observation: &dyn InitialTelephoneRuntimeObservation,
    ) -> Result<OperationRecord> {
        if origin.source().is_private_candidate() {
            return Err(Error::forbidden());
        }
        self.record_initial_telephone_connected_inner(origin, &|| observation.observe(), false)
    }
    pub fn record_private_initial_telephone_connected(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
        observation: &dyn PrivateInitialTelephoneRuntimeObservation,
    ) -> Result<OperationRecord> {
        if !origin.source().is_private_candidate() {
            return Err(Error::forbidden());
        }
        let record = self.record_initial_telephone_connected_inner(
            origin.clone(),
            &|| observation.observe_private(),
            true,
        )?;
        // Only committed private readiness can seed the separate metadata cache.
        let c = self.connection()?;
        private_facts::remember(self, &c, origin)?;
        Ok(record)
    }
    fn record_initial_telephone_connected_inner(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
        observe: &dyn Fn() -> Result<InitialTelephoneReadyObservation>,
        private: bool,
    ) -> Result<OperationRecord> {
        let value = observe()?;
        let original = origin.signal.origin();
        let s = origin.source();
        if value.runtime_connection_id != origin.runtime_connection_id()
            || value.call_id != origin.sip_call_id()
            || value.participant_id != s.target.participant_id
            || value.membership_epoch != s.target.membership_epoch
            || value.generation != 1
            || value.stream_revision == 0
            || value.participant_revision == 0
            || value.streams.len() != 1
            || (private && (value.stream_revision != 2 || value.streams != ["telephone-audio"]))
        {
            return Err(Error::forbidden());
        }
        for v in &value.streams {
            id(v)?;
        }
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                historical(self, &c, &origin.signal)?;
                current_scopes(&c, original, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = historical(self, &tx, &origin.signal)?;
        let read = self.initial_telephone_read_with_purpose(
            &tx,
            origin.clone(),
            held.clone(),
            crate::now() + 30,
            if private {
                ProjectionPurpose::PrivateAdmission
            } else {
                ProjectionPurpose::Media
            },
        )?;
        let mut details = json!({"connection_id":original.connection,"participant_id":value.participant_id,"membership_epoch":value.membership_epoch,"generation":value.generation,"endpoint_id":s.endpoint_id,"device_id":s.device_id,"stream_revision":value.stream_revision,"participant_revision":value.participant_revision,"streams":value.streams,"codec":origin.codec});
        if private {
            details["private_audio"] = json!(true);
            details["private_consultation_record_id"] = json!(s.parent.record_id());
            details["runtime_connection_id"] = json!(value.runtime_connection_id);
            details["call_id"] = json!(value.call_id);
        }
        let encoded = crate::provisioning::canonical_json(&details)?;
        if row.record.state == OperationState::Completed {
            let old:String=tx.query_row("SELECT connected_json FROM initial_telephone_answers WHERE operation_record_id=?1",[origin.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if old != encoded {
                return Err(conflict());
            }
            return Ok(row.record);
        }
        if !read.current() || row.record.state != OperationState::Preparing {
            return Err(Error::forbidden());
        }
        tx.execute("UPDATE connections SET state='ready',observed_runtime_id=?2,observed_at=?3,readiness_json=?4,revision=revision+1 WHERE id=?1 AND generation=1 AND state='preparing'",params![original.connection,original.runtime,crate::now(),encoded]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE initial_telephone_answers SET connected_json=?2,connected_at=?3 WHERE operation_record_id=?1 AND connected_json IS NULL AND accepted=1",params![origin.record_id(),encoded,crate::now()]).map_err(|_|Error::storage())?;
        let mut current = row;
        for next in [
            OperationState::Ready,
            OperationState::Committing,
            OperationState::Completed,
        ] {
            rows::change(
                &tx,
                &current,
                next,
                if next == OperationState::Completed {
                    Some(details.clone())
                } else {
                    None
                },
                self,
            )?;
            current = rows::by_record(&tx, origin.record_id())?;
        }
        emit_connection(&tx, self, &origin, "connection_ready")?;
        if observe()? != value {
            return Err(Error::forbidden());
        }
        if !private {
            self.remember_session_ready(
                &tx,
                crate::session_lifecycle::ReadyOrigin::InitialTelephone(origin.clone()),
            )?;
            crate::session_lifecycle::activate(&tx, self, &s.conversation_id, &s.session_id)?;
            let ready = self.initial_telephone_session_readiness(&tx, &origin)?;
            crate::session_participant_lifecycle::observed_join(
                &tx,
                self,
                &s.conversation_id,
                &s.session_id,
                &s.target.participant_id,
                s.target.membership_epoch,
                origin.connection_id(),
                ready.observed_at_millis,
            )?;
            crate::session_reconnect::connected(&tx, &s.session_id)?;
        }
        super::super::workers::invalidate(&tx, origin.record_id())?;
        let result = rows::by_record(&tx, origin.record_id())?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
fn emit_connection(
    c: &Connection,
    store: &Store,
    origin: &InitialTelephoneMediaOrigin,
    kind: &str,
) -> Result<()> {
    let original = origin.signal.origin();
    let s = origin.source();
    let value=c.query_row("SELECT n.id,n.session_id,n.participant_id,n.membership_epoch,n.endpoint_id,e.device_id,n.generation,n.revision,n.state,n.replaces FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[&original.connection],|r|Ok(connect_protocol::events::ConnectionState{connection_id:r.get(0)?,session_id:r.get(1)?,participant_id:r.get(2)?,membership_epoch:r.get(3)?,endpoint_id:r.get(4)?,device_id:r.get(5)?,generation:r.get(6)?,revision:r.get(7)?,state:r.get(8)?,replaces:r.get(9)?})).map_err(|_|Error::storage())?;
    let recipients = if s.is_private_candidate() {
        s.audience.iter().map(|m| m.identity_id.clone()).collect()
    } else {
        crate::audience::source_members(
            c,
            &s.conversation_id,
            connect_protocol::membership::ContentKind::Session,
            &s.session_id,
        )?
    };
    if !crate::conversations::capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Initial telephone journal capacity exhausted",
        ));
    }
    for recipient in recipients {
        let mut data = json!({"source_kind":"session","source_id":s.session_id,"session_id":s.session_id,"connection_id":original.connection,"participant_id":s.target.participant_id,"identity_id":s.target.identity_id,"connection":value});
        if recipient == s.caller.identity_id {
            data["operation_id"] = json!(original.operation);
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json,source_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,s.conversation_id,kind,original.record,value.revision,data.to_string(),json!({"kind":"connect-sip-runtime","correlation_id":original.connection}).to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
pub struct InitialTelephoneNativeRead {
    phone: InitialTelephoneMediaRead,
    native: crate::NativeMediaRead,
}
media_redacted!(InitialTelephoneNativeRead);
impl InitialTelephoneNativeRead {
    pub fn current(&self) -> bool {
        self.phone.current() && self.native.current()
    }
    pub fn phone(&self) -> &InitialTelephoneMediaRead {
        &self.phone
    }
    pub fn native(&self) -> &crate::NativeMediaRead {
        &self.native
    }
}
impl Store {
    pub fn try_initial_telephone_native_read(
        &self,
        phone: Arc<InitialTelephoneMediaOrigin>,
        native: Arc<crate::NativeMediaOrigin>,
    ) -> Result<InitialTelephoneNativeRead> {
        let s = phone.source();
        if native.conversation_id() != s.conversation_id || native.session_id() != s.session_id {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        let (_, mut required, _) = self.native_scene_facts(
            &c,
            &[native.clone()],
            native.conversation_id(),
            native.session_id(),
        )?;
        required.extend(if phone.source().is_private_candidate() {
            private_facts::current_authority(self, &c, &phone)?.0
        } else {
            current_scopes(&c, phone.signal.origin(), false)?
        });
        let held = self.try_acquire_authority(&AuthorityScope::merged(&required)?)?;
        let (facts, _, expiry) = self.native_scene_facts(
            &c,
            &[native.clone()],
            native.conversation_id(),
            native.session_id(),
        )?;
        let mut reads = self.native_scene_reads(facts, held.clone(), expiry)?;
        let read = reads
            .remove(&(native.runtime_connection_id().into(), None))
            .ok_or_else(Error::forbidden)?;
        self.initial_telephone_native_pair_in(&c, phone, &native, read)
    }
    pub(crate) fn initial_telephone_native_pair_in(
        &self,
        c: &Connection,
        phone: Arc<InitialTelephoneMediaOrigin>,
        native_origin: &crate::NativeMediaOrigin,
        native: crate::NativeMediaRead,
    ) -> Result<InitialTelephoneNativeRead> {
        self.require_sip_native_pair_context(
            c,
            phone.connection_id(),
            native_origin,
            native.target().private_consultation.as_deref(),
        )?;
        let actor = self.native_media_route_actor(c, native_origin)?;
        if !actor.permits_recipient(&phone.source().target.identity_id) {
            return Err(Error::forbidden());
        }
        let initial = if phone.source().is_private_candidate() {
            self.private_initial_telephone_media_in(
                c,
                phone,
                native.interval().clone(),
                native.expires_at(),
            )?
        } else {
            self.initial_telephone_media_in(
                c,
                phone,
                native.interval().clone(),
                native.expires_at(),
            )?
        };
        if !initial.current()
            || !native.current()
            || initial.source().session_id != native.target().session.session.id
            || !initial
                .endpoint_actor()
                .permits_recipient(&native.target().session.actor.identity_id)
            || !initial
                .route_actor()
                .permits_recipient(&native.target().session.actor.identity_id)
        {
            return Err(Error::forbidden());
        }
        Ok(InitialTelephoneNativeRead {
            phone: initial,
            native,
        })
    }
}
impl Store {
    /// An original origin can retire its never-submitted reservation without
    /// renewing a withdrawn caller grant. A started effect cannot use this path.
    pub fn abandon_initial_telephone_unstarted(
        &self,
        origin: &InitialTelephoneOrigin,
    ) -> Result<()> {
        origin.lifetime(self)?;
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                current_scopes(&c, origin, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        origin.lifetime(self)?;
        if !self.authority_covers(&held, &current_scopes(&tx, origin, true)?) {
            return Err(Error::forbidden());
        }
        let started:bool=tx.query_row("SELECT signaling_started FROM initial_telephone_operations WHERE operation_record_id=?1 AND caller_credential_id=?2",params![origin.record,origin.credential],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        if started {
            return Err(Error::new(
                "reconciliation-required",
                "A submitted telephone attempt needs runtime cleanup",
            ));
        }
        let row = rows::by_record(&tx, &origin.record)?;
        if !row.record.state.terminal() {
            rows::change(&tx, &row, OperationState::Failed, None, self)?;
        }
        tx.execute("UPDATE connections SET state='retired',generation=generation+1,revision=revision+1 WHERE id=?1 AND state='preparing'",[&origin.connection]).map_err(|_|Error::storage())?;
        super::super::workers::invalidate(&tx, &origin.record)?;
        tx.commit().map_err(|_| Error::storage())
    }
    /// Same-runtime original custody plus actual adapter absence settles local
    /// resource cleanup. An operator assertion or empty restarted Core cannot.
    pub fn settle_initial_telephone_cleanup(
        &self,
        signal: &Arc<InitialTelephoneSignaling>,
        absent: &dyn crate::TelephoneConnectionRuntimeObservation,
    ) -> Result<()> {
        absent.observe_absent(signal.runtime_connection_id(), 1)?;
        let original = signal.origin();
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                historical(self, &c, signal)?;
                current_scopes(&c, original, true)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = historical(self, &tx, signal)?;
        if !self.authority_covers(&held, &current_scopes(&tx, original, true)?) {
            return Err(Error::forbidden());
        }
        let details = json!({"kind":"owned-runtime-absent","runtime_connection_id":signal.runtime_connection_id(),"call_id":signal.sip_call_id(),"generation":1});
        let effect: String = tx
            .query_row(
                "SELECT effect_id FROM initial_telephone_operations WHERE operation_record_id=?1",
                [&original.record],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        // Local absence settles resource custody, not the historical result of
        // an uncertain remote submission. Preserve that outcome separately.
        tx.execute(
            "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
            [&effect],
        )
        .map_err(|_| Error::storage())?;
        let cleanup = format!("{}-cleanup", original.record);
        tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,provider_ref,observation_json,compensates_effect_id) VALUES(?1,?2,'sip-initial',?1,'none','confirmed',1,1,?3,?4,?5) ON CONFLICT(id) DO NOTHING",params![cleanup,original.record,signal.sip_call_id(),details.to_string(),effect]).map_err(|_|Error::storage())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_effects WHERE id=?1 AND operation_record_id=?2 AND provider='sip-initial' AND state='confirmed' AND provider_ref=?3 AND observation_json=?4 AND compensates_effect_id=?5)",params![cleanup,original.record,signal.sip_call_id(),details.to_string(),effect],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !exact {
            return Err(conflict());
        }
        let retired = tx.execute("UPDATE connections SET state='retired',generation=CASE WHEN generation=1 THEN 2 ELSE generation END,revision=revision+1,readiness_json=NULL,observed_runtime_id=NULL,observed_at=NULL WHERE id=?1 AND ((state!='retired' AND generation=1) OR (state='retiring' AND generation=2))",[&original.connection]).map_err(|_|Error::storage())?;
        tx.execute("UPDATE initial_telephone_answers SET retired_at=?2 WHERE operation_record_id=?1 AND retired_at IS NULL",params![original.record,crate::now()]).map_err(|_|Error::storage())?;
        if !row.record.state.terminal() {
            let next = if row.record.cancel_requested {
                OperationState::Cancelled
            } else {
                OperationState::Failed
            };
            rows::change(&tx, &row, next, None, self)?;
        }
        let observed:Option<(String,String)>=tx.query_row("SELECT codec,observation_json FROM initial_telephone_answers WHERE operation_record_id=?1",[&original.record],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
        if let Some((codec, observation)) = observed.filter(|_| retired == 1) {
            let origin = InitialTelephoneMediaOrigin {
                signal: signal.clone(),
                codec,
                observation,
            };
            emit_connection(&tx, self, &origin, "connection_retired")?;
        }
        if retired == 1 {
            super::super::effects::touch(&tx, &original.record, self)?;
        }
        absent.observe_absent(signal.runtime_connection_id(), 1)?;
        super::super::workers::invalidate(&tx, &original.record)?;
        crate::session_participant_lifecycle::reconcile(
            &tx,
            self,
            &original.source.conversation_id,
            &original.source.session_id,
            None,
        )?;
        tx.commit().map_err(|_| Error::storage())?;
        if original.source.is_private_candidate() {
            self.private_initial_telephone_origins
                .lock()
                .map_err(|_| Error::storage())?
                .remove(&original.connection);
        }
        Ok(())
    }
}
