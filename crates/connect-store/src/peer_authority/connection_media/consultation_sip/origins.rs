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
            Self::Initial(o) => &o.source().session_id,
            Self::Established(o) => &o.snapshot().session_id,
        }
    }
    pub(super) fn conversation_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().conversation_id,
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
    pub(in crate::peer_authority::connection_media) fn facts_for(
        &self,
        store: &Store,
        c: &Connection,
        purpose: crate::connection_control::private_consultation::Purpose,
    ) -> Result<(Vec<AuthorityScope>, serde_json::Value, i64)> {
        if purpose.media() {
            return self.facts(store, c);
        }
        match self {
            Self::Initial(o) => store.initial_telephone_audience_facts_for(c, o, purpose),
            Self::Established(o) => store.telephone_audience_facts_for(c, o, purpose),
        }
    }
    /// A return may close only its exact private candidate without that
    /// candidate's media rights. Other SIP programmes keep their media checks.
    pub(in crate::peer_authority::connection_media) fn return_facts(
        &self,
        store: &Store,
        c: &Connection,
        candidate: &str,
    ) -> Result<(Vec<AuthorityScope>, serde_json::Value, i64)> {
        if self.connection_id() != candidate {
            return self.facts(store, c);
        }
        match self {
            Self::Initial(o) if o.source().is_private_candidate() => self.facts_for(
                store,
                c,
                crate::connection_control::private_consultation::Purpose::Presence,
            ),
            _ => Err(Error::forbidden()),
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
