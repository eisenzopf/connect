//! Actual installed SIP legs shared by native and received signal routing.
use crate::initial_telephone::policy::Leg as InitialLeg;
use connect_store::SipSignalOrigin;
use rvoip_core::{dtmf::DtmfWriteError, ids::ConnectionId};
use std::sync::{Arc, Weak};
type WriteResult<T> = std::result::Result<T, DtmfWriteError>;
#[derive(Clone)]
pub(crate) enum SipLeg {
    Initial(Arc<InitialLeg>),
    Established(Arc<crate::telephone_replacement::media::Policy>),
}
impl SipLeg {
    pub(crate) fn registration(&self) -> Option<&str> {
        match self {
            Self::Initial(leg) => leg.dtmf_registration.get().map(String::as_str),
            Self::Established(policy) => Some(&policy.dtmf_registration),
        }
    }
    pub(crate) fn origin(&self) -> WriteResult<SipSignalOrigin> {
        match self {
            Self::Initial(leg) => leg
                .media
                .get()
                .cloned()
                .map(SipSignalOrigin::Initial)
                .ok_or(DtmfWriteError::Refused),
            Self::Established(policy) => policy
                .established_origin()
                .map(SipSignalOrigin::Established)
                .ok_or(DtmfWriteError::Refused),
        }
    }
    pub(crate) fn matches(&self, origin: &SipSignalOrigin) -> bool {
        match (self, origin) {
            (Self::Initial(leg), SipSignalOrigin::Initial(origin)) => leg
                .media
                .get()
                .is_some_and(|actual| Arc::ptr_eq(actual, origin)),
            (Self::Established(policy), SipSignalOrigin::Established(origin)) => policy
                .established_origin()
                .is_some_and(|actual| Arc::ptr_eq(&actual, origin)),
            _ => false,
        }
    }
    pub(crate) fn cancelled(&self) -> bool {
        match self {
            Self::Initial(leg) => leg.cancel.is_cancelled(),
            Self::Established(policy) => !policy.established_signal_current(),
        }
    }
    pub(crate) fn connection(&self) -> WriteResult<ConnectionId> {
        self.origin()
            .map(|origin| ConnectionId::from_string(origin.runtime_connection_id()))
    }
    pub(crate) fn principal(&self) -> WriteResult<rvoip_core::AuthenticatedPrincipal> {
        match self {
            Self::Initial(leg) => Ok(leg.target.principal.clone()),
            Self::Established(policy) => policy.principal().map_err(|_| DtmfWriteError::Refused),
        }
    }
    pub(crate) fn controller(
        &self,
    ) -> WriteResult<Arc<rvoip_core::staged_streams::StreamController>> {
        if self.cancelled() {
            return Err(DtmfWriteError::Refused);
        }
        match self {
            Self::Initial(leg) => {
                let controller = leg
                    .controller
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or(DtmfWriteError::Refused)?;
                let prepared = leg.prepared.get().ok_or(DtmfWriteError::Refused)?;
                if !prepared.is_current() || !controller.is_bound_to_outbound(prepared) {
                    return Err(DtmfWriteError::Refused);
                }
                Ok(controller)
            }
            Self::Established(policy) => {
                let controller = policy.controller().map_err(|_| DtmfWriteError::Refused)?;
                if !controller.is_bound_to_replacement(&policy.pending) {
                    return Err(DtmfWriteError::Refused);
                }
                Ok(controller)
            }
        }
    }
    pub(crate) fn matches_member(&self, member: &connect_protocol::membership::MemberView) -> bool {
        match self {
            Self::Initial(leg) => {
                member.participant_id == leg.target.member.participant().as_str()
                    && member.membership_epoch == leg.target.member.epoch()
                    && member.identity_id == leg.target.principal.subject
            }
            Self::Established(policy) => policy.established_origin().is_some_and(|origin| {
                let s = origin.snapshot();
                member.participant_id == s.participant_id
                    && member.membership_epoch == s.membership_epoch
                    && member.identity_id == s.identity_id
            }),
        }
    }
}
