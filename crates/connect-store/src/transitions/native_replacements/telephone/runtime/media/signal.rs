//! Named signal permission on the actual established SIP incarnation, not RTP.
use super::lifetime::{ProjectionPurpose, TelephoneMediaOrigin};
use super::*;
use crate::{
    AuthorityGuardSet, AuthorityScope, InstallationOwnership, NativeMediaOrigin, NativeSignalRead,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TelephoneSignalDirection {
    FromTelephone,
    ToTelephone,
}
impl TelephoneSignalDirection {
    fn permission(self) -> &'static str {
        match self {
            Self::FromTelephone => "dtmf.send",
            Self::ToTelephone => "dtmf.receive",
        }
    }
    fn native_permission(self) -> &'static str {
        match self {
            Self::FromTelephone => "dtmf.receive",
            Self::ToTelephone => "dtmf.send",
        }
    }
}
pub struct TelephoneSignalRead {
    origin: Arc<TelephoneMediaOrigin>,
    direction: TelephoneSignalDirection,
    owner: Arc<InstallationOwnership>,
    interval: Arc<AuthorityGuardSet>,
    actor: Actor,
    member: connect_protocol::membership::MemberView,
    expires: i64,
}
pub struct TelephoneNativeSignalRead {
    phone: TelephoneSignalRead,
    native: NativeSignalRead,
}
redacted!(TelephoneSignalRead, TelephoneNativeSignalRead);
impl TelephoneSignalRead {
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires
    }
    pub fn origin(&self) -> &TelephoneMediaOrigin {
        &self.origin
    }
    pub fn snapshot(&self) -> &TelephoneConnectionSnapshot {
        self.origin.snapshot()
    }
    pub fn direction(&self) -> TelephoneSignalDirection {
        self.direction
    }
    pub fn member(&self) -> &connect_protocol::membership::MemberView {
        &self.member
    }
    pub fn route_actor(&self) -> &Actor {
        &self.actor
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
}
impl TelephoneNativeSignalRead {
    pub fn current(&self) -> bool {
        self.phone.current() && self.native.current()
    }
    pub fn phone(&self) -> &TelephoneSignalRead {
        &self.phone
    }
    pub fn native(&self) -> &NativeSignalRead {
        &self.native
    }
    pub fn expires_at(&self) -> i64 {
        self.phone.expires_at()
    }
    pub(crate) fn interval(&self) -> &Arc<AuthorityGuardSet> {
        &self.phone.interval
    }
}
impl Store {
    pub fn try_telephone_signal_read(
        &self,
        origin: Arc<TelephoneMediaOrigin>,
        direction: TelephoneSignalDirection,
    ) -> Result<TelephoneSignalRead> {
        let c = self.try_connection()?;
        let scopes = self.telephone_established_scopes(&c, &origin)?;
        let interval = self.try_acquire_authority(&scopes)?;
        self.telephone_signal_in(&c, origin, direction, interval, crate::now() + 30)
    }
    pub(crate) fn telephone_signal_in(
        &self,
        c: &Connection,
        origin: Arc<TelephoneMediaOrigin>,
        direction: TelephoneSignalDirection,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<TelephoneSignalRead> {
        let required = self.telephone_established_scopes(c, &origin)?;
        if !self.authority_covers(&interval, &required) {
            return Err(Error::forbidden());
        }
        let projected = self.telephone_projection(c, &origin, until, ProjectionPurpose::Signal)?;
        projected.actor.require(direction.permission())?;
        let snapshot = origin.snapshot();
        let device: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND identity_id=?2 AND revoked=0)",
                params![snapshot.device_id, snapshot.identity_id],
                |row| row.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !device {
            return Err(Error::forbidden());
        }
        Ok(TelephoneSignalRead {
            origin,
            direction,
            owner: self.require_runtime_owner()?,
            interval,
            actor: projected.actor,
            member: projected.member,
            expires: projected.expires,
        })
    }
    pub(crate) fn telephone_native_signal_scopes(
        &self,
        c: &Connection,
        phone: &Arc<TelephoneMediaOrigin>,
        native: &Arc<NativeMediaOrigin>,
    ) -> Result<Vec<AuthorityScope>> {
        let s = phone.snapshot();
        if native.conversation_id() != s.conversation_id
            || native.session_id() != s.session_id
            || native.participant_id() == s.participant_id
        {
            return Err(Error::forbidden());
        }
        let mut scopes = self.telephone_established_scopes(c, phone)?;
        scopes.extend(self.native_signal_facts_at(c, native)?.1);
        AuthorityScope::merged(&scopes)
    }
    pub fn try_telephone_native_signal_read(
        &self,
        phone: Arc<TelephoneMediaOrigin>,
        native: Arc<NativeMediaOrigin>,
        direction: TelephoneSignalDirection,
    ) -> Result<TelephoneNativeSignalRead> {
        let c = self.try_connection()?;
        let scopes = self.telephone_native_signal_scopes(&c, &phone, &native)?;
        let interval = self.try_acquire_authority(&scopes)?;
        self.telephone_native_signal_in(&c, phone, native, direction, interval, crate::now() + 30)
    }
    pub(crate) fn telephone_native_signal_in(
        &self,
        c: &Connection,
        phone: Arc<TelephoneMediaOrigin>,
        native: Arc<NativeMediaOrigin>,
        direction: TelephoneSignalDirection,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<TelephoneNativeSignalRead> {
        let scopes = self.telephone_native_signal_scopes(c, &phone, &native)?;
        if !self.authority_covers(&interval, &scopes) {
            return Err(Error::forbidden());
        }
        let expiry = self.native_signal_facts_at(c, &native)?.2;
        let phone =
            self.telephone_signal_in(c, phone, direction, interval.clone(), until.min(expiry))?;
        let native_read = self.native_signal_read_in(c, &native, interval, phone.expires_at())?;
        let actor = self.native_media_route_actor(c, &native)?;
        actor.require(direction.native_permission())?;
        let view = native_read.target();
        self.require_sip_native_pair_context(
            c,
            &phone.snapshot().connection_id,
            &native,
            view.private_consultation.as_deref(),
        )?;
        if !phone.current()
            || !native_read.current()
            || view.handoff_preparation
            || view.publication_suspended
            || !view.session.session.media.iter().any(|m| m == "audio")
            || !view.endpoint_capabilities.iter().any(|m| m == "audio")
            || !actor.permits_recipient(&phone.member.identity_id)
            || !phone
                .actor
                .permits_recipient(&view.session.actor.identity_id)
        {
            return Err(Error::forbidden());
        }
        Ok(TelephoneNativeSignalRead {
            phone,
            native: native_read,
        })
    }
}
