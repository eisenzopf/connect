//! A server reoffer owns fresh original-source readers for its exact Core plan.
use super::sources::StreamSource;
use super::*;
use connect_store::{
    CredentialMetadata, NativeMediaRead, NativeReconciliationRead, NativeSessionCodecRead,
    PeerDeviceClaim, PeerSigningKey,
};
use rvoip_uctp::v2::native_negotiation::NativeOfferPlan;
use rvoip_uctp::v2::native_session_codecs::{NativeSessionCodecInvocation, NativeSessionCodecPlan};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, DispatchError},
    native_requests::NativeRequestScope,
    native_subscriptions::{NativeSubscriptionInvocation, NativeSubscriptionPlan},
    Envelope,
};
use serde_json::{json, Value};

pub(crate) enum Source {
    Subscription(Arc<NativeReconciliationRead>),
    Codec(Arc<NativeSessionCodecRead>),
}
impl Source {
    pub fn subscription(&self) -> Result<&NativeReconciliationRead, DispatchError> {
        match self {
            Self::Subscription(v) => Ok(v),
            _ => Err(DispatchError::Authority),
        }
    }
    pub fn codec(&self) -> Result<&NativeSessionCodecRead, DispatchError> {
        match self {
            Self::Codec(v) => Ok(v),
            _ => Err(DispatchError::Authority),
        }
    }
    pub fn current(&self) -> bool {
        match self {
            Self::Subscription(v) => v.current(),
            Self::Codec(v) => v.current(),
        }
    }
    pub fn media(&self) -> &NativeMediaRead {
        match self {
            Self::Subscription(v) => v.media(),
            Self::Codec(v) => v.media(),
        }
    }
    fn stream_revision(&self) -> u64 {
        match self {
            Self::Subscription(v) => v.stream_revision(),
            Self::Codec(v) => v.stream_revision(),
        }
    }
    pub fn credential_metadata(&self) -> &CredentialMetadata {
        match self {
            Self::Subscription(v) => v.credential_metadata(),
            Self::Codec(v) => v.credential_metadata(),
        }
    }
    pub fn signing_key(&self) -> &PeerSigningKey {
        match self {
            Self::Subscription(v) => v.signing_key(),
            Self::Codec(v) => v.signing_key(),
        }
    }
    pub fn signing_key_id(&self) -> &str {
        match self {
            Self::Subscription(v) => v.signing_key_id(),
            Self::Codec(v) => v.signing_key_id(),
        }
    }
    pub fn peer_device(&self) -> &PeerDeviceClaim {
        match self {
            Self::Subscription(v) => v.peer_device(),
            Self::Codec(v) => v.peer_device(),
        }
    }
}
pub(crate) struct Grant {
    pub policy: Arc<Policy>,
    pub domain: Arc<()>,
    pub connection: Arc<Connection>,
    pub source: Source,
    pub plan: Option<NativeSubscriptionPlan>,
    pub media: NativeOfferPlan,
    pub resources: bool,
    pub operation: String,
    pub scope: NativeRequestScope,
    pub deadline: tokio::time::Instant,
    cancel: tokio_util::sync::CancellationToken,
    sources: BTreeMap<ConnectionId, StreamSource>,
    pub proposal: Mutex<Option<Envelope>>,
}
impl Grant {
    pub fn media(&self) -> &NativeOfferPlan {
        &self.media
    }
    pub fn current(&self) -> bool {
        self.source.current()
            && !self.cancel.is_cancelled()
            && tokio::time::Instant::now() < self.deadline
    }
    pub fn matches(&self, scope: &NativeRequestScope) -> bool {
        self.current()
            && Arc::ptr_eq(&self.scope.origin, &scope.origin)
            && self.scope.stream_revision == scope.stream_revision
            && self.scope.connection_generation == scope.connection_generation
            && self.scope.credential_revision == scope.credential_revision
            && self.scope.conversation == scope.conversation
    }
    fn owner(&self, route: bool) -> ControlResult<MediaOwner> {
        if !self.current() {
            return Err(StreamControlError::Authority);
        }
        let owner = self.connection.owner_with_remote(
            self.source.media().target(),
            &self.scope.origin.principal,
            route,
            Some(self.media().remote_capabilities()),
        )?;
        self.connection
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)
    }
    pub fn descriptor(&self) -> Result<Value, DispatchError> {
        let streams=self.media().selected().iter().map(|s|Ok(json!({
            "id":s.requested.logical_id.as_str(),"kind":kind(s.requested.medium),
            "direction":direction(s.requested.direction),"codec":crate::peer_authority::native_stream_codec(&s.format)?
        }))).collect::<Result<Vec<_>,DispatchError>>()?;
        let sources = self
            .sources
            .values()
            .map(StreamSource::descriptor)
            .collect::<Vec<_>>();
        Ok(json!({"streams":streams,"sources":sources,"resources_changed":self.resources}))
    }
    pub fn authorize_proposal(
        &self,
        scope: &NativeRequestScope,
        proposal: &Envelope,
    ) -> Result<(), DispatchError> {
        if !self.matches(scope)
            || proposal.kind() != "connection.offer"
            || proposal.scope(Scope::Reply).is_some()
            || proposal.scope(Scope::Operation) != Some(self.operation.as_str())
            || proposal.scope(Scope::Conversation) != Some(self.connection.origin.conversation_id())
            || proposal.scope(Scope::Session) != Some(self.connection.origin.session_id())
            || proposal.scope(Scope::Connection) != Some(self.connection.origin.connection_id())
            || proposal.expected_revision() != Some(scope.stream_revision)
        {
            return Err(DispatchError::Authority);
        }
        let p = &proposal.original()["payload"];
        let setup = &p["substrate_setup"];
        if p["substrate"] != "quic"
            || p["by_participant"] != self.connection.origin.participant_id()
            || setup["kind"] != "native-prepared/1"
            || setup["revision"].as_u64() != scope.stream_revision.checked_add(1)
            || setup["offer_id"]
                .as_str()
                .is_none_or(|id| id.is_empty() || id.len() > 128)
        {
            return Err(DispatchError::Authority);
        }
        let streams = setup["streams"]
            .as_array()
            .ok_or(DispatchError::Authority)?;
        if streams.is_empty()
            || streams.len() > 32
            || streams.len() != self.media().selected().len()
        {
            return Err(DispatchError::Authority);
        }
        let mut ids = BTreeSet::new();
        let mut handles = BTreeSet::new();
        for stream in streams {
            let id = stream["strm_id"].as_str().ok_or(DispatchError::Authority)?;
            let selected = self
                .media()
                .selected()
                .iter()
                .find(|s| s.requested.logical_id.as_str() == id)
                .ok_or(DispatchError::Authority)?;
            let handle = stream["stream_local_id"]
                .as_u64()
                .ok_or(DispatchError::Authority)?;
            if !ids.insert(id)
                || !handles.insert(handle)
                || !(1..=65535).contains(&handle)
                || stream["kind"] != kind(selected.requested.medium)
                || stream["direction"] != direction(selected.requested.direction)
                || stream["codec"] != crate::peer_authority::native_stream_codec(&selected.format)?
            {
                return Err(DispatchError::Authority);
            }
        }
        let offered=streams.iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
        if p["streams_offered"] != json!(offered) {
            return Err(DispatchError::Authority);
        }
        let mut saved = self
            .proposal
            .try_lock()
            .map_err(|_| DispatchError::Authority)?;
        if saved
            .as_ref()
            .is_some_and(|old| old.original() != proposal.original())
        {
            return Err(DispatchError::Authority);
        }
        *saved = Some(proposal.clone());
        Ok(())
    }
}
fn direction(value: rvoip_core::stream_lifecycle::StreamDirection) -> &'static str {
    use rvoip_core::stream_lifecycle::StreamDirection::*;
    match value {
        Send => "sendonly",
        Receive => "recvonly",
        Both => "sendrecv",
    }
}
impl Policy {
    pub(crate) fn reconciliation_grant(
        self: &Arc<Self>,
        domain: Arc<()>,
        entry: Arc<Connection>,
        scope: &NativeRequestScope,
        plan: NativeSubscriptionPlan,
        operation: String,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Arc<Grant>, DispatchError> {
        let source = Arc::new(
            self.store
                .try_native_reconciliation_read(&entry.origin)
                .map_err(|_| DispatchError::Authority)?,
        );
        let media = plan.media().clone();
        let resources = plan.changes_resources();
        self.server_grant(
            domain,
            entry,
            scope,
            Source::Subscription(source),
            Some(plan),
            media,
            resources,
            operation,
            cancel,
        )
    }
    pub(crate) fn codec_grant(
        self: &Arc<Self>,
        domain: Arc<()>,
        entry: Arc<Connection>,
        scope: &NativeRequestScope,
        source: Arc<NativeSessionCodecRead>,
        plan: &NativeSessionCodecPlan,
        operation: String,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Arc<Grant>, DispatchError> {
        if plan.expected_revision() != scope.stream_revision {
            return Err(DispatchError::Authority);
        }
        self.server_grant(
            domain,
            entry,
            scope,
            Source::Codec(source),
            None,
            plan.media().clone(),
            plan.changes_resources(),
            operation,
            cancel,
        )
    }
    fn server_grant(
        self: &Arc<Self>,
        domain: Arc<()>,
        entry: Arc<Connection>,
        scope: &NativeRequestScope,
        source: Source,
        plan: Option<NativeSubscriptionPlan>,
        media: NativeOfferPlan,
        resources: bool,
        operation: String,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Arc<Grant>, DispatchError> {
        let view = source.media().target();
        let s = &scope.origin;
        if source.stream_revision() != scope.stream_revision
            || plan
                .as_ref()
                .is_some_and(|p| p.selection().expected_revision() != scope.stream_revision)
            || scope.credential_revision != entry.origin.credential_epoch() + 1
            || s.media.connection.as_str() != entry.origin.runtime_connection_id()
            || s.media.session.as_str() != entry.origin.session_id()
            || s.media.wire_connection != entry.origin.connection_id()
            || s.media.wire_session != entry.origin.session_id()
            || s.media.wire_conversation.as_deref() != Some(entry.origin.conversation_id())
            || s.media.owner != entry.principal.ownership_key()
            || scope.conversation.as_str() != entry.origin.conversation_id()
            || scope.connection_generation != entry.origin.generation()
            || s.participant.as_str() != entry.origin.participant_id()
            || s.membership_epoch != entry.origin.membership_epoch()
            || s.device.as_str() != view.device_id
            || s.endpoint.as_str() != view.connection.endpoint_id
            || s.identity.as_str() != view.session.actor.identity_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &s.principal,
                &entry.principal,
            )
        {
            return Err(DispatchError::Authority);
        }
        let entries = self
            .entries
            .try_lock()
            .map_err(|_| DispatchError::Authority)?
            .values()
            .filter(|e| {
                e.origin.session_id() == entry.origin.session_id()
                    && e.origin.conversation_id() == entry.origin.conversation_id()
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut sources = BTreeMap::new();
        for e in entries {
            match self
                .store
                .try_native_media_read(&entry.origin, Some(&e.origin))
            {
                Ok(read) => {
                    sources.insert(
                        ConnectionId::from_string(e.origin.runtime_connection_id().to_owned()),
                        StreamSource::Native(e, Arc::new(read)),
                    );
                }
                Err(e) if e.code == "forbidden" => (),
                Err(_) => return Err(DispatchError::Authority),
            }
        }
        for phone in self
            .telephone_sources(&entry)
            .map_err(|_| DispatchError::Authority)?
        {
            let origin = phone.established_origin().ok_or(DispatchError::Authority)?;
            let id = ConnectionId::from_string(origin.snapshot().runtime_connection_id.clone());
            match self
                .store
                .try_telephone_native_media_read(origin, entry.origin.clone())
            {
                Ok(read) => {
                    sources.insert(id, StreamSource::Telephone(phone, Arc::new(read)));
                }
                Err(error) if error.code == "forbidden" => (),
                Err(_) => return Err(DispatchError::Authority),
            }
        }
        for phone in self
            .initial_sources(&entry)
            .map_err(|_| DispatchError::Authority)?
        {
            let origin = phone.origin().map_err(|_| DispatchError::Authority)?;
            match self
                .store
                .try_initial_telephone_native_read(origin, entry.origin.clone())
            {
                Ok(read) => {
                    sources.insert(
                        phone.connection(),
                        StreamSource::Initial(phone, Arc::new(read)),
                    );
                }
                Err(error) if error.code == "forbidden" => (),
                Err(_) => return Err(DispatchError::Authority),
            }
        }
        let deadline = tokio::time::Instant::now()
            + Duration::from_secs(
                (source.media().expires_at() - chrono::Utc::now().timestamp()).clamp(0, 30) as u64,
            );
        let grant = Arc::new(Grant {
            policy: self.clone(),
            domain,
            connection: entry,
            source,
            plan,
            media,
            resources,
            operation,
            scope: NativeRequestScope {
                origin: scope.origin.clone(),
                stream_revision: scope.stream_revision,
                conversation: scope.conversation.clone(),
                connection_generation: scope.connection_generation,
                credential_revision: scope.credential_revision,
            },
            deadline,
            cancel,
            sources,
            proposal: Mutex::new(None),
        });
        grant.owner(false).map_err(|_| DispatchError::Authority)?;
        Ok(grant)
    }
    pub(super) fn reconciliation_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        inv: Arc<NativeSubscriptionInvocation>,
    ) -> ControlResult<MediaControl> {
        let grant = inv
            .authority::<Arc<Grant>>()
            .ok_or(StreamControlError::Authority)?
            .clone();
        if !inv.current()
            || !grant.matches(inv.scope())
            || inv.operation() != grant.operation
            || !std::ptr::eq(self, Arc::as_ptr(&grant.policy))
            || inv.plan().media().selected() != grant.media().selected()
            || grant
                .plan
                .as_ref()
                .is_none_or(|p| inv.plan().selection().routing() != p.selection().routing())
            || grant.source.subscription().is_err()
        {
            return Err(StreamControlError::Authority);
        }
        self.scope(scope, &grant.connection)?;
        super::streams::check_plan(
            grant.plan.as_ref().ok_or(StreamControlError::Authority)?,
            intent,
        )?;
        let owner = grant.owner(false)?;
        MediaControl::new(
            Box::new(Held {
                inv: Invocation::Subscription(inv.clone()),
                grant,
                owner,
            }),
            inv.cancellation(),
            inv.deadline(),
        )
    }
    pub(super) fn codec_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        inv: Arc<NativeSessionCodecInvocation>,
    ) -> ControlResult<MediaControl> {
        let grant = inv
            .authority::<Arc<Grant>>()
            .ok_or(StreamControlError::Authority)?
            .clone();
        if !inv.current()
            || !grant.matches(inv.scope())
            || inv.operation() != grant.operation
            || !std::ptr::eq(self, Arc::as_ptr(&grant.policy))
            || grant.source.codec().is_err()
            || grant.plan.is_some()
            || inv.plan().media().selected() != grant.media().selected()
            || !inv.plan().changes_resources()
        {
            return Err(StreamControlError::Authority);
        }
        self.scope(scope, &grant.connection)?;
        match intent {
            MediaControlIntent::Change {
                expected,
                desired,
                replace,
            } if *expected == inv.scope().stream_revision
                && replace.is_empty()
                && desired
                    .iter()
                    .eq(grant.media().selected().iter().map(|s| &s.requested)) =>
            {
                ()
            }
            _ => return Err(StreamControlError::Authority),
        }
        let owner = grant.owner(false)?;
        MediaControl::new(
            Box::new(Held {
                inv: Invocation::Codec(inv.clone()),
                grant,
                owner,
            }),
            inv.cancellation(),
            inv.deadline(),
        )
    }
}
enum Invocation {
    Subscription(Arc<NativeSubscriptionInvocation>),
    Codec(Arc<NativeSessionCodecInvocation>),
}
impl Invocation {
    fn current(&self) -> bool {
        match self {
            Self::Subscription(i) => i.current(),
            Self::Codec(i) => i.current(),
        }
    }
    fn scope(&self) -> &NativeRequestScope {
        match self {
            Self::Subscription(i) => i.scope(),
            Self::Codec(i) => i.scope(),
        }
    }
}
struct Held {
    inv: Invocation,
    grant: Arc<Grant>,
    owner: MediaOwner,
}
impl Held {
    fn current(&self) -> ControlResult<()> {
        if !self.inv.current() || !self.grant.matches(self.inv.scope()) {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
}
impl MediaAuthorityLease for Held {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        self.current()?;
        Ok(Box::new(self.grant.owner(false)?))
    }
    fn control_receive(&self, source: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        self.current()?;
        self.grant
            .sources
            .get(&source.connection)
            .ok_or(StreamControlError::Authority)?
            .receive(
                &self.grant.connection,
                &self.grant.scope.origin.principal,
                source,
            )
    }

    fn control_participant_receive(
        &self,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        self.current()?;
        self.grant
            .sources
            .get(&source.connection)
            .ok_or(StreamControlError::Authority)?
            .participant(
                &self.grant.policy,
                &self.grant.connection,
                &self.grant.scope.origin.principal,
                source,
            )
    }
}

pub(crate) fn lease(grant: Arc<Grant>) -> AuthorityLease {
    AuthorityLease::checking(grant.clone(), grant.deadline, move || grant.current())
}
