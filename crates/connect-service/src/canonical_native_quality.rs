//! Current named quality control uses the original command's held source fence.
//! Its engine callback is pure: it neither reacquires policy nor enters Core.
use crate::{
    canonical_media::Connection,
    canonical_runtime::{Physical, Runtime},
    StoreInvocationAuthority,
};
use connect_protocol::{Error, Result};
use rvoip_core::{ids::ConnectionId, staged_streams::MediaOwner};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, AuthorizedEnvelope, DispatchError},
    native_lifecycle::NativeLifecycleScope,
    native_quality::NativeQualityConfigPolicy,
};
use std::sync::Arc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub(crate) struct Invocation {
    entry: Arc<Connection>,
    authorized: Arc<AuthorizedEnvelope>,
    read: Arc<connect_store::NativeMediaRead>,
    _owner: MediaOwner,
    revision: u64,
    interval_ms: u64,
    deadline: Instant,
    physical: CancellationToken,
    stop: CancellationToken,
}
impl Invocation {
    pub(crate) fn prepare(
        runtime: &Arc<Runtime>,
        a: Arc<AuthorizedEnvelope>,
        lease: &connect_store::NativeStreamLease,
        physical: &Physical,
        deadline: Instant,
    ) -> Result<Self> {
        let wire = crate::canonical_native_streams::intent(a.envelope())?;
        let interval_ms = wire.quality_interval_ms().ok_or_else(Error::forbidden)?;
        let proof = a
            .authority::<StoreInvocationAuthority>()
            .and_then(|h| h.peer_guard())
            .ok_or_else(Error::forbidden)?;
        let entry = runtime
            .media
            .entry(&ConnectionId::from_string(
                lease.runtime_connection_id().to_owned(),
            ))
            .map_err(|_| Error::forbidden())?;
        let read = Arc::new(runtime.media.store.native_stream_media_read_with_peer(
            &entry.origin,
            None,
            proof,
            lease,
        )?);
        if wire.connection_id != lease.connection_id()
            || wire.expected_revision != Some(lease.expected_revision())
        {
            return Err(Error::forbidden());
        }
        // Resolve engine ownership before configure_native_quality takes its
        // internal locks. The synchronous callback below only inspects held facts.
        let owner = entry
            .owner(read.target(), a.principal(), false)
            .map_err(|_| Error::forbidden())?;
        let owner = entry
            .activation()
            .map_err(|_| Error::forbidden())?
            .ok_or_else(Error::forbidden)?
            .current(owner)
            .map_err(|_| Error::forbidden())?;
        Ok(Self {
            entry,
            authorized: a,
            read,
            _owner: owner,
            revision: lease.expected_revision(),
            interval_ms,
            deadline,
            physical: physical.lifetime.clone(),
            stop: runtime.stop.clone(),
        })
    }
    fn current(&self) -> bool {
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
impl NativeQualityConfigPolicy for Invocation {
    fn authorize(
        &self,
        scope: &NativeLifecycleScope,
        a: &AuthorizedEnvelope,
        interval_ms: u64,
    ) -> std::result::Result<AuthorityLease, DispatchError> {
        let origin = &self.entry.origin;
        let view = self.read.target();
        if !self.current()
            || !std::ptr::eq(a, self.authorized.as_ref())
            || interval_ms != self.interval_ms
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
            move || current.current(),
        ))
    }
}
