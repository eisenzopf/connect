//! Current membership, verified endpoint, configured route and fresh observation.
use crate::{audience, identity::actor_from_connection, Actor, DiscoveryRuntime, Store};
use connect_protocol::{provisioning::EndpointTransport, routing::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
const ENDPOINTS_PER_PARTICIPANT: usize = 64;

struct Endpoint {
    id: String,
    device: String,
    transport: EndpointTransport,
    address: String,
    capabilities: Vec<String>,
    verified: bool,
    revision: u64,
    secret_ref: Option<String>,
}

/// Trusted runtime binding. Addresses/references never enter a public result or Debug.
pub struct SelectedEndpointBinding {
    pub selection: ParticipantRoute,
    pub address: String,
    pub secret_ref: Option<String>,
}
impl std::fmt::Debug for SelectedEndpointBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SelectedEndpointBinding(private)")
    }
}

fn authority(
    c: &Connection,
    actor: &Actor,
    request: &ParticipantRouteRequest,
) -> Result<connect_protocol::membership::MemberView> {
    actor.require("resolve")?;
    actor.require("availability_get")?;
    let required = match (request.purpose, request.direction) {
        (RoutePurpose::Message, RouteDirection::Outgoing) => "send",
        (RoutePurpose::Message, RouteDirection::Incoming) => "inbox",
        (RoutePurpose::Session, RouteDirection::Outgoing) => "session_invite",
        (RoutePurpose::Session, RouteDirection::Incoming) => "session_accept",
    };
    actor.require(required)?;
    if request.media.contains(&RouteMedium::Files) {
        actor.require(if request.direction == RouteDirection::Outgoing {
            "artifact_begin"
        } else {
            "artifact_get"
        })?;
    }
    for transport in request
        .transports
        .iter()
        .filter(|_| request.direction == RouteDirection::Outgoing)
    {
        match transport {
            EndpointTransport::Telephone => actor.require("call_start")?,
            EndpointTransport::Sip => actor.require("sip_dial")?,
            _ => (),
        }
    }
    audience::member(c, &actor.identity_id, &request.conversation_id, true)?;
    let identity: Option<String> = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",
            params![request.participant_id, request.conversation_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let target = audience::member(
        c,
        &identity.ok_or_else(Error::forbidden)?,
        &request.conversation_id,
        true,
    )?;
    if !actor.permits_recipient(&target.identity_id)
        || (request.direction == RouteDirection::Incoming
            && actor.identity_id != target.identity_id)
    {
        return Err(Error::forbidden());
    }
    if let Some(device) = &request.preferred_device_id {
        let owned: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM devices WHERE id=?1 AND identity_id=?2 AND revoked=0)",
                params![device, target.identity_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !owned {
            return Err(Error::forbidden());
        }
    }
    Ok(target)
}

fn configured(
    c: &Connection,
    actor: &Actor,
    target: &str,
    request: &ParticipantRouteRequest,
    e: &Endpoint,
    runtime: &DiscoveryRuntime,
) -> Result<RouteReason> {
    use EndpointTransport::*;
    match e.transport {
        Webrtc if runtime.browser.is_some() => {
            let route = runtime.browser.as_ref().ok_or_else(Error::forbidden)?;
            let configured = match actor_from_connection(c, &route.credential_id) {
                Ok(a) => a,
                Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {
                    return Ok(RouteReason::PermissionRequired)
                }
                Err(e) => return Err(e),
            };
            let peer = &route.binding.participant_id;
            if (target != configured.identity_id && target != peer)
                || (actor.identity_id != configured.identity_id && actor.identity_id != *peer)
            {
                return Ok(RouteReason::Unconfigured);
            }
            if !configured.permits("session_get") || !configured.permits_recipient(peer) {
                return Ok(RouteReason::PermissionRequired);
            }
            for identity in [&configured.identity_id, peer] {
                match audience::member(c, identity, &request.conversation_id, true) {
                    Ok(_) => (),
                    Err(e) if e.code == "forbidden" => return Ok(RouteReason::PermissionRequired),
                    Err(e) => return Err(e),
                }
            }
        }
        Sms | VapiChat => {
            let connector = if e.transport == Sms { "telnyx" } else { "vapi" };
            let mut st=c.prepare("SELECT id,credential_id,source FROM connector_routes WHERE conversation_id=?1 AND identity_id=?2 AND connector=?3 AND target=?4 AND enabled=1 AND (?5 IS NULL OR id IN(SELECT value FROM json_each(?5))) ORDER BY id LIMIT 2").map_err(|_|Error::storage())?;
            let routes = st
                .query_map(
                    params![
                        request.conversation_id,
                        target,
                        connector,
                        e.address,
                        runtime.endpoints.message_route_ids()?
                    ],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            if routes.len() > 1 {
                return Ok(RouteReason::Ambiguous);
            }
            let Some((route_id, credential, source)) = routes.first() else {
                return Ok(RouteReason::Unconfigured);
            };
            match runtime.endpoints.message_route_current(c, route_id, &e.id) {
                Ok(true) => (),
                Ok(false) => return Ok(RouteReason::Unconfigured),
                Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {
                    return Ok(RouteReason::PermissionRequired)
                }
                Err(e) => return Err(e),
            }
            let delegate = match actor_from_connection(c, credential) {
                Ok(v) => v,
                Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {
                    return Ok(RouteReason::PermissionRequired)
                }
                Err(e) => return Err(e),
            };
            if delegate.identity_id != target
                || !delegate.permits("send")
                || !delegate.permits_recipient(&actor.identity_id)
            {
                return Ok(RouteReason::PermissionRequired);
            }
            if e.transport == Sms
                && crate::sms_eligibility::eligible(
                    c,
                    &e.address,
                    source.as_deref().unwrap_or_default(),
                    &request.conversation_id,
                )?
                .is_err()
            {
                return Ok(RouteReason::ConsentRequired);
            }
        }
        Telephone => {
            // A voice route's identity is its assistant, while this endpoint's
            // identity owns the dialed phone. Never relabel one as the other.
            let mut st=c.prepare("SELECT identity_id FROM voice_routes r WHERE conversation_id=?1 AND enabled=1 AND EXISTS(SELECT 1 FROM json_each(r.destinations) WHERE value=?2) ORDER BY id LIMIT 2").map_err(|_|Error::storage())?;
            let identities = st
                .query_map(params![request.conversation_id, e.address], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            if identities.is_empty() {
                return Ok(RouteReason::Unconfigured);
            }
            if identities.len() > 1 {
                return Ok(RouteReason::Ambiguous);
            }
            let assistant = &identities[0];
            if !actor.permits_recipient(assistant) {
                return Ok(RouteReason::PermissionRequired);
            }
            match audience::member(c, assistant, &request.conversation_id, true) {
                Ok(_) => (),
                Err(e) if e.code == "forbidden" => return Ok(RouteReason::PermissionRequired),
                Err(e) => return Err(e),
            }
        }
        VapiVoice => {
            if runtime.endpoints.restricts_assistants() {
                let Some(host) = runtime.endpoints.assistant_binding(&e.id) else {
                    return Ok(RouteReason::Unconfigured);
                };
                if host.identity_id != target
                    || e.secret_ref.as_deref() != Some(host.secret_ref.as_str())
                {
                    return Ok(RouteReason::Unconfigured);
                }
                match host.current(c, Some(&actor.identity_id)) {
                    Ok(()) => {}
                    Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {
                        return Ok(RouteReason::PermissionRequired)
                    }
                    Err(e) => return Err(e),
                }
            }
            let count:u64=c.query_row("SELECT COUNT(*) FROM (SELECT assistant_id FROM voice_routes WHERE conversation_id=?1 AND identity_id=?2 AND enabled=1 AND assistant_id=?3 UNION SELECT target FROM connector_routes WHERE conversation_id=?1 AND identity_id=?2 AND enabled=1 AND connector='vapi' AND target=?3)",params![request.conversation_id,target,e.address],|r|r.get(0)).map_err(|_|Error::storage())?;
            if count == 0 {
                return Ok(RouteReason::Unconfigured);
            }
            if count > 1 {
                return Ok(RouteReason::Ambiguous);
            }
            if e.transport == VapiVoice {
                match crate::session_routes::context_actor(c, &request.conversation_id, target) {
                    Ok(_) => (),
                    Err(e) if matches!(e.code.as_str(), "unauthorized" | "forbidden") => {
                        return Ok(RouteReason::PermissionRequired)
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        _ => (),
    }
    Ok(RouteReason::Selected)
}

pub(crate) fn resolve(
    c: &Connection,
    actor: &Actor,
    mut request: ParticipantRouteRequest,
    runtime: &DiscoveryRuntime,
    _store: &Store,
) -> Result<ParticipantRoute> {
    request.normalize()?;
    let target = authority(c, actor, &request)?;
    let (revision, closed): (u64, bool) = c
        .query_row(
            "SELECT revision,closed FROM conversations WHERE id=?1",
            [&request.conversation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    if closed {
        return Err(Error::new("conversation-closed", "Conversation is closed"));
    }
    let mut st=c.prepare("SELECT e.id,e.device_id,e.transport,e.address,e.capabilities_json,e.verification_state,e.revision,e.secret_ref FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.identity_id=?1 AND e.verification_state!='revoked' ORDER BY e.id LIMIT ?2").map_err(|_|Error::storage())?;
    let rows = st
        .query_map(
            params![target.identity_id, ENDPOINTS_PER_PARTICIPANT + 1],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, u64>(6)?,
                    r.get::<_, Option<String>>(7)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if rows.len() > ENDPOINTS_PER_PARTICIPANT {
        return Err(Error::new(
            "capacity-exceeded",
            "Participant endpoint inventory exceeds its bound",
        ));
    }
    let installation: String = c
        .query_row("SELECT id FROM installation", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    let availability = crate::availability::get(c, &target.identity_id)?;
    let mut candidates = vec![];
    for (id, device, transport, address, caps, verified, endpoint_revision, secret_ref) in rows {
        let transport: EndpointTransport =
            serde_json::from_value(serde_json::Value::String(transport))
                .map_err(|_| Error::storage())?;
        if !request.transports.contains(&transport) {
            continue;
        }
        let e = Endpoint {
            id,
            device,
            transport,
            address,
            capabilities: serde_json::from_str(&caps).map_err(|_| Error::storage())?,
            verified: verified == "verified",
            revision: endpoint_revision,
            secret_ref,
        };
        let mut candidate = EndpointCandidate {
            endpoint_id: e.id.clone(),
            device_id: e.device.clone(),
            transport: e.transport.clone(),
            endpoint_revision: e.revision,
            reason: RouteReason::Unknown,
            reachability: EndpointReachability::Unknown,
            observation_sequence: None,
            valid_until: None,
        };
        candidate.reason = if !e.verified {
            RouteReason::Unverified
        } else if !request
            .media
            .iter()
            .all(|m| e.capabilities.iter().any(|c| c == m.capability()))
        {
            RouteReason::UnsupportedCapability
        } else if !runtime
            .endpoints
            .has_secret(&e.transport, e.secret_ref.as_deref())
        {
            RouteReason::Unconfigured
        } else {
            match runtime.endpoints.supports(&e.transport, &request.media) {
                None => RouteReason::Unconfigured,
                Some(false) => RouteReason::UnsupportedCapability,
                Some(true) => configured(c, actor, &target.identity_id, &request, &e, runtime)?,
            }
        };
        if candidate.reason == RouteReason::Selected {
            if let Some(obs) = runtime.endpoints.observation(
                &installation,
                runtime.endpoints.epoch(),
                &e.id,
                e.revision,
                crate::now(),
            )? {
                candidate.reachability = obs.state;
                candidate.observation_sequence = Some(obs.sequence);
                candidate.valid_until = Some(obs.expires_at);
                candidate.reason = match obs.state {
                    EndpointReachability::Reachable => RouteReason::Selected,
                    EndpointReachability::Offline => RouteReason::Offline,
                    EndpointReachability::Busy => RouteReason::Busy,
                    EndpointReachability::Unknown => RouteReason::Unknown,
                };
            } else {
                candidate.reason = RouteReason::Unknown;
            }
            if availability.state == "unavailable" {
                candidate.reachability = EndpointReachability::Offline;
                candidate.reason = RouteReason::Offline;
            }
        }
        candidates.push(candidate);
    }
    let preferred = candidates
        .iter()
        .filter(|v| {
            request
                .preferred_device_id
                .as_ref()
                .is_none_or(|p| p == &v.device_id)
        })
        .collect::<Vec<_>>();
    let mut usable = preferred
        .iter()
        .copied()
        .filter(|v| v.reason == RouteReason::Selected)
        .collect::<Vec<_>>();
    if usable.is_empty() && request.reachability == ReachabilityPolicy::AllowUnknown {
        usable = preferred
            .iter()
            .copied()
            .filter(|v| v.reason == RouteReason::Unknown)
            .collect();
    }
    if usable.is_empty()
        && request.preferred_device_id.is_some()
        && request.failover == FailoverPolicy::OtherAvailableDevice
    {
        usable = candidates
            .iter()
            .filter(|v| v.reason == RouteReason::Selected)
            .collect();
    }
    let (reason, selected) = match usable.as_slice() {
        [one] => (
            if one.reason == RouteReason::Unknown {
                RouteReason::AttemptAllowed
            } else {
                RouteReason::Selected
            },
            Some((*one).clone()),
        ),
        [] => {
            let considered = if request.preferred_device_id.is_some()
                && request.failover == FailoverPolicy::None
            {
                preferred
            } else {
                candidates.iter().collect()
            };
            let reason = considered.first().map_or(RouteReason::NoEndpoint, |first| {
                if considered.iter().all(|v| v.reason == first.reason) {
                    first.reason
                } else {
                    RouteReason::PrerequisitesMissing
                }
            });
            (reason, None)
        }
        _ => (RouteReason::Ambiguous, None),
    };
    Ok(ParticipantRoute {
        conversation_id: request.conversation_id,
        conversation_revision: revision,
        participant_id: target.participant_id,
        membership_epoch: target.membership_epoch,
        reason,
        selected,
        candidates,
    })
}

impl Store {
    pub fn participant_route(
        &self,
        credential: &str,
        request: ParticipantRouteRequest,
        runtime: &DiscoveryRuntime,
    ) -> Result<ParticipantRoute> {
        let mut connection = self.connection()?;
        let c = connection.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&c, credential)?;
        let value = resolve(&c, &actor, request, runtime, self)?;
        actor_from_connection(&c, credential)?;
        Ok(value)
    }
    /// Re-resolve after asynchronous preparation. A stale or forged selection
    /// cannot reveal an address, change participant, or silently fail over.
    pub fn authorize_selected_endpoint(
        &self,
        credential: &str,
        request: ParticipantRouteRequest,
        selected: &ParticipantRoute,
        runtime: &DiscoveryRuntime,
    ) -> Result<SelectedEndpointBinding> {
        let mut connection = self.connection()?;
        let c = connection.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&c, credential)?;
        let current = resolve(&c, &actor, request, runtime, self)?;
        if &current != selected
            || !matches!(
                current.reason,
                RouteReason::Selected | RouteReason::AttemptAllowed
            )
        {
            return Err(Error::new(
                "conflict",
                "Endpoint selection requires fresh resolution",
            ));
        }
        let id = &current
            .selected
            .as_ref()
            .ok_or_else(Error::forbidden)?
            .endpoint_id;
        let (address, secret_ref) = c
            .query_row(
                "SELECT address,secret_ref FROM endpoints WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::storage())?;
        actor_from_connection(&c, credential)?;
        Ok(SelectedEndpointBinding {
            selection: current,
            address,
            secret_ref,
        })
    }
}
