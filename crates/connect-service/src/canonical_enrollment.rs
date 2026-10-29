//! Immutable enrollment constraints compose with the original owned peer proof.
use crate::{canonical_runtime::Physical, StoreInvocationAuthority};
use connect_protocol::{enrollment::EnrollmentTransport, Error, Result};
use connect_store::Store;
use rvoip_core::Transport;
use std::sync::Arc;

pub(crate) async fn admit(
    store: Arc<Store>,
    physical: Option<&Arc<Physical>>,
    authority: &StoreInvocationAuthority,
) -> Result<()> {
    let transport = physical.map(|p| p.binding.transport());
    let origin = physical.and_then(|p| p.browser_origin.clone());
    let owned = authority.retained();
    tokio::task::spawn_blocking(move || {
        let proof = owned
            .peer_guard()
            .filter(|p| p.current())
            .ok_or_else(Error::forbidden)?;
        if proof.metadata.actor.credential_id != owned.binding().credential_id() {
            return Err(Error::forbidden());
        }
        if let Some(binding) =
            store.enrollment_credential_binding(owned.binding().credential_id())?
        {
            let expected = match binding.transport {
                EnrollmentTransport::Quic => Transport::Quic,
                EnrollmentTransport::Webtransport => Transport::WebTransport,
                EnrollmentTransport::WssWebrtc => Transport::WebSocket,
            };
            if transport != Some(expected)
                || origin != binding.browser_origin
                || proof.key.identity_id != binding.identity_id
                || proof.key.device_id != binding.device_id
                || proof.key_id() != binding.key_id
            {
                return Err(Error::forbidden());
            }
        }
        if !proof.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    })
    .await
    .map_err(|_| Error::storage())?
}
