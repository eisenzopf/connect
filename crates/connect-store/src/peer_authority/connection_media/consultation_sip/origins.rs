//! Purpose-bound media authority for an unchanged SIP audience outsider.
use super::*;
use crate::{TelephoneMediaOrigin, TelephoneMediaRead};
#[derive(Clone)]
pub enum SipAudienceOrigin {
    Initial(Arc<InitialTelephoneMediaOrigin>),
    Established(Arc<TelephoneMediaOrigin>),
}
pub enum SipAudienceRead {
    Initial(Arc<InitialTelephoneMediaRead>),
    Established(Arc<TelephoneMediaRead>),
}
impl SipAudienceRead {
    pub fn session(&self) -> &connect_protocol::session_control::SessionView {
        match self {
            Self::Initial(r) => r.session(),
            Self::Established(r) => r.session(),
        }
    }

    pub fn may_receive(&self) -> bool {
        self.current()
            && match self {
                Self::Initial(r) => r.may_receive(),
                Self::Established(r) => r.route_actor().permits("stream.subscribe"),
            }
    }
    pub fn current(&self) -> bool {
        match self {
            Self::Initial(r) => r.current(),
            Self::Established(r) => r.current(),
        }
    }
    pub fn member(&self) -> &connect_protocol::membership::MemberView {
        match self {
            Self::Initial(r) => r.member(),
            Self::Established(r) => r.member(),
        }
    }
    pub(in crate::peer_authority::connection_media) fn permits_recipient(&self, id: &str) -> bool {
        match self {
            Self::Initial(r) => {
                r.endpoint_actor().permits_recipient(id) && r.route_actor().permits_recipient(id)
            }
            Self::Established(r) => r.route_actor().permits_recipient(id),
        }
    }
}
impl SipAudienceOrigin {
    pub fn participant_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().target.participant_id,
            Self::Established(o) => &o.snapshot().participant_id,
        }
    }
    pub fn membership_epoch(&self) -> u64 {
        match self {
            Self::Initial(o) => o.source().target.membership_epoch,
            Self::Established(o) => o.snapshot().membership_epoch,
        }
    }
    pub fn connection_id(&self) -> &str {
        match self {
            Self::Initial(o) => o.connection_id(),
            Self::Established(o) => &o.snapshot().connection_id,
        }
    }
    pub fn runtime_connection_id(&self) -> &str {
        match self {
            Self::Initial(o) => o.runtime_connection_id(),
            Self::Established(o) => &o.snapshot().runtime_connection_id,
        }
    }
    pub fn session_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().request.session_id,
            Self::Established(o) => &o.snapshot().session_id,
        }
    }
    pub(super) fn conversation_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().request.conversation_id,
            Self::Established(o) => &o.snapshot().conversation_id,
        }
    }
    pub(super) fn generation(&self) -> u64 {
        match self {
            Self::Initial(_) => 1,
            Self::Established(o) => o.snapshot().generation,
        }
    }
    pub(in crate::peer_authority::connection_media) fn facts(
        &self,
        store: &Store,
        c: &Connection,
    ) -> Result<(Vec<AuthorityScope>, serde_json::Value, i64)> {
        match self {
            Self::Initial(o) => store.initial_telephone_audience_facts(c, o),
            Self::Established(o) => store.telephone_audience_facts(c, o),
        }
    }
    pub(super) fn read(
        &self,
        store: &Store,
        c: &Connection,
        held: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<SipAudienceRead> {
        match self {
            Self::Initial(o) => store
                .initial_telephone_media_in(c, o.clone(), held, until)
                .map(|r| SipAudienceRead::Initial(Arc::new(r))),
            Self::Established(o) => store
                .telephone_media_established_in(c, o.clone(), held, until)
                .map(|r| SipAudienceRead::Established(Arc::new(r))),
        }
    }
}
