//! Physical consultation metadata, never a native projection for a SIP call.
use super::*;

#[derive(Clone)]
pub enum ConsultationMediaOrigin {
    Native(Arc<NativeMediaOrigin>),
    Sip(SipAudienceOrigin),
}
impl ConsultationMediaOrigin {
    pub fn connection_id(&self) -> &str {
        match self {
            Self::Native(o) => o.connection_id(),
            Self::Sip(o) => o.connection_id(),
        }
    }
    pub fn runtime_connection_id(&self) -> &str {
        match self {
            Self::Native(o) => o.runtime_connection_id(),
            Self::Sip(o) => o.runtime_connection_id(),
        }
    }
    pub fn participant_id(&self) -> &str {
        match self {
            Self::Native(o) => o.participant_id(),
            Self::Sip(o) => o.participant_id(),
        }
    }
    pub fn membership_epoch(&self) -> u64 {
        match self {
            Self::Native(o) => o.membership_epoch(),
            Self::Sip(o) => o.membership_epoch(),
        }
    }
    pub(super) fn native(&self) -> Option<&NativeMediaOrigin> {
        match self {
            Self::Native(o) => Some(o),
            Self::Sip(_) => None,
        }
    }
    pub(super) fn sip(&self) -> Option<&SipAudienceOrigin> {
        match self {
            Self::Sip(o) => Some(o),
            Self::Native(_) => None,
        }
    }
    pub(super) fn snapshot(&self, store: &Store, c: &Connection) -> Result<serde_json::Value> {
        match self {
            Self::Native(o) => Ok(super::returns::describe(o)),
            Self::Sip(o) => o.facts(store, c).map(|(_, value, _)| value),
        }
    }
}
impl std::fmt::Debug for ConsultationMediaOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConsultationMediaOrigin(redacted)")
    }
}
