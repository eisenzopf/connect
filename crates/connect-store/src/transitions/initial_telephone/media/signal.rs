//! Named signal custody for the existing answered SIP leg, never an RTP grant.
use super::*;

/// Direction is relative to the telephone Endpoint, not the SIP gateway host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialTelephoneSignalDirection {
    FromTelephone,
    ToTelephone,
}
impl InitialTelephoneSignalDirection {
    fn permission(self) -> &'static str {
        match self {
            Self::FromTelephone => "dtmf.send",
            Self::ToTelephone => "dtmf.receive",
        }
    }
}

/// The installed adapter must separately authorize the exact other party and
/// freeze the original signal before any write. This read is intentionally not
/// convertible into InitialTelephoneMediaRead or a media/subscription grant.
pub struct InitialTelephoneSignalRead {
    origin: Arc<InitialTelephoneMediaOrigin>,
    direction: InitialTelephoneSignalDirection,
    owner: Arc<InstallationOwnership>,
    _interval: Arc<AuthorityGuardSet>,
    member: MemberView,
    endpoint_actor: Actor,
    route_actor: Actor,
    expires: i64,
}
media_redacted!(InitialTelephoneSignalRead);
impl InitialTelephoneSignalRead {
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires
    }
    pub fn origin(&self) -> &InitialTelephoneMediaOrigin {
        &self.origin
    }
    pub fn direction(&self) -> InitialTelephoneSignalDirection {
        self.direction
    }
    pub fn member(&self) -> &MemberView {
        &self.member
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub(crate) fn endpoint_actor(&self) -> &Actor {
        &self.endpoint_actor
    }
    pub(crate) fn route_actor(&self) -> &Actor {
        &self.route_actor
    }
}
impl Store {
    /// Recheck the original answered/connected incarnation under current
    /// authority. Signal permission is independent of codec, offer and receive
    /// media permissions; a retained origin alone never grants signaling IO.
    pub(crate) fn initial_telephone_signal_scopes(
        &self,
        c: &Connection,
        origin: &InitialTelephoneMediaOrigin,
    ) -> Result<Vec<AuthorityScope>> {
        current_scopes(c, origin.signal.origin(), false)
    }
    pub fn try_initial_telephone_signal_read(
        &self,
        origin: Arc<InitialTelephoneMediaOrigin>,
        direction: InitialTelephoneSignalDirection,
    ) -> Result<InitialTelephoneSignalRead> {
        let required = {
            let c = self.try_connection()?;
            historical_lifetime(self, &c, &origin.signal)?;
            current_scopes(&c, origin.signal.origin(), false)?
        };
        let interval = self.try_acquire_authority(&required)?;
        let c = self.try_connection()?;
        self.initial_telephone_signal_in(&c, origin, direction, interval, crate::now() + 30)
    }
    pub(crate) fn initial_telephone_signal_in(
        &self,
        c: &Connection,
        origin: Arc<InitialTelephoneMediaOrigin>,
        direction: InitialTelephoneSignalDirection,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<InitialTelephoneSignalRead> {
        if !self.authority_covers(
            &interval,
            &current_scopes(c, origin.signal.origin(), false)?,
        ) {
            return Err(Error::forbidden());
        }
        let p = self.initial_telephone_projection(c, &origin, until, ProjectionPurpose::Signal)?;
        p.endpoint_actor.require(direction.permission())?;
        let connected: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND generation=1 AND state IN ('ready','active') AND observed_runtime_id=?2 AND readiness_json IS NOT NULL)",
            params![origin.connection_id(), self.runtime_epoch()],
            |r| r.get(0),
        ).map_err(|_| Error::storage())?;
        if !p.connected || !connected || !p.session.media.iter().any(|m| m == "audio") {
            return Err(Error::forbidden());
        }
        Ok(InitialTelephoneSignalRead {
            origin,
            direction,
            owner: self.require_runtime_owner()?,
            _interval: interval,
            member: p.member,
            endpoint_actor: p.endpoint_actor,
            route_actor: p.route_actor,
            expires: p.expires,
        })
    }
}

/// One complete authority interval for this exact public native/SIP pair.
/// Queue metadata, never this reader. It supplies neither a durable attempt nor
/// an actual transport observation, and cannot be converted into a media read.
pub struct InitialTelephoneNativeSignalRead {
    phone: InitialTelephoneSignalRead,
    native: crate::NativeSignalRead,
}
media_redacted!(InitialTelephoneNativeSignalRead);
impl InitialTelephoneNativeSignalRead {
    pub fn current(&self) -> bool {
        self.phone.current() && self.native.current()
    }
    pub fn phone(&self) -> &InitialTelephoneSignalRead {
        &self.phone
    }
    pub fn native(&self) -> &crate::NativeSignalRead {
        &self.native
    }
    pub fn expires_at(&self) -> i64 {
        self.phone.expires_at()
    }
    pub(crate) fn interval(&self) -> &Arc<AuthorityGuardSet> {
        &self.phone._interval
    }
}
impl Store {
    /// Acquire both sides together, then recheck current named permissions,
    /// original audiences and exact physical identities inside the same fence.
    pub fn try_initial_telephone_native_signal_read(
        &self,
        phone: Arc<InitialTelephoneMediaOrigin>,
        native: Arc<crate::NativeMediaOrigin>,
        direction: InitialTelephoneSignalDirection,
    ) -> Result<InitialTelephoneNativeSignalRead> {
        let c = self.try_connection()?;
        let required = self.initial_telephone_native_signal_scopes(&c, &phone, &native)?;
        let interval = self.try_acquire_authority(&required)?;
        self.initial_telephone_native_signal_in(
            &c,
            phone,
            native,
            direction,
            interval,
            crate::now() + 30,
        )
    }
    pub(crate) fn initial_telephone_native_signal_scopes(
        &self,
        c: &Connection,
        phone: &InitialTelephoneMediaOrigin,
        native: &crate::NativeMediaOrigin,
    ) -> Result<Vec<AuthorityScope>> {
        let source = phone.source();
        if native.conversation_id() != source.request.conversation_id
            || native.session_id() != source.request.session_id
            || native.participant_id() == source.target.participant_id
        {
            return Err(Error::forbidden());
        }
        let (_, mut required, _) = self.native_signal_facts_at(c, native)?;
        required.extend(current_scopes(c, phone.signal.origin(), false)?);
        AuthorityScope::merged(&required)
    }
    pub(crate) fn initial_telephone_native_signal_in(
        &self,
        c: &Connection,
        phone: Arc<InitialTelephoneMediaOrigin>,
        native: Arc<crate::NativeMediaOrigin>,
        direction: InitialTelephoneSignalDirection,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<InitialTelephoneNativeSignalRead> {
        let scopes = self.initial_telephone_native_signal_scopes(c, &phone, &native)?;
        if !self.authority_covers(&interval, &scopes) {
            return Err(Error::forbidden());
        }
        let (_, _, expiry) = self.native_signal_facts_at(c, &native)?;
        let phone = self.initial_telephone_signal_in(
            c,
            phone,
            direction,
            interval.clone(),
            until.min(expiry),
        )?;
        let native_read = self.native_signal_read_in(c, &native, interval, phone.expires_at())?;
        let view = native_read.target();
        self.require_sip_native_signal_context(
            c,
            phone.origin().connection_id(),
            &native,
            view.private_consultation.as_deref(),
        )?;
        let actor = self.native_media_route_actor(c, &native)?;
        actor.require(match direction {
            InitialTelephoneSignalDirection::FromTelephone => "dtmf.receive",
            InitialTelephoneSignalDirection::ToTelephone => "dtmf.send",
        })?;
        if !phone.current()
            || !native_read.current()
            || view.handoff_preparation
            || view.publication_suspended
            || !view.session.session.media.iter().any(|m| m == "audio")
            || !view.endpoint_capabilities.iter().any(|m| m == "audio")
            || !actor.permits_recipient(&phone.member.identity_id)
            || !phone
                .endpoint_actor
                .permits_recipient(&view.session.actor.identity_id)
            || !phone
                .route_actor
                .permits_recipient(&view.session.actor.identity_id)
        {
            return Err(Error::forbidden());
        }
        Ok(InitialTelephoneNativeSignalRead {
            phone,
            native: native_read,
        })
    }
}
