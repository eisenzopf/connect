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
}
impl Store {
    /// Recheck the original answered/connected incarnation under current
    /// authority. Signal permission is independent of codec, offer and receive
    /// media permissions; a retained origin alone never grants signaling IO.
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
        if !self.authority_covers(
            &interval,
            &current_scopes(&c, origin.signal.origin(), false)?,
        ) {
            return Err(Error::forbidden());
        }
        let p = self.initial_telephone_projection(
            &c,
            &origin,
            crate::now() + 30,
            ProjectionPurpose::Signal,
        )?;
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
            expires: p.expires,
        })
    }
}
