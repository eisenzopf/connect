//! Actual SIP incarnations and independent signal readers. No RTP grant.
use super::*;
use crate::{AuthorityGuardSet, AuthorityScope, NativeMediaOrigin, NativeSignalRead};
use connect_protocol::membership::MemberView;

#[derive(Clone)]
pub enum SipSignalOrigin {
    Initial(Arc<InitialTelephoneMediaOrigin>),
    Established(Arc<TelephoneMediaOrigin>),
}
pub enum SipSignalRead {
    Initial(InitialTelephoneSignalRead),
    Established(TelephoneSignalRead),
}
pub enum SipNativeSignalRead {
    Initial(InitialTelephoneNativeSignalRead),
    Established(TelephoneNativeSignalRead),
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {f.write_str(concat!(stringify!($t),"(redacted)"))}})*};}
redacted!(SipSignalOrigin, SipSignalRead, SipNativeSignalRead);
fn initial_direction(direction: TelephoneSignalDirection) -> InitialTelephoneSignalDirection {
    match direction {
        TelephoneSignalDirection::FromTelephone => InitialTelephoneSignalDirection::FromTelephone,
        TelephoneSignalDirection::ToTelephone => InitialTelephoneSignalDirection::ToTelephone,
    }
}
fn direction(direction: InitialTelephoneSignalDirection) -> TelephoneSignalDirection {
    match direction {
        InitialTelephoneSignalDirection::FromTelephone => TelephoneSignalDirection::FromTelephone,
        InitialTelephoneSignalDirection::ToTelephone => TelephoneSignalDirection::ToTelephone,
    }
}
impl SipSignalOrigin {
    pub fn initial(&self) -> Option<&Arc<InitialTelephoneMediaOrigin>> {
        match self {
            Self::Initial(o) => Some(o),
            _ => None,
        }
    }
    pub fn established(&self) -> Option<&Arc<TelephoneMediaOrigin>> {
        match self {
            Self::Established(o) => Some(o),
            _ => None,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Initial(_) => "sip-initial",
            Self::Established(_) => "sip-established",
        }
    }
    pub fn record_id(&self) -> &str {
        match self {
            Self::Initial(o) => o.record_id(),
            Self::Established(o) => o.record_id(),
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
    pub fn sip_call_id(&self) -> &str {
        match self {
            Self::Initial(o) => o.sip_call_id(),
            Self::Established(o) => &o.snapshot().media.call_id,
        }
    }
    pub fn conversation_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().request.conversation_id,
            Self::Established(o) => &o.snapshot().conversation_id,
        }
    }
    pub fn session_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().request.session_id,
            Self::Established(o) => &o.snapshot().session_id,
        }
    }
    pub fn participant_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().target.participant_id,
            Self::Established(o) => &o.snapshot().participant_id,
        }
    }
    pub fn identity_id(&self) -> &str {
        match self {
            Self::Initial(o) => &o.source().target.identity_id,
            Self::Established(o) => &o.snapshot().identity_id,
        }
    }
    pub(crate) fn gateway_table(&self) -> &'static str {
        match self {
            Self::Initial(_) => "native_dtmf_gateways",
            Self::Established(_) => "native_dtmf_established_gateways",
        }
    }
    pub(crate) fn scopes(&self, store: &Store, c: &Connection) -> Result<Vec<AuthorityScope>> {
        match self {
            Self::Initial(o) => store.initial_telephone_signal_scopes(c, o),
            Self::Established(o) => store.telephone_established_scopes(c, o),
        }
    }
    pub(crate) fn pair_scopes(
        &self,
        store: &Store,
        c: &Connection,
        native: &Arc<NativeMediaOrigin>,
    ) -> Result<Vec<AuthorityScope>> {
        match self {
            Self::Initial(o) => store.initial_telephone_native_signal_scopes(c, o, native),
            Self::Established(o) => store.telephone_native_signal_scopes(c, o, native),
        }
    }
    pub(crate) fn read_in(
        &self,
        store: &Store,
        c: &Connection,
        direction: TelephoneSignalDirection,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<SipSignalRead> {
        match self {
            Self::Initial(o) => store
                .initial_telephone_signal_in(
                    c,
                    o.clone(),
                    initial_direction(direction),
                    interval,
                    until,
                )
                .map(SipSignalRead::Initial),
            Self::Established(o) => store
                .telephone_signal_in(c, o.clone(), direction, interval, until)
                .map(SipSignalRead::Established),
        }
    }
    pub(crate) fn pair_in(
        &self,
        store: &Store,
        c: &Connection,
        native: Arc<NativeMediaOrigin>,
        direction: TelephoneSignalDirection,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<SipNativeSignalRead> {
        match self {
            Self::Initial(o) => store
                .initial_telephone_native_signal_in(
                    c,
                    o.clone(),
                    native,
                    initial_direction(direction),
                    interval,
                    until,
                )
                .map(SipNativeSignalRead::Initial),
            Self::Established(o) => store
                .telephone_native_signal_in(c, o.clone(), native, direction, interval, until)
                .map(SipNativeSignalRead::Established),
        }
    }
}
fn initial_snapshot(read: &InitialTelephoneSignalRead) -> serde_json::Value {
    let o = read.origin();
    let s = o.source();
    serde_json::json!({"connection_id":o.connection_id(),"runtime_connection_id":o.runtime_connection_id(),"sip_call_id":o.sip_call_id(),"participant_id":read.member().participant_id,"membership_epoch":read.member().membership_epoch,"generation":1,"identity_id":read.member().identity_id,"endpoint_id":s.endpoint_id,"endpoint_revision":s.endpoint_revision,"device_id":s.device_id,"endpoint_credential_id":read.endpoint_actor().credential_id,"route_credential_id":read.route_actor().credential_id,"route_owner_id":s.route_owner_id})
}
fn established_snapshot(read: &TelephoneSignalRead) -> serde_json::Value {
    let s = read.snapshot();
    serde_json::json!({"connection_id":s.connection_id,"runtime_connection_id":s.runtime_connection_id,"sip_call_id":s.media.call_id,"participant_id":read.member().participant_id,"membership_epoch":read.member().membership_epoch,"generation":s.generation,"identity_id":read.member().identity_id,"endpoint_id":s.endpoint_id,"device_id":s.device_id,"route_credential_id":read.route_actor().credential_id,"route_owner_id":s.route_owner_id})
}
impl SipSignalRead {
    pub fn current(&self) -> bool {
        match self {
            Self::Initial(r) => r.current(),
            Self::Established(r) => r.current(),
        }
    }
    pub fn expires_at(&self) -> i64 {
        match self {
            Self::Initial(r) => r.expires_at(),
            Self::Established(r) => r.expires_at(),
        }
    }
    pub fn member(&self) -> &MemberView {
        match self {
            Self::Initial(r) => r.member(),
            Self::Established(r) => r.member(),
        }
    }
    pub(crate) fn permission_actor(&self) -> &Actor {
        match self {
            Self::Initial(r) => r.endpoint_actor(),
            Self::Established(r) => r.route_actor(),
        }
    }
    pub(crate) fn route_actor(&self) -> &Actor {
        match self {
            Self::Initial(r) => r.route_actor(),
            Self::Established(r) => r.route_actor(),
        }
    }
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        match self {
            Self::Initial(r) => initial_snapshot(r),
            Self::Established(r) => established_snapshot(r),
        }
    }
}
impl SipNativeSignalRead {
    pub fn current(&self) -> bool {
        match self {
            Self::Initial(r) => r.current(),
            Self::Established(r) => r.current(),
        }
    }
    pub fn expires_at(&self) -> i64 {
        match self {
            Self::Initial(r) => r.expires_at(),
            Self::Established(r) => r.expires_at(),
        }
    }
    pub fn member(&self) -> &MemberView {
        match self {
            Self::Initial(r) => r.phone().member(),
            Self::Established(r) => r.phone().member(),
        }
    }
    pub fn native(&self) -> &NativeSignalRead {
        match self {
            Self::Initial(r) => r.native(),
            Self::Established(r) => r.native(),
        }
    }
    pub(crate) fn interval(&self) -> &Arc<AuthorityGuardSet> {
        match self {
            Self::Initial(r) => r.interval(),
            Self::Established(r) => r.interval(),
        }
    }
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        match self {
            Self::Initial(r) => initial_snapshot(r.phone()),
            Self::Established(r) => established_snapshot(r.phone()),
        }
    }
    pub(crate) fn matches(
        &self,
        origin: &SipSignalOrigin,
        wanted: TelephoneSignalDirection,
    ) -> bool {
        match (self, origin) {
            (Self::Initial(r), SipSignalOrigin::Initial(o)) => {
                direction(r.phone().direction()) == wanted
                    && r.phone().origin().record_id() == o.record_id()
            }
            (Self::Established(r), SipSignalOrigin::Established(o)) => {
                r.phone().direction() == wanted && r.phone().origin().record_id() == o.record_id()
            }
            _ => false,
        }
    }
}
impl Store {
    /// The signal pair must keep both physical private floors. A Participant
    /// match never lends the registered consultant's authority to a sibling.
    pub(crate) fn require_sip_native_signal_context(
        &self,
        c: &Connection,
        phone: &str,
        native: &NativeMediaOrigin,
        private_record: Option<&str>,
    ) -> Result<()> {
        use crate::connection_control::private_consultation::private_partner;
        let phone_partner = private_partner(c, phone)?;
        let native_partner = private_partner(c, native.connection_id())?;
        match private_record {
            None if phone_partner.is_none() && native_partner.is_none() => Ok(()),
            Some(record)
                if phone_partner.as_deref() == Some(native.connection_id())
                    && native_partner.as_deref() == Some(phone) =>
            {
                let exact: bool = c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM native_consultation_legs l JOIN native_private_content_floors f ON f.operation_record_id=l.operation_record_id AND f.connection_id=l.primary_connection_id AND f.is_candidate=0 WHERE l.operation_record_id=?1 AND l.connection_id=?2 AND l.primary_connection_id=?3 AND l.runtime_epoch=?4)",
                    params![record, native.connection_id(), phone, self.runtime_epoch()],
                    |r| r.get(0),
                ).map_err(|_| Error::storage())?;
                if exact {
                    Ok(())
                } else {
                    Err(Error::forbidden())
                }
            }
            _ => Err(Error::forbidden()),
        }
    }
    pub fn try_sip_signal_read(
        &self,
        origin: &SipSignalOrigin,
        direction: TelephoneSignalDirection,
    ) -> Result<SipSignalRead> {
        let c = self.try_connection()?;
        let scopes = origin.scopes(self, &c)?;
        let interval = self.try_acquire_authority(&scopes)?;
        origin.read_in(self, &c, direction, interval, crate::now() + 30)
    }
    pub fn try_sip_native_signal_read(
        &self,
        origin: &SipSignalOrigin,
        native: Arc<NativeMediaOrigin>,
        direction: TelephoneSignalDirection,
    ) -> Result<SipNativeSignalRead> {
        let c = self.try_connection()?;
        let scopes = origin.pair_scopes(self, &c, &native)?;
        let interval = self.try_acquire_authority(&scopes)?;
        origin.pair_in(self, &c, native, direction, interval, crate::now() + 30)
    }
}
