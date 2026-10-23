//! A weak link to one bounded original publication, never an idle wire grant.
use super::*;
use crate::canonical_media::data::DataInvocation;
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;
pub(crate) fn bind(
    a: &AuthorizedEnvelope,
    invocation: &Arc<DataInvocation>,
) -> connect_protocol::Result<()> {
    let h = a
        .authority::<StoreInvocationAuthority>()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let g = h
        .guard::<Arc<Grant>>()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let PeerCommand::NativeStream(wire) = &g.request.command else {
        return Err(connect_protocol::Error::forbidden());
    };
    if wire.command != connect_protocol::native_streams::Command::Data
        || !invocation.matches(a)
        || !g.proof.current()
        || a.envelope().id() != g.invocation_id
        || crate::canonical_native_streams::intent(a.envelope()).as_ref() != Ok(wire)
    {
        return Err(connect_protocol::Error::forbidden());
    }
    let mut saved = g
        .native_data
        .try_lock()
        .map_err(|_| connect_protocol::Error::forbidden())?;
    if saved
        .upgrade()
        .is_some_and(|old| !Arc::ptr_eq(&old, invocation))
    {
        return Err(connect_protocol::Error::forbidden());
    }
    *saved = Arc::downgrade(invocation);
    Ok(())
}
pub(crate) fn invocation(a: &AuthorizedEnvelope) -> Option<Arc<DataInvocation>> {
    let g = a
        .authority::<StoreInvocationAuthority>()?
        .guard::<Arc<Grant>>()?;
    let i = g.native_data.try_lock().ok()?.upgrade()?;
    i.matches(a).then_some(i)
}
