//! Receive an endpoint declaration under its actual original native authority.
//! No codec, route, assurance or permission is changed by this metadata receipt.
use crate::{
    canonical_media::Connection,
    canonical_runtime::{Physical, Runtime},
    StoreInvocationAuthority,
};
use connect_protocol::{Error, Result};
use rvoip_core::{
    ids::ConnectionId,
    staged_streams::{MediaOwner, StreamController},
};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, AuthorizedEnvelope, DispatchError},
    native_capabilities::NativeCapabilityDeclarationPolicy,
    native_lifecycle::NativeLifecycleScope,
};
use std::sync::Arc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct Received {
    entry: Arc<Connection>,
    descriptor: serde_json::Value,
    read: Arc<connect_store::NativeMediaRead>,
    authorized: Arc<AuthorizedEnvelope>,
    _owner: MediaOwner,
    controller: Arc<StreamController>,
    revision: u64,
    deadline: Instant,
    physical: CancellationToken,
    stop: CancellationToken,
}
impl Received {
    pub(crate) fn prepare(
        runtime: &Arc<Runtime>,
        a: Arc<AuthorizedEnvelope>,
        lease: &connect_store::NativeStreamLease,
        physical: &Physical,
        deadline: Instant,
    ) -> Result<Self> {
        let wire = crate::canonical_native_streams::intent(a.envelope())?;
        if wire.command != connect_protocol::native_streams::Command::Advertise {
            return Err(Error::forbidden());
        }
        // A self-declared assurance label cannot change or overstate the real
        // verified credential. Missing labels carry no assurance assertion.
        if wire.payload["capabilities"]
            .get("identity_assurance_offered")
            .is_some_and(|v| v.as_str() != Some(a.principal().assurance.kind()))
        {
            return Err(Error::new(
                "forbidden-for-assurance-level",
                "Advertised assurance differs from the authenticated peer",
            ));
        }
        let proof = a
            .authority::<StoreInvocationAuthority>()
            .and_then(|h| h.peer_guard())
            .ok_or_else(Error::forbidden)?;
        let id = ConnectionId::from_string(lease.runtime_connection_id());
        let bound = runtime
            .native
            .physical_source(&id)
            .map_err(|_| Error::forbidden())?;
        if !Arc::ptr_eq(&bound, &physical.binding) {
            return Err(Error::forbidden());
        }
        let entry = runtime.media.entry(&id).map_err(|_| Error::forbidden())?;
        let read = Arc::new(runtime.media.store.native_stream_media_read_with_peer(
            &entry.origin,
            None,
            proof,
            lease,
        )?);
        let owner = entry
            .owner(read.target(), a.principal(), false)
            .map_err(|_| Error::forbidden())?;
        let owner = entry
            .activation()
            .map_err(|_| Error::forbidden())?
            .ok_or_else(Error::forbidden)?
            .current(owner)
            .map_err(|_| Error::forbidden())?;
        let controller = runtime
            .native
            .media_controller(&id)
            .map_err(|_| Error::forbidden())?;
        if wire.connection_id != lease.connection_id()
            || wire.expected_revision != Some(lease.expected_revision())
            || !controller.matches_data_credential(a.principal())
        {
            return Err(Error::forbidden());
        }
        let received = Self {
            entry,
            descriptor: wire.payload["capabilities"].clone(),
            read,
            authorized: a,
            _owner: owner,
            controller,
            revision: lease.expected_revision(),
            deadline,
            physical: physical.lifetime.clone(),
            stop: runtime.stop.clone(),
        };
        if !received.current() {
            return Err(Error::forbidden());
        }
        Ok(received)
    }
    pub(crate) fn entry(&self) -> Arc<Connection> {
        self.entry.clone()
    }
    pub(crate) fn current(&self) -> bool {
        self.domain_current() && self.controller.revision() == self.revision
    }
    // Pure domain facts only. This callback runs before the native adapter
    // enters controller state and must never reenter Core or acquire a reader.
    fn domain_current(&self) -> bool {
        self.read.current()
            && Instant::now() < self.deadline
            && !self.physical.is_cancelled()
            && !self.stop.is_cancelled()
            && !self.authorized.invocation_cancellation().is_cancelled()
            && self
                .authorized
                .authority::<StoreInvocationAuthority>()
                .and_then(|h| h.peer_guard())
                .is_some_and(|p| p.current())
    }
}
impl NativeCapabilityDeclarationPolicy for Received {
    fn authorize(
        &self,
        scope: &NativeLifecycleScope,
        a: &AuthorizedEnvelope,
        descriptor: &serde_json::Value,
    ) -> std::result::Result<AuthorityLease, DispatchError> {
        let origin = &self.entry.origin;
        let view = self.read.target();
        if !self.domain_current()
            || !std::ptr::eq(a, self.authorized.as_ref())
            || descriptor != &self.descriptor
            || a.envelope().expected_revision() != Some(self.revision)
            || scope.media.connection.as_str() != origin.runtime_connection_id()
            || scope.media.session.as_str() != origin.session_id()
            || scope.media.wire_conversation.as_deref() != Some(origin.conversation_id())
            || scope.media.wire_session != origin.session_id()
            || scope.media.wire_connection != origin.connection_id()
            || scope.media.owner != self.entry.principal.ownership_key()
            || scope.participant.as_str() != origin.participant_id()
            || scope.membership_epoch != origin.membership_epoch()
            || scope.device.as_str() != view.device_id
            || scope.endpoint.as_str() != view.connection.endpoint_id
            || scope.identity.as_str() != view.session.actor.identity_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &scope.principal,
                &self.entry.principal,
            )
        {
            return Err(DispatchError::Authority);
        }
        let current = self.clone();
        Ok(AuthorityLease::checking(
            self.clone(),
            self.deadline,
            move || current.domain_current(),
        ))
    }
}
