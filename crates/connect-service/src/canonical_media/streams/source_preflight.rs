//! Read-only source authorization before creating a durable stream attempt.
//! This type cannot supply MediaControl or be used by an engine effect.
use super::*;
use rvoip_core::staged_streams::MediaParticipantSource;

struct SourcePreflight {
    policy: Arc<Policy>,
    connection: Arc<Connection>,
    authorized: Arc<AuthorizedEnvelope>,
    target: Arc<NativeMediaRead>,
    sources: BTreeMap<ConnectionId, StreamSource>,
    deadline: tokio::time::Instant,
}
impl SourcePreflight {
    fn current(&self) -> ControlResult<()> {
        if !self
            .authorized
            .authority::<StoreInvocationAuthority>()
            .and_then(|a| a.peer_guard())
            .is_some_and(|p| p.current())
            || !self.target.current()
            || self.authorized.invocation_cancellation().is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn source(&self, id: &ConnectionId) -> ControlResult<&StreamSource> {
        self.current()?;
        self.sources.get(id).ok_or(StreamControlError::Authority)
    }
    fn receive(&self, source: &MediaSource, medium: Medium) -> ControlResult<()> {
        let owner = self.source(&source.connection)?.receive(
            &self.connection,
            self.authorized.principal(),
            source,
        )?;
        if !owner.may_route_source || !owner.send_media.contains(&medium) {
            return Err(StreamControlError::Authority);
        }
        self.current()
    }
    fn participant(&self, source: &MediaParticipantSource) -> ControlResult<()> {
        let owner = self.source(&source.connection)?.participant(
            &self.policy,
            &self.connection,
            self.authorized.principal(),
            source,
        )?;
        if !owner.may_route_source || !owner.send_media.contains(&source.medium) {
            return Err(StreamControlError::Authority);
        }
        self.current()
    }
}
impl Policy {
    pub(crate) fn preflight_subscription_sources(
        self: &Arc<Self>,
        core: &rvoip_core::Orchestrator,
        a: Arc<AuthorizedEnvelope>,
        id: &ConnectionId,
        plan: &rvoip_core::staged_streams::subscription_plan::MediaSubscriptionPlan,
        deadline: tokio::time::Instant,
    ) -> connect_protocol::Result<()> {
        use connect_protocol::Error;
        let proof = a
            .authority::<StoreInvocationAuthority>()
            .and_then(|h| h.peer_guard())
            .ok_or_else(Error::forbidden)?;
        let connection = self.entry(id).map_err(|error| match error {
            StreamControlError::NotReady => {
                Error::new("authority-busy", "Subscription source metadata is busy")
            }
            _ => Error::forbidden(),
        })?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| Error::storage())?
            .values()
            .filter(|s| {
                s.origin.conversation_id() == connection.origin.conversation_id()
                    && s.origin.session_id() == connection.origin.session_id()
            })
            .cloned()
            .collect::<Vec<_>>();
        let origins = entries
            .iter()
            .map(|s| s.origin.as_ref())
            .collect::<Vec<_>>();
        let (target, reads) = self.store.native_stream_preflight_media_reads_with_peer(
            &connection.origin,
            &origins,
            proof,
        )?;
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
            let origin = phone.established_origin().ok_or_else(Error::forbidden)?;
            let id = ConnectionId::from_string(origin.snapshot().runtime_connection_id.clone());
            match self.store.telephone_native_stream_preflight_read_with_peer(
                origin,
                &connection.origin,
                proof,
            ) {
                Ok(read) => {
                    sources.insert(id, StreamSource::Telephone(phone, Arc::new(read)));
                }
                Err(error) if error.code == "forbidden" => (),
                Err(error) => return Err(error),
            }
        }
        for phone in self.initial_sources(&connection)? {
            let origin = phone.origin().map_err(|_| Error::forbidden())?;
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
        let preflight = SourcePreflight {
            policy: self.clone(),
            connection,
            authorized: a,
            target: Arc::new(target),
            sources,
            deadline,
        };
        preflight.current().map_err(|_| Error::forbidden())?;
        let sid = SessionId::from_string(preflight.connection.origin.session_id().to_owned());
        core.validate_staged_media_selection_sources(
            &sid,
            id,
            plan,
            |source, medium| preflight.receive(source, medium),
            |source| preflight.participant(source),
        )
        .map_err(|error| match error {
            rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                StreamControlError::Backpressure,
            ) => Error::new("authority-busy", "Subscription source metadata is busy"),
            rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                StreamControlError::Unsupported,
            ) => Error::new(
                "incompatible-capabilities",
                "Subscription capabilities have no usable intersection",
            ),
            rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                StreamControlError::Model(
                    rvoip_core::stream_lifecycle::MediaTransitionError::Conflict,
                ),
            ) => Error::new(
                "conflict",
                "Subscription source revision or binding changed",
            ),
            _ => Error::forbidden(),
        })?;
        preflight.current().map_err(|_| Error::forbidden())
    }
}
