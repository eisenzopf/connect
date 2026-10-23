//! Explicit actual native assistant destination and its current public audience.
use super::*;
use serde_json::Value;
mod provider;
pub use provider::{
    AssistantContextFailure, AssistantProviderApplied, AssistantProviderAttempt,
    AssistantProviderClosed, AssistantProviderContext, AssistantProviderCreated,
    AssistantProviderEnd, AssistantProviderExport, AssistantProviderInitialization,
    AssistantProviderRead, AssistantProviderRejected, AssistantProviderTiming,
    AssistantProviderUpdate, AssistantProviderUpdateApplied, AssistantProviderUpdateExport,
    AssistantProviderUpdateRejected, AssistantProviderUpdateStatus,
};

/// One bounded current interval for a server-owned assistant export. The JSON
/// is data, not a future grant. Retain this read through the exact bounded IO;
/// the host must separately bind/capture its actual provider attempt and receipt.
pub struct NativeAssistantContextRead {
    native: NativeMediaRead,
    origin: NativeMediaOrigin,
    credential: String,
    endpoint_id: String,
    json: String,
    endpoint: Value,
    audience: Vec<(String, String, u64)>,
    routing: Option<(
        crate::DiscoveryRuntime,
        connect_protocol::routing::ReachabilityPolicy,
    )>,
}
impl std::fmt::Debug for NativeAssistantContextRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeAssistantContextRead(redacted)")
    }
}
impl NativeAssistantContextRead {
    pub fn current(&self) -> bool {
        self.native.current()
    }
    pub fn expires_at(&self) -> i64 {
        self.native.expires_at()
    }
    pub fn target(&self) -> &PeerConnectionProjection {
        self.native.target()
    }
    pub fn context_json(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(&self.json)
    }
    pub fn provider_endpoint(&self) -> Result<&Value> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(&self.endpoint)
    }
    pub fn audience(&self) -> &[(String, String, u64)] {
        &self.audience
    }
}

fn check_route(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
    actor: &crate::Actor,
    endpoint: &Value,
    endpoint_id: &str,
    routing: &(
        crate::DiscoveryRuntime,
        connect_protocol::routing::ReachabilityPolicy,
    ),
) -> Result<()> {
    use connect_protocol::{provisioning::EndpointTransport, routing::*};
    let selected = crate::participant_routes::resolve(
        c,
        actor,
        ParticipantRouteRequest {
            conversation_id: origin.conversation_id().into(),
            participant_id: origin.participant_id().into(),
            purpose: RoutePurpose::Session,
            direction: RouteDirection::Incoming,
            media: vec![RouteMedium::Audio],
            transports: vec![EndpointTransport::VapiVoice],
            preferred_device_id: endpoint["device_id"].as_str().map(str::to_owned),
            failover: FailoverPolicy::None,
            reachability: routing.1,
        },
        &routing.0,
        store,
    )?;
    if !matches!(
        selected.reason,
        RouteReason::Selected | RouteReason::AttemptAllowed
    ) || selected.selected.as_ref().map(|s| s.endpoint_id.as_str()) != Some(endpoint_id)
    {
        return Err(Error::new(
            "endpoint-unavailable",
            "Configured assistant endpoint is not currently eligible",
        ));
    }
    Ok(())
}

struct Facts {
    view: PeerConnectionProjection,
    scopes: Vec<AuthorityScope>,
    expires: i64,
    actor: crate::Actor,
    delegate: crate::Actor,
    endpoint: Value,
    audience: Vec<(String, String, u64)>,
}
fn resolve(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
    context: &str,
    endpoint: &str,
) -> Result<Facts> {
    let (view, scopes, mut expires) = facts(store, c, origin)?;
    let actor = crate::identity::actor_from_connection(c, context)?;
    let delegate = store.native_media_route_actor(c, origin)?;
    actor.require("context_view")?;
    delegate.require("context_view")?;
    let cid = origin.conversation_id();
    crate::conversations::authorized_conversation(c, &actor, cid)?;
    crate::audience::require(
        c,
        &actor,
        cid,
        connect_protocol::membership::ContentKind::Session,
        origin.session_id(),
    )?;
    // Private physical destinations require their exact consultation purpose,
    // creation and memory lifecycle. This public path never borrows that grant.
    let private: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM native_consultation_legs WHERE connection_id=?1)",
            [origin.connection_id()],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if private {
        return Err(Error::new(
            "private-context-required",
            "Use the Connection-bound private provider context",
        ));
    }
    let mut q = c.prepare("SELECT p.identity_id,p.participant_id,p.membership_epoch FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.conversation_id=sp.conversation_id AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0 WHERE sp.session_id=?1 AND sp.conversation_id=?2 AND sp.state='present' ORDER BY p.participant_id").map_err(|_|Error::storage())?;
    let mut audience = q
        .query_map(params![origin.session_id(), cid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    if let Some(eligible) = crate::connection_control::handoff_preparation::observation_audience(
        c,
        origin.connection_id(),
    )? {
        audience.retain(|(_, pid, epoch)| eligible.contains(&(pid.clone(), *epoch)));
    }
    if !audience.iter().any(|(id, _, _)| id == &actor.identity_id)
        || !audience.iter().any(|(id, pid, epoch)| {
            id == &delegate.identity_id
                && pid == origin.participant_id()
                && *epoch == origin.membership_epoch()
        })
        || audience
            .iter()
            .any(|(id, _, _)| !actor.permits_recipient(id) || !delegate.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    let ai: bool = c.query_row("SELECT p.kind='ai' AND i.kind='ai' FROM participants p JOIN identities i ON i.id=p.identity_id WHERE p.participant_id=?1 AND p.membership_epoch=?2 AND p.identity_id=?3",params![origin.participant_id(),origin.membership_epoch(),delegate.identity_id], |r|r.get(0)).map_err(|_|Error::forbidden())?;
    if !ai {
        return Err(Error::new(
            "unsupported-media-profile",
            "Assistant context requires an AI Participant",
        ));
    }
    let endpoint: Value = c.query_row("SELECT e.id,e.revision,e.address,e.secret_ref,e.device_id,d.kind,d.platform FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.transport='vapi-voice' AND e.verification_state='verified' AND e.secret_ref IS NOT NULL AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio')",params![endpoint,delegate.identity_id],|r|Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"revision":r.get::<_,u64>(1)?,"address":r.get::<_,String>(2)?,"secret_ref":r.get::<_,String>(3)?,"device_id":r.get::<_,String>(4)?,"device_kind":r.get::<_,String>(5)?,"device_platform":r.get::<_,String>(6)?}))).map_err(|_|Error::forbidden())?;
    expires = expires
        .min(actor.expires_at)
        .min(delegate.expires_at)
        .min(crate::now() + 5);
    Ok(Facts {
        view,
        scopes,
        expires,
        actor,
        delegate,
        endpoint,
        audience,
    })
}
impl Store {
    /// Initial public assistant admission through the same current endpoint
    /// policy used by participant routing. The actual native read holds the
    /// Conversation/Identity scopes while routing rechecks its dependencies.
    /// Unknown reachability is allowed only by explicit protected host policy.
    pub fn try_routed_native_assistant_context_read(
        &self,
        target: &NativeMediaOrigin,
        context_credential: &str,
        provider_endpoint: &str,
        runtime: &crate::DiscoveryRuntime,
        reachability: connect_protocol::routing::ReachabilityPolicy,
    ) -> Result<NativeAssistantContextRead> {
        let mut read =
            self.try_native_assistant_context_read(target, context_credential, provider_endpoint)?;
        let c = self.try_connection()?;
        let actor = crate::identity::actor_from_connection(&c, context_credential)?;
        let routing = (runtime.clone(), reachability);
        check_route(
            self,
            &c,
            target,
            &actor,
            &read.endpoint,
            provider_endpoint,
            &routing,
        )?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        read.routing = Some(routing);
        Ok(read)
    }
    /// The target is a Store-issued actual native incarnation, never the first
    /// Session invitee or an untrusted identity/endpoint supplied by a client.
    /// This read performs no provider IO or delivery acknowledgment.
    pub fn try_native_assistant_context_read(
        &self,
        target: &NativeMediaOrigin,
        context_credential: &str,
        provider_endpoint: &str,
    ) -> Result<NativeAssistantContextRead> {
        self.try_native_assistant_context_read_at(
            target,
            context_credential,
            provider_endpoint,
            None,
        )
    }
    fn try_native_assistant_context_read_at(
        &self,
        target: &NativeMediaOrigin,
        context_credential: &str,
        provider_endpoint: &str,
        through: Option<u64>,
    ) -> Result<NativeAssistantContextRead> {
        let needed = {
            let c = self.try_connection()?;
            resolve(self, &c, target, context_credential, provider_endpoint)?.scopes
        };
        let held = self.try_acquire_authority(&needed)?;
        let c = self.try_connection()?;
        let f = resolve(self, &c, target, context_credential, provider_endpoint)?;
        if !self.authority_covers(&held, &f.scopes) || crate::now() >= f.expires {
            return Err(Error::forbidden());
        }
        let context = crate::context::view_for_audience_scope_at(
            self,
            &c,
            &f.actor,
            target.conversation_id(),
            1000,
            Some(&f.delegate.identity_id),
            Some(&f.delegate),
            through,
            None,
            Some(&f.audience),
        )?;
        let json = serde_json::to_string(&context).map_err(|_| Error::storage())?;
        if json.len() > 65_536 {
            return Err(Error::new(
                "capacity-exceeded",
                "Assistant context exceeds its byte budget",
            ));
        }
        let read = NativeAssistantContextRead {
            origin: target.clone(),
            credential: context_credential.into(),
            endpoint_id: provider_endpoint.into(),
            native: NativeMediaRead {
                target: f.view,
                source: None,
                _scopes: held,
                owner: current_owner(self, target)?,
                expires_at: crate::records::context::deadline(&context)
                    .map_or(f.expires, |until| f.expires.min(until)),
                may_subscribe: subscription_permission(&c, target)?,
            },
            json,
            endpoint: f.endpoint,
            audience: f.audience,
            routing: None,
        };
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
}
