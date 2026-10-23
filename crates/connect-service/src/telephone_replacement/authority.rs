//! Logical endpoint ownership is resolved separately from its current route actor.
use super::*;
use crate::authentication::StoreValidator;
use connect_store::{NativeReplacementPreparationRead, TelephoneReplacementRead};
use rvoip_core::{
    ids::{DeviceId, EndpointId, IdentityId},
    relationships::{EndpointBinding, IdentityBinding, Membership},
    AuthenticatedPrincipal, Transport,
};

pub(crate) struct Target {
    pub principal: AuthenticatedPrincipal,
    pub member: Membership,
    pub endpoint: EndpointBinding,
}
/// Called while the original/source/retained/route-owner interval is held. The
/// independently selected credential must cover BOTH logical endpoint owner and
/// retained recipient. No old native or operator credential is lent to SIP.
pub(crate) fn target(
    runtime: &Runtime,
    read: &NativeReplacementPreparationRead,
    route: &TelephoneReplacementRoute,
    destination: &str,
    original: &Membership,
) -> Result<Target> {
    let ctx = read.context();
    let actor = runtime.media.store.revalidate(&route.credential_id)?;
    actor.require("sip_dial")?;
    if !read.current()
        || ctx.target_transport != connect_protocol::provisioning::EndpointTransport::Telephone
        || ctx.target_secret_ref.as_deref() != Some(route.policy.route_reference.as_str())
        || actor.identity_id != ctx.target_route_owner_id
        || !actor.permits_recipient(&ctx.identity_id)
        || !actor.permits_recipient(&read.retained().target().session.actor.identity_id)
        || original.identity().as_str() != ctx.identity_id
        || original.participant().as_str() != ctx.participant_id
        || original.epoch() != ctx.membership_epoch
        || destination != format!("sip:{}@{}", ctx.target_address, route.policy.sip_authority)
    {
        return Err(Error::forbidden());
    }
    let principal = StoreValidator::canonical(runtime.media.store.clone())
        .principal(actor)
        .map_err(|_| Error::forbidden())?;
    let identity = IdentityBinding::from_authenticated(
        &principal,
        IdentityId::from_string(ctx.identity_id.clone()),
        chrono::Utc::now(),
    )
    .map_err(|_| Error::forbidden())?;
    // This is an explicit trusted resolver mapping, not credential renewal.
    // Preserve the verified logical membership while independently binding its
    // physical telephone route to the current configured account principal.
    let member = Membership::from_current(
        identity.clone(),
        original.conversation().clone(),
        original.participant().clone(),
        original.kind().clone(),
        original.default_role().clone(),
        original.epoch(),
        original.revision(),
        original.is_active(),
    )
    .map_err(|_| Error::forbidden())?;
    let mut endpoint = EndpointBinding::provision(
        identity.clone(),
        EndpointId::from_string(ctx.target_endpoint_id.clone()),
        DeviceId::from_string(ctx.target_device_id.clone()),
        Transport::Sip,
        destination.to_owned(),
    )
    .map_err(|_| Error::forbidden())?;
    endpoint
        .verify(
            &identity,
            "connect-store-telephone-route".into(),
            chrono::Utc::now(),
        )
        .map_err(|_| Error::forbidden())?;
    Ok(Target {
        principal,
        member,
        endpoint,
    })
}

pub(crate) fn allocated_target(
    runtime: &Runtime,
    read: &TelephoneReplacementRead,
    route: &TelephoneReplacementRoute,
    original: &Membership,
) -> Result<Target> {
    let target = target(
        runtime,
        read.source(),
        route,
        &read.destination()?,
        original,
    )?;
    if target.principal.subject != read.route_actor().identity_id
        || target.member.identity().as_str() != read.target_member().identity_id
        || target.member.epoch() != read.target_member().membership_epoch
    {
        return Err(Error::forbidden());
    }
    Ok(target)
}
