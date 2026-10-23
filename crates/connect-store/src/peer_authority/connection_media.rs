//! Reacquire current packet authority without retaining a command's domain grant.
use super::*;
use connect_protocol::connection_control::{OfferIntent, PrepareRequest};
use std::sync::Weak;
mod admission;
mod credential_refresh;
pub use credential_refresh::{
    NativeCredentialRefreshApplied, NativeCredentialRefreshAttempt, NativeCredentialRefreshPlan,
    NativeCredentialRefreshTarget,
};
mod assistant;
pub use assistant::{
    AssistantContextFailure, AssistantProviderApplied, AssistantProviderAttempt,
    AssistantProviderClosed, AssistantProviderContext, AssistantProviderCreated,
    AssistantProviderEnd, AssistantProviderExport, AssistantProviderInitialization,
    AssistantProviderRead, AssistantProviderRejected, AssistantProviderTiming,
    AssistantProviderUpdate, AssistantProviderUpdateApplied, AssistantProviderUpdateExport,
    AssistantProviderUpdateRejected, AssistantProviderUpdateStatus, NativeAssistantContextRead,
};
mod consultation;
pub(crate) mod consultation_sip;
use admission::NativeMediaAdmission;
mod handoff;
mod handoff_lifetime;
mod replacement;
mod roles;
pub use roles::SessionRoleMediaRead;
mod returns;
pub use consultation::{NativeConsultationRead, NativeConsultationSelection};
pub use handoff::{NativeHandoffRead, NativeHandoffSelection};
pub use replacement::{
    NativeReplacementPreparationRead, NativeReplacementSelection, NativeReplacementSourceOrigin,
    NativeReplacementSourceRead,
};
pub use returns::{NativeReturnRead, NativeReturnSelection};
mod capabilities;
mod flow_facts;
pub use flow_facts::{NativeConnectionFlowFact, NativeConnectionFlowObservation};
mod data_reads;
mod data_writes;
mod reconciliation;
mod session_codecs;
pub use data_reads::NativeDataRead;
pub use data_writes::NativeDataWriter;
pub use reconciliation::NativeReconciliationRead;
pub use session_codecs::NativeSessionCodecRead;

/// Store-issued immutable origin, not a grant. No command/SQL/domain grant is
/// retained here; each future use must acquire and resolve current authority.
#[derive(Clone)]
pub struct NativeMediaOrigin {
    attempt: NativeMediaAdmission,
    owner: Weak<InstallationOwnership>,
    binding: PeerAuthorityRequest,
    intent: OfferIntent,
    endpoint_id: String,
    participant_id: String,
    membership_epoch: u64,
    generation: u64,
    runtime_connection_id: String,
    public_key: Vec<u8>,
    expires_at: i64,
    credential_epoch: u64,
}
impl NativeMediaOrigin {
    pub(crate) fn require_runtime(&self, store: &Store) -> Result<()> {
        current_owner(store, self).map(|_| ())
    }
    pub fn credential_epoch(&self) -> u64 {
        self.credential_epoch
    }
    pub fn offer_record_id(&self) -> &str {
        self.attempt.record_id()
    }
    pub fn connection_id(&self) -> &str {
        &self.intent.connection_id
    }
    pub fn runtime_connection_id(&self) -> &str {
        &self.runtime_connection_id
    }
    pub fn conversation_id(&self) -> &str {
        &self.intent.conversation_id
    }
    pub fn session_id(&self) -> &str {
        &self.intent.session_id
    }
    pub fn participant_id(&self) -> &str {
        &self.participant_id
    }
    pub fn membership_epoch(&self) -> u64 {
        self.membership_epoch
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
}
impl std::fmt::Debug for NativeMediaOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeMediaOrigin(redacted)")
    }
}
/// Keep this actual owned fence through the bounded engine operation/delivery.
/// The views themselves are facts; copying them does not copy current authority.
pub struct NativeMediaRead {
    target: PeerConnectionProjection,
    source: Option<PeerConnectionProjection>,
    _scopes: Arc<AuthorityGuardSet>,
    owner: Arc<InstallationOwnership>,
    expires_at: i64,
    may_subscribe: bool,
}
impl NativeMediaRead {
    pub(crate) fn interval(&self) -> &Arc<AuthorityGuardSet> {
        &self._scopes
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn target(&self) -> &PeerConnectionProjection {
        &self.target
    }
    pub fn source(&self) -> Option<&PeerConnectionProjection> {
        self.source.as_ref()
    }
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires_at
    }
    /// Separate current named permission; Connection creation alone never
    /// authorizes receipt of another publisher's selected media.
    pub fn permits_subscriptions(&self) -> bool {
        self.current() && self.may_subscribe
    }
}
/// Named-signal metadata and current readers. This cannot be used as an RTP
/// publication/subscription grant or converted into NativeMediaRead.
pub struct NativeSignalRead(NativeMediaRead);
impl NativeSignalRead {
    pub fn target(&self) -> &PeerConnectionProjection {
        self.0.target()
    }
    pub fn source(&self) -> Option<&PeerConnectionProjection> {
        self.0.source()
    }
    pub fn current(&self) -> bool {
        self.0.current()
    }
    pub(crate) fn interval(&self) -> &Arc<AuthorityGuardSet> {
        self.0.interval()
    }
}
#[derive(Clone, Copy)]
enum ReadPurpose {
    Media,
    Signal,
}
fn subscription_permission(c: &Connection, target: &NativeMediaOrigin) -> Result<bool> {
    let credential = target
        .binding
        .credentials
        .last()
        .ok_or_else(Error::forbidden)?;
    Ok(crate::identity::actor_from_connection(c, credential)?.permits("stream.subscribe"))
}
fn current_owner(store: &Store, origin: &NativeMediaOrigin) -> Result<Arc<InstallationOwnership>> {
    let owner = store.require_runtime_owner()?;
    if !origin
        .owner
        .upgrade()
        .is_some_and(|o| Arc::ptr_eq(&owner, &o))
        || !owner.current()
        || crate::now() >= origin.expires_at
    {
        return Err(Error::forbidden());
    }
    Ok(owner)
}
fn facts(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    facts_for(store, c, origin, ReadPurpose::Media)
}
fn facts_for(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
    purpose: ReadPurpose,
) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
    current_owner(store, origin)?;
    credential_refresh::require_epoch(store, c, origin)?;
    if let Some(life) = handoff_lifetime::current(store, c, origin)? {
        let (mut view, mut scopes, expiry) = match purpose {
            ReadPurpose::Media => returns::handoff_facts(store, c, origin)?,
            ReadPurpose::Signal => returns::signal_facts(store, c, origin)?,
        };
        if let Some(role) = life.role {
            view.presence_role = role;
        }
        scopes.extend(life.scopes);
        return Ok((
            view,
            AuthorityScope::merged(&scopes)?,
            expiry.min(life.expires_at),
        ));
    }
    let attempt_expiry = origin.attempt.current(c, store)?;
    let (meta, key, mut scopes, mut expires_at) = checked(store, c, &origin.binding)?;
    if let Some(context) = crate::connection_control::private_consultation::current(
        store,
        c,
        &origin.intent,
        Some(&origin.endpoint_id),
    )? {
        expires_at = expires_at.min(context.expires_at);
        scopes.extend(context.scopes);
    }
    if key.public_key != origin.public_key {
        return Err(Error::forbidden());
    }
    if matches!(purpose, ReadPurpose::Signal) {
        crate::audience::require_original(
            c,
            &meta.actor,
            &origin.intent.conversation_id,
            connect_protocol::membership::ContentKind::Session,
            &origin.intent.session_id,
        )?;
    }
    if matches!(purpose, ReadPurpose::Media) {
        meta.actor.require("connection.offer")?;
        let recipients =
            crate::connection_control::intent_resources(c, &meta.actor, &origin.intent)?;
        if recipients
            .iter()
            .any(|id| !meta.actor.permits_recipient(id))
        {
            return Err(Error::forbidden());
        }
    }
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        &origin.intent.conversation_id,
        AuthorityMode::Read,
    )?);
    let projection = super::connection_projection::current_projection(
        c,
        &meta.actor,
        &origin.intent,
        &origin.endpoint_id,
        &origin.binding.device.id,
        store.runtime_epoch(),
        false,
    )?;
    if projection.runtime_connection_id != origin.runtime_connection_id
        || projection.connection.participant_id != origin.participant_id
        || projection.connection.membership_epoch != origin.membership_epoch
        || projection.connection.generation != origin.generation
    {
        return Err(Error::forbidden());
    }
    Ok((
        projection,
        scopes,
        expires_at
            .min(origin.expires_at)
            .min(attempt_expiry.unwrap_or(i64::MAX)),
    ))
}
pub(crate) fn views(
    store: &Store,
    c: &Connection,
    target: &NativeMediaOrigin,
    source: Option<&NativeMediaOrigin>,
) -> Result<(
    PeerConnectionProjection,
    Option<PeerConnectionProjection>,
    Vec<AuthorityScope>,
    i64,
)> {
    views_for(store, c, target, source, ReadPurpose::Media)
}
fn views_for(
    store: &Store,
    c: &Connection,
    target: &NativeMediaOrigin,
    source: Option<&NativeMediaOrigin>,
    purpose: ReadPurpose,
) -> Result<(
    PeerConnectionProjection,
    Option<PeerConnectionProjection>,
    Vec<AuthorityScope>,
    i64,
)> {
    let (view, mut scopes, mut expires) = facts_for(store, c, target, purpose)?;
    let source = if let Some(source) = source {
        if target.intent.conversation_id != source.intent.conversation_id
            || target.intent.session_id != source.intent.session_id
        {
            return Err(Error::forbidden());
        }
        let (source_view, source_scopes, source_expiry) = facts_for(store, c, source, purpose)?;
        let target_handoff = handoff_lifetime::current(store, c, target)?;
        let source_handoff = handoff_lifetime::current(store, c, source)?;
        if target_handoff.is_some() || source_handoff.is_some() {
            match (&target_handoff, &source_handoff) {
                (Some(t), Some(s))
                    if t.record == s.record
                        && t.pairs.contains(&(
                            target.connection_id().into(),
                            source.connection_id().into(),
                        )) =>
                {
                    ()
                }
                _ => return Err(Error::forbidden()),
            }
        } else {
            if view.handoff_preparation || source_view.handoff_preparation {
                return Err(Error::forbidden());
            }
            for (private, other) in [(target, source), (source, target)] {
                if let Some(partner) =
                    crate::connection_control::private_consultation::private_partner(
                        c,
                        private.connection_id(),
                    )?
                {
                    if other.connection_id() != partner {
                        return Err(Error::forbidden());
                    }
                }
            }
        }
        let target_actor = crate::identity::actor_from_connection(
            c,
            target
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?,
        )?;
        let source_actor = crate::identity::actor_from_connection(
            c,
            source
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?,
        )?;
        if !target_actor.permits_recipient(&source_view.session.actor.identity_id)
            || !source_actor.permits_recipient(&view.session.actor.identity_id)
        {
            return Err(Error::forbidden());
        }
        scopes.extend(source_scopes);
        expires = expires.min(source_expiry);
        Some(source_view)
    } else {
        None
    };
    Ok((view, source, AuthorityScope::merged(&scopes)?, expires))
}
impl Store {
    pub(crate) fn native_signal_facts_at(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
    ) -> Result<(PeerConnectionProjection, Vec<AuthorityScope>, i64)> {
        facts_for(self, c, origin, ReadPurpose::Signal)
    }
    pub(crate) fn native_signal_read_in(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<NativeSignalRead> {
        let (target, scopes, expiry) = self.native_signal_facts_at(c, origin)?;
        let expires_at = expiry.min(until);
        if !self.authority_covers(&interval, &scopes) || crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        Ok(NativeSignalRead(NativeMediaRead {
            target,
            source: None,
            _scopes: interval,
            owner: current_owner(self, origin)?,
            expires_at,
            may_subscribe: false,
        }))
    }
    pub(crate) fn native_dtmf_pair_at(
        &self,
        c: &Connection,
        source: &NativeMediaOrigin,
        target: &NativeMediaOrigin,
    ) -> Result<(Vec<AuthorityScope>, i64)> {
        if source.runtime_connection_id() == target.runtime_connection_id() {
            return Err(Error::forbidden());
        }
        let (to, from, scopes, expires) =
            views_for(self, c, target, Some(source), ReadPurpose::Signal)?;
        let from = from.ok_or_else(Error::forbidden)?;
        for view in [&to, &from] {
            if !view.session.session.media.iter().any(|m| m == "audio")
                || !view.endpoint_capabilities.iter().any(|m| m == "audio")
            {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "DTMF requires a current audio Connection",
                ));
            }
        }
        self.native_media_route_actor(c, source)?
            .require("dtmf.send")?;
        self.native_media_route_actor(c, target)?
            .require("dtmf.receive")?;
        Ok((scopes, expires))
    }
    /// Fresh signal readers preserve the same physical/audience fences while
    /// requiring independent send/receive permissions at the named boundary.
    pub(crate) fn try_native_signal_read(
        &self,
        target: &NativeMediaOrigin,
        source: &NativeMediaOrigin,
    ) -> Result<NativeSignalRead> {
        let scopes = {
            let c = self.try_connection()?;
            self.native_dtmf_pair_at(&c, source, target)?.0
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (scopes, expires_at) = self.native_dtmf_pair_at(&c, source, target)?;
        let (target_view, source_view, _, _) =
            views_for(self, &c, target, Some(source), ReadPurpose::Signal)?;
        if !self.authority_covers(&held, &scopes) || crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        Ok(NativeSignalRead(NativeMediaRead {
            target: target_view,
            source: source_view,
            _scopes: held,
            owner: current_owner(self, target)?,
            expires_at,
            may_subscribe: false,
        }))
    }
    pub(crate) fn validate_native_dtmf_source_with_peer(
        &self,
        target: &NativeMediaOrigin,
        proof: &PeerAuthorityGuard,
        lease: &crate::NativeStreamLease,
    ) -> Result<()> {
        let credential = proof
            .request
            .credentials
            .last()
            .ok_or_else(Error::forbidden)?;
        let PeerCommand::NativeStream(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if intent.command != connect_protocol::native_streams::Command::Dtmf {
            return Err(Error::forbidden());
        }
        let projected = self.validate_native_stream_attempt(credential, proof, lease)?;
        let c = self.try_connection()?;
        self.require_peer_source(&c, proof, credential)?;
        self.native_media_route_actor(&c, target)?
            .require("dtmf.send")?;
        let (view, _, scopes, expiry) = views_for(self, &c, target, None, ReadPurpose::Signal)?;
        if projected.connection.runtime_connection_id != view.runtime_connection_id
            || projected.connection.connection != view.connection
            || view.runtime_connection_id != lease.runtime_connection_id()
            || view.connection.generation != lease.generation()
            || !self.authority_covers(&proof._scopes, &scopes)
            || crate::now() >= expiry.min(proof.expires_at()).min(lease.expires_at())
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn native_dtmf_origin_snapshot(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
    ) -> Result<serde_json::Value> {
        let actor = self.native_media_route_actor(c, origin)?;
        Ok(
            serde_json::json!({"connection_id":origin.connection_id(),"runtime_connection_id":origin.runtime_connection_id(),"participant_id":origin.participant_id(),"membership_epoch":origin.membership_epoch(),"generation":origin.generation(),"credential_epoch":origin.credential_epoch,"credential_id":actor.credential_id,"identity_id":actor.identity_id,"endpoint_id":origin.endpoint_id,"device_id":origin.binding.device.id,"key_id":origin.binding.key_id}),
        )
    }
    pub(crate) fn native_media_route_actor(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
    ) -> Result<crate::Actor> {
        credential_refresh::require_epoch(self, c, origin)?;
        crate::identity::actor_from_connection(
            c,
            origin
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?,
        )
    }
    /// The caller has just completed the exact private preparation under this
    /// signed invocation. Pending private source intervals are rechecked in
    /// facts(); established contexts follow their own current physical lifetime.
    pub fn native_media_origin_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        lease: &crate::NativeOfferLease,
    ) -> Result<Arc<NativeMediaOrigin>> {
        self.validate_native_offer_attempt(credential, lease, proof)?;
        let projection = self.project_connection_with_peer(credential, proof)?;
        let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if !proof.current() {
            return Err(Error::forbidden());
        }
        let mut binding = proof.request.clone();
        binding.credentials = vec![credential.into()];
        binding.command = PeerCommand::Binding;
        let expires_at =
            if projection.private_consultation.is_some() || projection.handoff_preparation {
                let c = self.connection()?;
                checked(self, &c, &binding)?.3
            } else {
                proof.expires_at.min(lease.source_expires_at())
            };
        Ok(Arc::new(NativeMediaOrigin {
            attempt: NativeMediaAdmission::Offer(lease.media_binding()),
            owner: Arc::downgrade(&proof._owner),
            binding,
            intent: intent.clone(),
            endpoint_id: projection.connection.endpoint_id,
            participant_id: projection.connection.participant_id,
            membership_epoch: projection.connection.membership_epoch,
            generation: projection.connection.generation,
            runtime_connection_id: projection.runtime_connection_id,
            public_key: proof.key.public_key.clone(),
            expires_at,
            credential_epoch: 0,
        }))
    }
    /// A transient physical-lifetime observation, never a media or signal IO
    /// grant. Current ownership/access can survive loss of an independent named
    /// operation permission. All readers are released before the caller idles.
    pub fn try_native_connection_current(&self, origin: &NativeMediaOrigin) -> Result<bool> {
        let scopes = {
            let c = self.try_connection()?;
            facts_for(self, &c, origin, ReadPurpose::Signal)?.1
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (_, scopes, expiry) = facts_for(self, &c, origin, ReadPurpose::Signal)?;
        if !self.authority_covers(&held, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(current_owner(self, origin)?.current() && crate::now() < expiry)
    }
    /// Resolve metadata with the same current origin checks as native IO.
    /// No authority reader or executable media handle is returned.
    pub(crate) fn native_session_readiness_projection(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
    ) -> Result<(PeerConnectionProjection, i64, bool)> {
        let (view, _, expiry) = facts(self, c, origin)?;
        // Keep private provenance for every other use. Public readiness may
        // use an actually attributed, completed handoff only after the same
        // lifetime validator has checked its current independent participants.
        let public_handoff = if let Some(life) = handoff_lifetime::current(self, c, origin)? {
            c.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE record_id=?1 AND state='completed' AND result_json IS NOT NULL)",[&life.record],|r|r.get(0)).map_err(|_|Error::storage())?
        } else {
            false
        };
        Ok((view, expiry, public_handoff))
    }
    /// Immediate fresh packet admission. Refuse contention instead of waiting
    /// under engine fences. Resolve again after acquiring the complete scope set.
    pub fn try_native_media_read(
        &self,
        target: &NativeMediaOrigin,
        source: Option<&NativeMediaOrigin>,
    ) -> Result<NativeMediaRead> {
        let scopes = {
            let c = self.try_connection()?;
            views(self, &c, target, source)?.2
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let (target_view, source_view, current_scopes, expires_at) =
            views(self, &c, target, source)?;
        if !self.authority_covers(&held, &current_scopes) {
            return Err(Error::forbidden());
        }
        let owner = current_owner(self, target)?;
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        self.remember_session_ready_origin(&c, target)?;
        if let Some(source) = source {
            self.remember_session_ready_origin(&c, source)?;
        }
        Ok(NativeMediaRead {
            target: target_view,
            source: source_view,
            _scopes: held,
            owner,
            expires_at,
            may_subscribe: subscription_permission(&c, target)?,
        })
    }
    /// Initial control derives the same admitted grant, with no reader reentry
    /// or fallback when a foreign/insufficient/different intent proof is supplied.
    pub fn native_media_read_with_peer(
        &self,
        target: &NativeMediaOrigin,
        source: Option<&NativeMediaOrigin>,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeMediaRead> {
        let c = self.try_connection()?;
        let credential = target
            .binding
            .credentials
            .last()
            .ok_or_else(Error::forbidden)?;
        self.require_peer_control(
            &c,
            proof,
            credential,
            &connect_protocol::Operation::ConnectionPrepare {
                request: PrepareRequest::new(target.intent.clone(), target.endpoint_id.clone()),
            },
        )?;
        let (target_view, source_view, scopes, expires_at) = views(self, &c, target, source)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        let owner = current_owner(self, target)?;
        let expires_at = expires_at.min(proof.expires_at);
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        Ok(NativeMediaRead {
            target: target_view,
            source: source_view,
            _scopes: proof._scopes.clone(),
            owner,
            expires_at,
            may_subscribe: subscription_permission(&c, target)?,
        })
    }
}

impl Store {
    /// Capture the exact current stream command's target/source readers outside
    /// engine fences. Callbacks retain this bounded view without SQL reentry.
    pub fn native_stream_media_read_with_peer(
        &self,
        target: &NativeMediaOrigin,
        source: Option<&NativeMediaOrigin>,
        proof: &PeerAuthorityGuard,
        lease: &crate::NativeStreamLease,
    ) -> Result<NativeMediaRead> {
        let credential = proof
            .request
            .credentials
            .last()
            .ok_or_else(Error::forbidden)?;
        let projected = self.validate_native_stream_attempt(credential, proof, lease)?;
        let c = self.try_connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let (target_view, source_view, scopes, expires_at) = views(self, &c, target, source)?;
        if projected.connection.runtime_connection_id != target_view.runtime_connection_id
            || projected.connection.connection != target_view.connection
            || target_view.runtime_connection_id != lease.runtime_connection_id()
            || target_view.connection.generation != lease.generation()
            || !self.authority_covers(&proof._scopes, &scopes)
        {
            return Err(Error::forbidden());
        }
        let owner = current_owner(self, target)?;
        let expires_at = expires_at.min(proof.expires_at()).min(lease.expires_at());
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        Ok(NativeMediaRead {
            target: target_view,
            source: source_view,
            _scopes: proof._scopes.clone(),
            owner,
            expires_at,
            may_subscribe: subscription_permission(&c, target)?,
        })
    }
}

/// Resolve existing scene routes without acquiring a nested authority interval.
/// A continuation first merges these scopes with its independent target/operator
/// footprint, acquires once, then resolves them again under that complete grant.
pub(crate) type SceneFacts = std::collections::BTreeMap<
    (String, Option<String>),
    (
        PeerConnectionProjection,
        Option<PeerConnectionProjection>,
        bool,
    ),
>;
impl Store {
    pub(crate) fn native_scene_facts(
        &self,
        c: &Connection,
        sources: &[Arc<NativeMediaOrigin>],
        conversation: &str,
        session: &str,
    ) -> Result<(SceneFacts, Vec<AuthorityScope>, i64)> {
        let mut rows = SceneFacts::new();
        let mut scopes = Vec::new();
        let mut expiry = crate::now() + 5;
        let mut seen = std::collections::BTreeSet::new();
        for target in sources {
            if target.conversation_id() != conversation
                || target.session_id() != session
                || !seen.insert(target.runtime_connection_id())
            {
                return Err(Error::forbidden());
            }
            let (view, _, required, until) = views(self, c, target, None)?;
            let may = subscription_permission(c, target)?;
            rows.insert(
                (target.runtime_connection_id().into(), None),
                (view, None, may),
            );
            scopes.extend(required);
            expiry = expiry.min(until);
            for source in sources {
                if Arc::ptr_eq(target, source) {
                    continue;
                }
                match views(self, c, target, Some(source)) {
                    Ok((view, other, required, until)) => {
                        scopes.extend(required);
                        expiry = expiry.min(until);
                        rows.insert(
                            (
                                target.runtime_connection_id().into(),
                                Some(source.runtime_connection_id().into()),
                            ),
                            (view, other, may),
                        );
                    }
                    Err(e) if e.code == "forbidden" => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok((rows, AuthorityScope::merged(&scopes)?, expiry))
    }
    pub(crate) fn native_scene_reads(
        &self,
        facts: SceneFacts,
        held: Arc<AuthorityGuardSet>,
        expiry: i64,
    ) -> Result<std::collections::BTreeMap<(String, Option<String>), NativeMediaRead>> {
        let owner = self.require_runtime_owner()?;
        Ok(facts
            .into_iter()
            .map(|(id, (target, source, may_subscribe))| {
                (
                    id,
                    NativeMediaRead {
                        target,
                        source,
                        _scopes: held.clone(),
                        owner: owner.clone(),
                        expires_at: expiry,
                        may_subscribe,
                    },
                )
            })
            .collect())
    }
}

impl Store {
    /// Retarget only already-authorized source pairs. The candidate supplies its
    /// own current named receive permission and both peers' recipient ceilings;
    /// the old connection's permission never becomes the new peer's permission.
    pub(crate) fn native_replacement_scene_pairs(
        &self,
        c: &Connection,
        sources: &[Arc<NativeMediaOrigin>],
        facts: &mut SceneFacts,
        old: &str,
        telephone: Option<&crate::TelephoneMediaRead>,
        target: &PeerConnectionProjection,
        target_credential: &str,
    ) -> Result<()> {
        let actor = crate::identity::actor_from_connection(c, target_credential)?;
        if actor.identity_id != target.session.actor.identity_id {
            return Err(Error::forbidden());
        }
        let candidate = &target.runtime_connection_id;
        let may = actor.permits("stream.subscribe");
        facts.insert((candidate.clone(), None), (target.clone(), None, may));
        for other in sources {
            if other.runtime_connection_id() == old {
                continue;
            }
            let peer = crate::identity::actor_from_connection(
                c,
                other
                    .binding
                    .credentials
                    .last()
                    .ok_or_else(Error::forbidden)?,
            )?;
            let Some((view, _, other_may)) = facts
                .get(&(other.runtime_connection_id().into(), None))
                .cloned()
            else {
                return Err(Error::forbidden());
            };
            if !actor.permits_recipient(&view.session.actor.identity_id)
                || !peer.permits_recipient(&actor.identity_id)
            {
                continue;
            }
            let telephone_pair = telephone.is_some_and(|phone| {
                phone.current()
                    && phone
                        .route_actor()
                        .permits_recipient(&view.session.actor.identity_id)
                    && peer.permits_recipient(&phone.member().identity_id)
            });
            if telephone_pair
                || facts.contains_key(&(old.into(), Some(other.runtime_connection_id().into())))
            {
                facts.insert(
                    (
                        candidate.clone(),
                        Some(other.runtime_connection_id().into()),
                    ),
                    (target.clone(), Some(view.clone()), may),
                );
            }
            if telephone_pair
                || facts.contains_key(&(other.runtime_connection_id().into(), Some(old.into())))
            {
                facts.insert(
                    (
                        other.runtime_connection_id().into(),
                        Some(candidate.clone()),
                    ),
                    (view, Some(target.clone()), other_may),
                );
            }
        }
        Ok(())
    }
}
