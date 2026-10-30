//! A separate exact subscription command lease; no initial-offer privilege.
mod source_preflight;
use super::sources::StreamSource;
use super::*;
use rvoip_uctp::v2::native_negotiation::NativeOfferPlan;
use rvoip_uctp::v2::native_subscriptions::NativeSubscriptionPlan;
#[derive(Clone)]
pub(crate) enum Plan {
    Subscriptions(NativeSubscriptionPlan),
    RtcSubscriptions(rvoip_core::staged_streams::subscription_plan::MediaSubscriptionPlan),
    Codecs(NativeOfferPlan),
    Gates(rvoip_uctp::v2::native_control::NativeControlPlan),
    RtcGates(RtcGatePlan),
}
impl Plan {
    pub(crate) fn media(&self) -> Option<&NativeOfferPlan> {
        match self {
            Self::Subscriptions(p) => Some(p.media()),
            Self::Codecs(p) => Some(p),
            Self::Gates(_) | Self::RtcGates(_) | Self::RtcSubscriptions(_) => None,
        }
    }
}

/// Exact RTC wire gate intent; it carries no authorization or native adapter grant.
#[derive(Clone)]
pub(crate) struct RtcGatePlan {
    pub expected: u64,
    pub streams: BTreeSet<rvoip_core::ids::StreamId>,
    pub action: rvoip_core::stream_lifecycle::StreamGateAction,
}

#[derive(Clone)]
pub(crate) struct StreamInvocation {
    policy: Arc<Policy>,
    connection: Arc<Connection>,
    authorized: Arc<AuthorizedEnvelope>,
    plan: Plan,
    revision: u64,
    target: Arc<NativeMediaRead>,
    sources: BTreeMap<ConnectionId, StreamSource>,
    deadline: tokio::time::Instant,
}
impl StreamInvocation {
    fn current(&self) -> ControlResult<()> {
        let authority = self
            .authorized
            .authority::<StoreInvocationAuthority>()
            .ok_or(StreamControlError::Authority)?;
        let wire =
            crate::canonical_native_streams::intent(self.authorized.envelope()).map_err(denied)?;
        if !authority.peer_guard().is_some_and(|p| p.current())
            || !self.target.current()
            || self.authorized.invocation_cancellation().is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
            || wire.connection_id != self.connection.origin.connection_id()
            || wire.conversation_id != self.connection.origin.conversation_id()
            || wire.session_id != self.connection.origin.session_id()
            || Some(self.revision) != wire.expected_revision
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn owner(&self, route: bool) -> ControlResult<MediaOwner> {
        self.current()?;
        let owner = self.connection.owner_with_remote(
            self.target.target(),
            self.authorized.principal(),
            route,
            self.plan.media().map(|p| p.remote_capabilities()),
        )?;
        self.connection
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)
    }
    /// Call only after this exact native command returned success. Remember
    /// its confirmed selected formats for existing packet IO; future choices
    /// still use the broker's latest declaration and current Store ceilings.
    pub(crate) fn remember_confirmed_selection(&self) {
        let Some(media) = self.plan.media() else {
            return;
        };
        let mut known = self
            .connection
            .negotiated_remote
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        for stream in media.selected() {
            let medium = stream.requested.medium;
            let add = |set: &mut Vec<MediaCapability>| {
                if let Some(cap) = set.iter_mut().find(|c| c.medium == medium) {
                    if !cap.formats.contains(&stream.format) {
                        cap.formats.push(stream.format.clone());
                    }
                } else {
                    set.push(MediaCapability {
                        medium,
                        formats: vec![stream.format.clone()],
                    });
                }
            };
            if matches!(
                stream.requested.direction,
                rvoip_core::stream_lifecycle::StreamDirection::Receive
                    | rvoip_core::stream_lifecycle::StreamDirection::Both
            ) {
                add(&mut known.send);
            }
            if matches!(
                stream.requested.direction,
                rvoip_core::stream_lifecycle::StreamDirection::Send
                    | rvoip_core::stream_lifecycle::StreamDirection::Both
            ) {
                add(&mut known.receive);
            }
        }
        known.max_streams = known.max_streams.max(media.selected().len()).min(32);
    }
}
impl Policy {
    pub(crate) fn stream_invocation(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        plan: Plan,
        lease: &connect_store::NativeStreamLease,
        deadline: tokio::time::Instant,
    ) -> connect_protocol::Result<Arc<StreamInvocation>> {
        let proof = a
            .authority::<StoreInvocationAuthority>()
            .and_then(|h| h.peer_guard())
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let connection = self
            .entry(&ConnectionId::from_string(
                lease.runtime_connection_id().to_owned(),
            ))
            .map_err(|error| match error {
                StreamControlError::NotReady => connect_protocol::Error::new(
                    "authority-busy",
                    "Native stream invocation metadata is busy",
                ),
                _ => connect_protocol::Error::forbidden(),
            })?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .values()
            .filter(|s| {
                s.origin.conversation_id() == connection.origin.conversation_id()
                    && s.origin.session_id() == connection.origin.session_id()
            })
            .cloned()
            .collect::<Vec<_>>();
        let origins = entries
            .iter()
            .map(|source| source.origin.as_ref())
            .collect::<Vec<_>>();
        let (target, reads) = self.store.native_stream_media_reads_with_peer(
            &connection.origin,
            &origins,
            proof,
            lease,
        )?;
        let target = Arc::new(target);
        let mut sources = BTreeMap::new();
        for (source, read) in entries.into_iter().zip(reads) {
            if let Some(read) = read {
                sources.insert(
                    ConnectionId::from_string(source.origin.runtime_connection_id().to_owned()),
                    StreamSource::Native(source, Arc::new(read)),
                );
            }
        }
        for phone in self.telephone_sources(&connection)? {
            let origin = phone
                .established_origin()
                .ok_or_else(connect_protocol::Error::forbidden)?;
            let id = ConnectionId::from_string(origin.snapshot().runtime_connection_id.clone());
            match self.store.telephone_native_stream_read_with_peer(
                origin,
                &connection.origin,
                proof,
                lease,
            ) {
                Ok(read) => {
                    sources.insert(id, StreamSource::Telephone(phone, Arc::new(read)));
                }
                Err(error) if error.code == "forbidden" => (),
                Err(error) => return Err(error),
            }
        }
        for phone in self.initial_sources(&connection)? {
            let origin = phone
                .origin()
                .map_err(|_| connect_protocol::Error::forbidden())?;
            match self
                .store
                .try_initial_telephone_native_read(origin, connection.origin.clone())
            {
                Ok(read) => {
                    sources.insert(
                        phone.connection(),
                        StreamSource::Initial(phone, Arc::new(read)),
                    );
                }
                Err(error) if error.code == "forbidden" => (),
                Err(error) => return Err(error),
            }
        }
        let invocation = Arc::new(StreamInvocation {
            policy: self.clone(),
            connection,
            authorized: a,
            plan,
            revision: lease.expected_revision(),
            target,
            sources,
            deadline,
        });
        invocation
            .current()
            .map_err(|_| connect_protocol::Error::forbidden())?;
        Ok(invocation)
    }
    pub(super) fn stream_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        i: &StreamInvocation,
    ) -> ControlResult<MediaControl> {
        if !std::ptr::eq(self, Arc::as_ptr(&i.policy)) {
            return Err(StreamControlError::Authority);
        }
        self.scope(scope, &i.connection)?;
        i.current()?;
        let plan = &i.plan;
        tracing::debug!(target:"connect_service::canonical_native_streams",sources=i.sources.len(),"Native stream control plan checking");
        match plan {
            Plan::Gates(plan) => match intent {
                MediaControlIntent::Gate {
                    expected,
                    streams,
                    action,
                } if *expected == i.revision
                    && *expected == plan.expected_revision()
                    && *streams == plan.streams()
                    && *action == plan.action() =>
                {
                    ()
                }
                _ => return Err(StreamControlError::Authority),
            },
            Plan::RtcGates(plan) => match intent {
                MediaControlIntent::Gate {
                    expected,
                    streams,
                    action,
                } if *expected == i.revision
                    && *expected == plan.expected
                    && *streams == &plan.streams
                    && *action == plan.action => {}
                _ => return Err(StreamControlError::Authority),
            },
            Plan::Subscriptions(plan) => check_plan(plan, intent)?,
            Plan::RtcSubscriptions(plan) => match intent {
                MediaControlIntent::Subscriptions { expected, change }
                    if *expected == i.revision
                        && *expected == plan.expected_revision()
                        && *change == plan.routing() => {}
                _ => return Err(StreamControlError::Authority),
            },
            Plan::Codecs(plan) => match intent {
                MediaControlIntent::Change {
                    expected,
                    desired,
                    replace,
                } if *expected == i.revision
                    && replace.is_empty()
                    && desired
                        .iter()
                        .eq(plan.selected().iter().map(|s| &s.requested)) =>
                {
                    ()
                }
                _ => return Err(StreamControlError::Authority),
            },
        }
        MediaControl::new(
            Box::new(StreamHeld {
                invocation: Arc::new(i.clone()),
                owner: i.owner(false)?,
            }),
            i.authorized.invocation_cancellation(),
            i.deadline,
        )
    }
}
pub(super) fn check_plan(
    plan: &NativeSubscriptionPlan,
    intent: &MediaControlIntent<'_>,
) -> ControlResult<()> {
    match intent {
        MediaControlIntent::ChangeSubscriptions {
            expected,
            desired,
            replace,
            change,
        } if *expected == plan.selection().expected_revision()
            && replace.is_empty()
            && desired
                .iter()
                .eq(plan.media().selected().iter().map(|s| &s.requested))
            && *change == plan.selection().routing() =>
        {
            ()
        }
        MediaControlIntent::Subscriptions { expected, change }
            if *expected == plan.selection().expected_revision()
                && *change == plan.selection().routing() =>
        {
            ()
        }
        MediaControlIntent::AudioSubscriptions { expected, change }
            if *expected == plan.selection().expected_revision()
                && plan.selection().routing().selection.is_none()
                && plan.selection().routing().direct.is_none()
                && plan.selection().routing().audio.as_ref() == Some(*change) =>
        {
            ()
        }
        MediaControlIntent::DirectSubscriptions { expected, change }
            if *expected == plan.selection().expected_revision()
                && plan.selection().routing().selection.is_none()
                && plan.selection().routing().audio.is_none()
                && plan.selection().routing().direct.as_ref() == Some(*change) =>
        {
            ()
        }
        _ => {
            tracing::debug!(target:"connect_service::canonical_native_streams","Native stream control plan mismatch");
            return Err(StreamControlError::Authority);
        }
    }
    Ok(())
}
struct StreamHeld {
    invocation: Arc<StreamInvocation>,
    owner: MediaOwner,
}
impl MediaAuthorityLease for StreamHeld {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        Ok(Box::new(self.invocation.owner(false)?))
    }
    fn control_participant_receive(
        &self,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        i.current()?;
        i.sources
            .get(&source.connection)
            .ok_or(StreamControlError::Authority)?
            .participant(&i.policy, &i.connection, i.authorized.principal(), source)
    }

    fn control_receive(&self, source: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        i.current()?;
        tracing::debug!(target:"connect_service::canonical_native_streams",known=i.sources.contains_key(&source.connection),"Native stream receive source checking");
        i.sources
            .get(&source.connection)
            .ok_or(StreamControlError::Authority)?
            .receive(&i.connection, i.authorized.principal(), source)
    }
}

impl MediaAuthorityLease for ParticipantStreamRead {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
struct ParticipantStreamRead {
    owner: MediaOwner,
    _read: NativeMediaRead,
}
impl Policy {
    pub(super) fn participant_receive_owner(
        &self,
        target: &Connection,
        target_view: &PeerConnectionProjection,
        principal: &AuthenticatedPrincipal,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
        entry: &Connection,
        source_view: &PeerConnectionProjection,
    ) -> ControlResult<MediaOwner> {
        if source.conversation.as_str() != entry.origin.conversation_id()
            || source.session.as_str() != entry.origin.session_id()
            || source.connection.as_str() != entry.origin.runtime_connection_id()
            || source.participant.as_str() != source_view.connection.participant_id
            || source.membership_epoch != source_view.connection.membership_epoch
            || source.identity.as_str() != source_view.session.actor.identity_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &source.principal,
                &entry.principal,
            )
            || !matches!(source_view.connection.state.as_str(), "ready" | "active")
        {
            return Err(StreamControlError::Authority);
        }
        let source_owner = entry.owner(source_view, &source.principal, false)?;
        let _source_owner = entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(source_owner)?;
        // Core uses one arbitrary live Connection as the Participant's identity
        // witness. A retained receive intent belongs to that member/epoch, not
        // to the witness endpoint's currently selected media. Current accepted
        // Session capability bounds the intent; actual future publication and
        // each routed stream still need their own endpoint/profile/port grant.
        let owner = target.owner(target_view, principal, true)?;
        let owner = target
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        if !owner.send_media.contains(&source.medium) {
            return Err(StreamControlError::Authority);
        }
        let supported = crate::canonical_native::base_profile().map_err(denied)?;
        if !supported.engine_capabilities().receive.iter().any(|cap| {
            cap.medium == source.medium
                && cap
                    .formats
                    .iter()
                    .any(|format| session_allows(source_view, source.medium, format))
        }) {
            return Err(StreamControlError::Unsupported);
        }
        Ok(owner)
    }
    pub(super) fn fresh_participant_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let target = self.entry(scope.connection)?;
        self.scope(scope, &target)?;
        if let Some(phone) = self.initial_target(&source.connection)? {
            return phone.native_participant_receive(scope, source, &target);
        }
        if let Some(phone) = self.telephone_target(&source.connection)? {
            let origin = phone
                .established_origin()
                .ok_or(StreamControlError::NotReady)?;
            let read = Arc::new(
                self.store
                    .try_telephone_native_media_read(origin, target.origin.clone())
                    .map_err(denied)?,
            );
            let source_read = StreamSource::Telephone(phone, read);
            let owner = *source_read.participant(self, &target, scope.principal, source)?;
            return Ok(Box::new(PhoneParticipantRead {
                owner,
                _source: source_read,
            }));
        }
        let entry = self.entry(&source.connection)?;
        let read = self
            .store
            .try_native_media_read(&target.origin, Some(&entry.origin))
            .map_err(denied)?;
        if !read.permits_subscriptions() {
            return Err(StreamControlError::Authority);
        }
        let owner = self.participant_receive_owner(
            &target,
            read.target(),
            scope.principal,
            source,
            &entry,
            read.source().ok_or(StreamControlError::Authority)?,
        )?;
        Ok(Box::new(ParticipantStreamRead { owner, _read: read }))
    }
}

struct PhoneParticipantRead {
    owner: MediaOwner,
    _source: StreamSource,
}
impl MediaAuthorityLease for PhoneParticipantRead {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
