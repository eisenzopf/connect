//! Native source custody with actual initial or established SIP incarnations.
use super::*;
use crate::{
    InitialTelephoneMediaOrigin, InitialTelephoneNativeSignalRead,
    InitialTelephoneSignalDirection::ToTelephone, TelephoneMediaOrigin, TelephoneNativeSignalRead,
    TelephoneSignalDirection,
};

/// An actual SIP origin; neither variant can substitute a native lifetime.
#[derive(Clone)]
pub enum NativeDtmfGatewayOrigin {
    Initial(Arc<InitialTelephoneMediaOrigin>),
    Established(Arc<TelephoneMediaOrigin>),
}
impl NativeDtmfGatewayOrigin {
    pub fn initial(&self) -> Option<&Arc<InitialTelephoneMediaOrigin>> {
        match self {
            Self::Initial(origin) => Some(origin),
            _ => None,
        }
    }
    pub fn established(&self) -> Option<&Arc<TelephoneMediaOrigin>> {
        match self {
            Self::Established(origin) => Some(origin),
            _ => None,
        }
    }
    pub fn record_id(&self) -> &str {
        match self {
            Self::Initial(origin) => origin.record_id(),
            Self::Established(origin) => origin.record_id(),
        }
    }
    pub fn connection_id(&self) -> &str {
        match self {
            Self::Initial(origin) => origin.connection_id(),
            Self::Established(origin) => &origin.snapshot().connection_id,
        }
    }
    pub fn runtime_connection_id(&self) -> &str {
        match self {
            Self::Initial(origin) => origin.runtime_connection_id(),
            Self::Established(origin) => &origin.snapshot().runtime_connection_id,
        }
    }
    pub fn sip_call_id(&self) -> &str {
        match self {
            Self::Initial(origin) => origin.sip_call_id(),
            Self::Established(origin) => &origin.snapshot().media.call_id,
        }
    }
    pub(super) fn table(&self) -> &'static str {
        match self {
            Self::Initial(_) => "native_dtmf_gateways",
            Self::Established(_) => "native_dtmf_established_gateways",
        }
    }
    fn scopes(
        &self,
        store: &Store,
        c: &Connection,
        native: &Arc<NativeMediaOrigin>,
    ) -> Result<Vec<AuthorityScope>> {
        match self {
            Self::Initial(origin) => {
                store.initial_telephone_native_signal_scopes(c, origin, native)
            }
            Self::Established(origin) => store.telephone_native_signal_scopes(c, origin, native),
        }
    }
    fn read_in(
        &self,
        store: &Store,
        c: &Connection,
        native: Arc<NativeMediaOrigin>,
        interval: Arc<crate::AuthorityGuardSet>,
        until: i64,
    ) -> Result<GatewayPair> {
        match self {
            Self::Initial(origin) => store
                .initial_telephone_native_signal_in(
                    c,
                    origin.clone(),
                    native,
                    ToTelephone,
                    interval,
                    until,
                )
                .map(GatewayPair::Initial),
            Self::Established(origin) => store
                .telephone_native_signal_in(
                    c,
                    origin.clone(),
                    native,
                    TelephoneSignalDirection::ToTelephone,
                    interval,
                    until,
                )
                .map(GatewayPair::Established),
        }
    }
}
enum GatewayPair {
    Initial(InitialTelephoneNativeSignalRead),
    Established(TelephoneNativeSignalRead),
}
impl GatewayPair {
    fn current(&self) -> bool {
        match self {
            Self::Initial(read) => read.current(),
            Self::Established(read) => read.current(),
        }
    }
    fn expires_at(&self) -> i64 {
        match self {
            Self::Initial(read) => read.expires_at(),
            Self::Established(read) => read.expires_at(),
        }
    }
    fn member(&self) -> &connect_protocol::membership::MemberView {
        match self {
            Self::Initial(read) => read.phone().member(),
            Self::Established(read) => read.phone().member(),
        }
    }
    fn interval(&self) -> &Arc<crate::AuthorityGuardSet> {
        match self {
            Self::Initial(read) => read.interval(),
            Self::Established(read) => read.interval(),
        }
    }
    fn matches(&self, origin: &NativeDtmfGatewayOrigin) -> bool {
        match (self, origin) {
            (Self::Initial(read), NativeDtmfGatewayOrigin::Initial(origin)) => {
                read.phone().direction() == ToTelephone
                    && read.phone().origin().record_id() == origin.record_id()
            }
            (Self::Established(read), NativeDtmfGatewayOrigin::Established(origin)) => {
                read.phone().direction() == TelephoneSignalDirection::ToTelephone
                    && read.phone().origin().record_id() == origin.record_id()
            }
            _ => false,
        }
    }
}

pub struct NativeDtmfGatewayRecipient {
    pub origin: NativeDtmfGatewayOrigin,
    pub registration_id: String,
}
#[derive(Clone)]
pub struct NativeDtmfGatewayTarget {
    pub(super) source: Arc<Source>,
    pub(super) origin: NativeDtmfGatewayOrigin,
    pub(super) registration: String,
    pub(super) snapshot: serde_json::Value,
}
impl NativeDtmfGatewayTarget {
    pub fn origin(&self) -> &NativeDtmfGatewayOrigin {
        &self.origin
    }
    pub fn registration_id(&self) -> &str {
        &self.registration
    }
    pub fn intent(&self) -> &Intent {
        &self.source.wire
    }
    pub fn observation(&self) -> &SourceObservation {
        &self.source.observation
    }
}
pub struct NativeDtmfGatewayRead {
    pair: GatewayPair,
    target: NativeDtmfGatewayTarget,
    expires: i64,
}
impl NativeDtmfGatewayRead {
    pub fn phone_member(&self) -> &connect_protocol::membership::MemberView {
        self.pair.member()
    }
    pub fn current(&self) -> bool {
        self.pair.current() && crate::now() < self.expires
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
}
macro_rules! gateway_redacted { ($($t:ty),*) => { $(impl std::fmt::Debug for $t {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(concat!(stringify!($t), "(redacted)")) }
})* }; }
gateway_redacted!(
    NativeDtmfGatewayOrigin,
    NativeDtmfGatewayRecipient,
    NativeDtmfGatewayTarget,
    NativeDtmfGatewayRead
);

fn snapshot(read: &GatewayPair) -> serde_json::Value {
    let phone = match read {
        GatewayPair::Initial(read) => read.phone(),
        GatewayPair::Established(read) => {
            let phone = read.phone();
            let s = phone.snapshot();
            return serde_json::json!({
                "connection_id":s.connection_id, "runtime_connection_id":s.runtime_connection_id,
                "sip_call_id":s.media.call_id, "participant_id":phone.member().participant_id,
                "membership_epoch":phone.member().membership_epoch, "generation":s.generation,
                "identity_id":phone.member().identity_id, "endpoint_id":s.endpoint_id,
                "device_id":s.device_id, "route_credential_id":phone.route_actor().credential_id,
                "route_owner_id":s.route_owner_id,
            });
        }
    };
    let origin = phone.origin();
    let source = origin.source();
    serde_json::json!({
        "connection_id":origin.connection_id(), "runtime_connection_id":origin.runtime_connection_id(),
        "sip_call_id":origin.sip_call_id(), "participant_id":phone.member().participant_id,
        "membership_epoch":phone.member().membership_epoch, "generation":1,
        "identity_id":phone.member().identity_id, "endpoint_id":source.endpoint_id,
        "endpoint_revision":source.endpoint_revision, "device_id":source.device_id,
        "endpoint_credential_id":phone.endpoint_actor().credential_id,
        "route_credential_id":phone.route_actor().credential_id, "route_owner_id":source.route_owner_id,
    })
}
fn current(
    c: &Connection,
    store: &Store,
    target: &NativeDtmfGatewayTarget,
    read: &GatewayPair,
) -> Result<()> {
    let row = historical(c, store, &target.source)?;
    let scopes = target.origin.scopes(store, c, &target.source.origin)?;
    if row.record.state != OperationState::Preparing
        || row.record.cancel_requested
        || crate::now() >= target.source.expires
        || !read.current()
        || !read.matches(&target.origin)
        || snapshot(read) != target.snapshot
        || !store.authority_covers(read.interval(), &scopes)
    {
        return Err(Error::forbidden());
    }
    let exact: bool = c.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {} r JOIN native_stream_operations n ON n.operation_record_id=r.operation_record_id JOIN operation_effects e ON e.id=n.effect_id WHERE r.operation_record_id=?1 AND r.target_record_id=?2 AND r.registration_id=?3 AND r.target_json=?4 AND r.disposition IS NULL AND n.closed=0 AND e.state='submitting')", target.origin.table()),
        params![target.source.record,target.origin.record_id(),target.registration,target.snapshot.to_string()], |r|r.get(0),
    ).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    Ok(())
}
impl Store {
    pub fn select_native_dtmf_gateways_with_peer(
        &self,
        lease: &NativeStreamLease,
        source: Arc<NativeMediaOrigin>,
        candidates: Vec<NativeDtmfGatewayRecipient>,
        proof: &PeerAuthorityGuard,
    ) -> Result<Vec<NativeDtmfGatewayRecipient>> {
        self.validate_native_dtmf_source_with_peer(&source, proof, lease)?;
        if candidates.len() > self.limits().connections.min(512) {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        let mut selected = Vec::new();
        for target in candidates {
            let scopes = target.origin.scopes(self, &c, &source)?;
            if !self.authority_covers(Store::peer_scopes(proof), &scopes) {
                return Err(Error::forbidden());
            }
            match target.origin.read_in(
                self,
                &c,
                source.clone(),
                Store::peer_scopes(proof).clone(),
                lease.expires_at(),
            ) {
                Ok(read) if read.current() => selected.push(target),
                Ok(_) => return Err(Error::forbidden()),
                Err(e)
                    if matches!(
                        e.code.as_str(),
                        "forbidden" | "not-recipient" | "unauthorized"
                    ) =>
                {
                    ()
                }
                Err(e) => return Err(e),
            }
        }
        Ok(selected)
    }
    pub(super) fn freeze_native_dtmf_gateways(
        &self,
        c: &Connection,
        source: &Arc<NativeMediaOrigin>,
        targets: Vec<NativeDtmfGatewayRecipient>,
        proof: &PeerAuthorityGuard,
        seen: &mut BTreeSet<String>,
        registrations: &mut BTreeSet<String>,
    ) -> Result<Vec<(NativeDtmfGatewayRecipient, serde_json::Value)>> {
        let mut frozen = Vec::with_capacity(targets.len());
        for target in targets {
            id(&target.registration_id)?;
            if !seen.insert(target.origin.record_id().into())
                || !registrations.insert(target.registration_id.clone())
            {
                return Err(conflict());
            }
            let read = target.origin.read_in(
                self,
                c,
                source.clone(),
                Store::peer_scopes(proof).clone(),
                crate::now() + 30,
            )?;
            if !read.current() {
                return Err(Error::forbidden());
            }
            frozen.push((target, snapshot(&read)));
        }
        Ok(frozen)
    }
    pub fn try_native_dtmf_gateway_read(
        &self,
        target: &NativeDtmfGatewayTarget,
    ) -> Result<NativeDtmfGatewayRead> {
        let c = self.try_connection()?;
        let scopes = target.origin.scopes(self, &c, &target.source.origin)?;
        let interval = self.try_acquire_authority(&scopes)?;
        let pair = target.origin.read_in(
            self,
            &c,
            target.source.origin.clone(),
            interval,
            crate::now() + 30,
        )?;
        current(&c, self, target, &pair)?;
        let expires = pair.expires_at().min(target.source.expires);
        Ok(NativeDtmfGatewayRead {
            pair,
            target: target.clone(),
            expires,
        })
    }
    /// Reserve the original local schedule ID before its first possible write.
    /// Per-packet/INFO retries may reacquire only this same attempt, never a new
    /// sequence. Current complete signal readers remain held through each IO.
    pub fn try_start_native_dtmf_gateway_write(
        &self,
        read: &NativeDtmfGatewayRead,
        attempt: &str,
    ) -> Result<()> {
        id(attempt)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        current(&tx, self, &read.target, &read.pair)?;
        let target = &read.target;
        let old: Option<String> = tx
            .query_row(
                &format!(
                    "SELECT frame_id FROM {} WHERE operation_record_id=?1 AND target_record_id=?2",
                    target.origin.table()
                ),
                params![target.source.record, target.origin.record_id()],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if let Some(old) = old {
            if old != attempt {
                return Err(conflict());
            }
        } else {
            tx.execute(&format!("UPDATE {} SET attempted=1,frame_id=?3 WHERE operation_record_id=?1 AND target_record_id=?2 AND attempted=0 AND disposition IS NULL",target.origin.table()),params![target.source.record,target.origin.record_id(),attempt]).map_err(|_|Error::storage())?;
        }
        if !read.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn try_record_native_dtmf_gateway_outcome(
        &self,
        target: &NativeDtmfGatewayTarget,
        disposition: Disposition,
    ) -> Result<()> {
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        historical(&tx, self, &target.source)?;
        outcome(
            &tx,
            self,
            &target.source.record,
            target.origin.record_id(),
            disposition,
            false,
        )?;
        tx.commit().map_err(|_| Error::storage())
    }
}
