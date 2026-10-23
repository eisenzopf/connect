//! Bounded process-local observations. These are hints, never ownership grants.
use connect_protocol::{provisioning::EndpointTransport, routing::*, Error, Result};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub const ENDPOINT_OBSERVATION_LIMIT: usize = 4096;
pub const ENDPOINT_OBSERVATION_LEASE: i64 = 60;
#[derive(Clone, Debug)]
pub struct EndpointProfile {
    pub transport: EndpointTransport,
    pub media: Vec<RouteMedium>,
}
/// Exact protected native host configuration. These are references, never a
/// wire principal or a grant. Current Store facts are checked on each selection.
#[derive(Clone)]
pub struct AssistantEndpointHost {
    pub endpoint_id: String,
    pub identity_id: String,
    pub credential_id: String,
    pub native_device_id: String,
    pub native_device_kind: String,
    pub native_device_platform: String,
    pub signing_key_id: String,
    pub signing_algorithm: String,
    pub secret_ref: String,
}
impl std::fmt::Debug for AssistantEndpointHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantEndpointHost(redacted)")
    }
}
impl AssistantEndpointHost {
    fn validate(&self) -> Result<()> {
        for id in [
            &self.endpoint_id,
            &self.identity_id,
            &self.credential_id,
            &self.native_device_id,
            &self.signing_key_id,
            &self.secret_ref,
        ] {
            connect_protocol::transitions::id(id)?;
        }
        if !matches!(self.signing_algorithm.as_str(), "EdDSA" | "ES256") {
            return Err(connect_protocol::Error::forbidden());
        }
        if self.native_device_kind.is_empty()
            || self.native_device_kind.len() > 64
            || self.native_device_platform.is_empty()
            || self.native_device_platform.len() > 128
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(crate) fn current(&self, c: &rusqlite::Connection, caller: Option<&str>) -> Result<()> {
        use rusqlite::params;
        self.validate()?;
        let actor = crate::identity::actor_from_connection(c, &self.credential_id)?;
        if actor.identity_id != self.identity_id
            || caller.is_some_and(|id| !actor.permits_recipient(id))
        {
            return Err(Error::forbidden());
        }
        for scope in [
            "context_view",
            "resolve",
            "availability_get",
            "session_accept",
            "session.accept",
            "connection.offer",
            "stream.subscribe",
        ] {
            actor.require(scope)?;
        }
        let tenant: String = c
            .query_row(
                "SELECT tenant_id FROM identities WHERE id=?1",
                [&self.identity_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        let key = crate::peer_credentials::peer_signing_key(
            c,
            &self.credential_id,
            &tenant,
            &self.signing_key_id,
            &self.signing_algorithm,
        )?;
        if key.identity_id != self.identity_id || key.device_id != self.native_device_id {
            return Err(Error::forbidden());
        }
        let bound:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.kind='ai' AND i.revoked=0 JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.transport='vapi-voice' AND e.verification_state='verified' AND e.secret_ref=?3 AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio'))",params![self.endpoint_id,self.identity_id,self.secret_ref],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !bound {
            return Err(Error::forbidden());
        }
        let native:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.identity_id=?1 AND e.device_id=?2 AND d.kind=?3 AND d.platform=?4 AND e.transport='quic' AND e.verification_state='verified' AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio') AND (SELECT COUNT(*) FROM endpoints n WHERE n.identity_id=e.identity_id AND n.device_id=e.device_id AND n.transport='quic' AND n.verification_state='verified')=1)",params![self.identity_id,self.native_device_id,self.native_device_kind,self.native_device_platform],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !native {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationSource {
    NativePeer,
    BrowserPeer,
    SipDialog,
    ProviderProbe,
    LocalFixture,
}
/// Trusted adapters construct this from an actual attributed observation. It is
/// deliberately not deserializable as a public command or provider callback.
#[derive(Clone)]
pub struct EndpointObservation {
    pub installation_id: String,
    pub runtime_epoch: String,
    pub endpoint_id: String,
    pub endpoint_revision: u64,
    pub sequence: u64,
    pub observed_at: i64,
    pub expires_at: i64,
    pub state: EndpointReachability,
    pub source: ObservationSource,
}
impl std::fmt::Debug for EndpointObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointObservation")
            .field("state", &self.state)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}
#[derive(Clone)]
pub struct EndpointRuntime {
    epoch: Arc<String>,
    profiles: Arc<Vec<EndpointProfile>>,
    observations: Arc<Mutex<BTreeMap<(String, String), EndpointObservation>>>,
    secret_refs: Arc<Vec<(EndpointTransport, String)>>,
    assistant_hosts: Arc<Vec<AssistantEndpointHost>>,
    message_bindings: Option<Arc<Vec<crate::ProviderMessageEndpoint>>>,
}
impl Default for EndpointRuntime {
    fn default() -> Self {
        Self {
            epoch: Arc::new(format!(
                "endpoint_runtime_{}",
                uuid::Uuid::new_v4().simple()
            )),
            profiles: Default::default(),
            observations: Default::default(),
            secret_refs: Default::default(),
            assistant_hosts: Default::default(),
            message_bindings: None,
        }
    }
}
impl EndpointRuntime {
    pub fn with_message_bindings(
        mut self,
        bindings: Vec<crate::ProviderMessageEndpoint>,
    ) -> Result<Self> {
        if bindings.is_empty()
            || bindings.len() > 512
            || bindings
                .iter()
                .enumerate()
                .any(|(i, p)| bindings[..i].iter().any(|old| old.route_id == p.route_id))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid mounted message Endpoints",
            ));
        }
        let mut profiles = (*self.profiles).clone();
        let mut refs = (*self.secret_refs).clone();
        for p in &bindings {
            let transport = match p.connector.as_str() {
                "vapi" => EndpointTransport::VapiChat,
                "telnyx" => EndpointTransport::Sms,
                _ => return Err(Error::forbidden()),
            };
            if !profiles.iter().any(|v| v.transport == transport) {
                profiles.push(EndpointProfile {
                    transport: transport.clone(),
                    media: vec![RouteMedium::Messages],
                });
            }
            if !refs.contains(&(transport.clone(), p.secret_reference.clone())) {
                refs.push((transport, p.secret_reference.clone()));
            }
        }
        if profiles.len() > 8 || refs.len() > 64 {
            return Err(Error::new(
                "capacity-exceeded",
                "Mounted message profile budget exceeded",
            ));
        }
        self.profiles = Arc::new(profiles);
        self.secret_refs = Arc::new(refs);
        self.message_bindings = Some(Arc::new(bindings));
        Ok(self)
    }
    pub(crate) fn message_route_ids(&self) -> Result<Option<String>> {
        self.message_bindings
            .as_ref()
            .map(|v| serde_json::to_string(&v.iter().map(|p| &p.route_id).collect::<Vec<_>>()))
            .transpose()
            .map_err(|_| Error::storage())
    }
    pub(crate) fn message_route_current(
        &self,
        c: &rusqlite::Connection,
        route: &str,
        endpoint: &str,
    ) -> Result<bool> {
        let Some(bindings) = &self.message_bindings else {
            return Ok(true);
        };
        let Some(p) = bindings
            .iter()
            .find(|p| p.route_id == route && p.endpoint_id == endpoint)
        else {
            return Ok(false);
        };
        Ok(
            crate::provider_message_endpoints::current(c, &crate::providers::route(c, route)?)?
                .as_ref()
                == Some(p),
        )
    }
    /// Restrict a constructed native Vapi host to exactly its configured
    /// Endpoint/credential/key/secret bindings. Other transports retain their
    /// own adapter profiles; IDs themselves never establish authorization.
    pub fn with_assistant_hosts(mut self, hosts: Vec<AssistantEndpointHost>) -> Result<Self> {
        if hosts.is_empty() || hosts.len() > 512 {
            return Err(Error::new(
                "invalid-request",
                "Invalid assistant host capacity",
            ));
        }
        if self.supports(&EndpointTransport::VapiVoice, &[RouteMedium::Audio]) != Some(true)
            || hosts
                .iter()
                .any(|h| !self.has_secret(&EndpointTransport::VapiVoice, Some(&h.secret_ref)))
        {
            return Err(Error::new("invalid-request","Assistant hosts require a constructed voice profile and selected secret references"));
        }
        for (i, host) in hosts.iter().enumerate() {
            host.validate()?;
            if hosts[..i]
                .iter()
                .any(|h| h.endpoint_id == host.endpoint_id || h.identity_id == host.identity_id)
            {
                return Err(Error::new("invalid-request", "Duplicate assistant host"));
            }
        }
        self.assistant_hosts = Arc::new(hosts);
        Ok(self)
    }
    pub(crate) fn assistant_binding(&self, id: &str) -> Option<&AssistantEndpointHost> {
        self.assistant_hosts.iter().find(|h| h.endpoint_id == id)
    }
    pub(crate) fn restricts_assistants(&self) -> bool {
        !self.assistant_hosts.is_empty()
    }
    pub(crate) fn assistant_configured(
        &self,
        c: &rusqlite::Connection,
        identity: &str,
        caller: &str,
    ) -> Result<bool> {
        if !self.restricts_assistants() {
            return Ok(true);
        }
        for host in self
            .assistant_hosts
            .iter()
            .filter(|h| h.identity_id == identity)
        {
            match host.current(c, Some(caller)) {
                Ok(()) => return Ok(true),
                Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => {}
                Err(e) => return Err(e),
            }
        }
        Ok(false)
    }
    /// Reference names identify already constructed credential bindings, never
    /// environment contents. Availability is transport-specific.
    pub fn with_secret_refs(mut self, refs: Vec<(EndpointTransport, String)>) -> Result<Self> {
        if refs.len() > 64
            || refs.iter().enumerate().any(|(i, (t, r))| {
                r.is_empty()
                    || r.len() > 128
                    || !r
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
                    || refs[..i].iter().any(|p| p == &(t.clone(), r.clone()))
            })
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid endpoint credential reference bindings",
            ));
        }
        self.secret_refs = Arc::new(refs);
        Ok(self)
    }
    pub(crate) fn has_secret(
        &self,
        transport: &EndpointTransport,
        reference: Option<&str>,
    ) -> bool {
        match reference {
            Some(reference) => self
                .secret_refs
                .iter()
                .any(|(t, r)| t == transport && r == reference),
            None => matches!(
                transport,
                EndpointTransport::Quic
                    | EndpointTransport::Webtransport
                    | EndpointTransport::Webrtc
            ),
        }
    }
    /// Capture when an adapter observer starts; never relabel a delayed event
    /// with a replacement runtime's epoch, even in the same server process.
    pub fn epoch(&self) -> &str {
        &self.epoch
    }
    pub fn new(profiles: Vec<EndpointProfile>) -> Result<Self> {
        if profiles.len() > 8
            || profiles
                .iter()
                .any(|p| p.media.is_empty() || p.media.len() > 6)
            || profiles.iter().any(|p| {
                p.media
                    .iter()
                    .enumerate()
                    .any(|(i, m)| p.media[..i].contains(m))
                    || p.media.iter().any(|m| match p.transport {
                        EndpointTransport::Sms | EndpointTransport::VapiChat => {
                            *m != RouteMedium::Messages
                        }
                        EndpointTransport::Sip
                        | EndpointTransport::Telephone
                        | EndpointTransport::VapiVoice => *m != RouteMedium::Audio,
                        _ => false,
                    })
            })
            || profiles
                .iter()
                .enumerate()
                .any(|(i, p)| profiles[..i].iter().any(|q| q.transport == p.transport))
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid runtime endpoint profile",
            ));
        }
        Ok(Self {
            profiles: Arc::new(profiles),
            ..Default::default()
        })
    }
    pub(crate) fn supports(
        &self,
        transport: &EndpointTransport,
        media: &[RouteMedium],
    ) -> Option<bool> {
        self.profiles
            .iter()
            .find(|p| p.transport == *transport)
            .map(|p| media.iter().all(|m| p.media.contains(m)))
    }
    pub(crate) fn record(&self, value: EndpointObservation, now: i64) -> Result<()> {
        if value.runtime_epoch != self.epoch() {
            return Err(Error::forbidden());
        }
        if value.observed_at < 0
            || value.sequence == 0
            || value.sequence > 9_007_199_254_740_991
            || value.observed_at > now
            || value.expires_at <= now
            || value.expires_at <= value.observed_at
            || value.expires_at.saturating_sub(value.observed_at) > ENDPOINT_OBSERVATION_LEASE
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid endpoint observation lifetime or sequence",
            ));
        }
        let mut observations = self.observations.lock().map_err(|_| Error::storage())?;
        let key = (value.installation_id.clone(), value.endpoint_id.clone());
        if let Some(old) = observations.get(&key) {
            if value.runtime_epoch == old.runtime_epoch
                && (value.sequence <= old.sequence
                    || value.observed_at < old.observed_at
                    || value.endpoint_revision < old.endpoint_revision)
            {
                return Err(Error::new("conflict", "Stale endpoint observation"));
            }
        } else if observations.len() >= ENDPOINT_OBSERVATION_LIMIT {
            return Err(Error::new(
                "capacity-exceeded",
                "Endpoint observation capacity exceeded",
            ));
        }
        observations.insert(key, value);
        Ok(())
    }
    pub(crate) fn observation(
        &self,
        installation: &str,
        epoch: &str,
        id: &str,
        revision: u64,
        now: i64,
    ) -> Result<Option<EndpointObservation>> {
        Ok(self
            .observations
            .lock()
            .map_err(|_| Error::storage())?
            .get(&(installation.into(), id.into()))
            .filter(|v| {
                v.runtime_epoch == epoch
                    && v.endpoint_revision == revision
                    && v.observed_at <= now
                    && v.expires_at > now
            })
            .cloned())
    }
}

impl crate::Store {
    /// Protected startup preflight. The value is configuration metadata, not an
    /// execution lease; routing and native/provider admission repeat authority.
    pub fn validate_assistant_host(&self, host: &AssistantEndpointHost) -> Result<()> {
        let c = self.connection()?;
        host.current(&c, None)
    }
    pub fn record_endpoint_observation(
        &self,
        runtime: &EndpointRuntime,
        value: EndpointObservation,
    ) -> Result<()> {
        let mut connection = self.connection()?;
        let c = connection.transaction().map_err(|_| Error::storage())?;
        let installation: String = c
            .query_row("SELECT id FROM installation", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.tenant_id=e.tenant_id AND i.revoked=0 JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.id=?1 AND e.revision=?2 AND e.verification_state='verified')",rusqlite::params![value.endpoint_id,value.endpoint_revision],|r|r.get(0)).map_err(|_|Error::storage())?;
        if value.installation_id != installation
            || value.runtime_epoch != runtime.epoch()
            || !current
        {
            return Err(Error::forbidden());
        }
        runtime.record(value, crate::now())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn observation(runtime: &EndpointRuntime, id: String) -> EndpointObservation {
        EndpointObservation {
            installation_id: "fixture".into(),
            runtime_epoch: runtime.epoch().into(),
            endpoint_id: id,
            endpoint_revision: 1,
            sequence: 1,
            observed_at: 100,
            expires_at: 160,
            state: EndpointReachability::Reachable,
            source: ObservationSource::LocalFixture,
        }
    }
    #[test]
    fn observation_capacity_retains_fences_and_accepts_only_newer_updates() {
        let runtime = EndpointRuntime::default();
        for n in 0..ENDPOINT_OBSERVATION_LIMIT {
            runtime
                .record(observation(&runtime, format!("ep_{n}")), 100)
                .unwrap();
        }
        assert_eq!(
            runtime
                .record(observation(&runtime, "overflow".into()), 100)
                .unwrap_err()
                .code,
            "capacity-exceeded"
        );
        let mut current = observation(&runtime, "ep_0".into());
        assert_eq!(
            runtime.record(current.clone(), 100).unwrap_err().code,
            "conflict"
        );
        current.sequence = 2;
        current.state = EndpointReachability::Offline;
        runtime.record(current, 100).unwrap();
        assert_eq!(
            runtime
                .observation("fixture", runtime.epoch(), "ep_0", 1, 101)
                .unwrap()
                .unwrap()
                .state,
            EndpointReachability::Offline
        );
        assert!(runtime
            .observation("fixture", runtime.epoch(), "ep_0", 1, 160)
            .unwrap()
            .is_none());
        assert_eq!(
            runtime
                .record(observation(&runtime, "overflow".into()), 161)
                .unwrap_err()
                .code,
            "invalid-request"
        );
    }
    #[test]
    fn runtime_profiles_reject_duplicate_transports_and_impossible_media() {
        let profile = EndpointProfile {
            transport: EndpointTransport::Quic,
            media: vec![RouteMedium::Audio],
        };
        assert!(EndpointRuntime::new(vec![profile.clone(), profile]).is_err());
        assert!(EndpointRuntime::new(vec![EndpointProfile {
            transport: EndpointTransport::Sms,
            media: vec![RouteMedium::Video]
        }])
        .is_err());
        let runtime = EndpointRuntime::default();
        let mut value = observation(&runtime, "one".into());
        value.observed_at = i64::MIN;
        value.expires_at = i64::MAX;
        assert!(runtime.record(value, 100).is_err());
    }
}
