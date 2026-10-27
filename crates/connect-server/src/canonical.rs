//! Production local canonical connect/2 listener. Uses the durable service;
//! unsupported media/task families remain explicit refusals, never stock fallthrough.
use anyhow::{Context, Result};
use connect_service::{CanonicalHandler, Service, StoreKeyResolver, StoreValidator};
use connect_store::Store;
use rvoip_auth_core::sig9421::{Sig9421Signer, Sig9421Verifier};
use rvoip_uctp::v2::{
    auth_dispatch::AuthenticationMethods,
    auth_peer::PeerConfig,
    server::{CanonicalProfile, ProfileServer},
    RAW_QUIC_ALPN,
};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::task::{JoinHandle, JoinSet};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalServerConfig {
    #[serde(default)]
    pub network_profile: connect_protocol::native_network::NativeNetworkProfile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webtransport: Option<crate::canonical_webtransport::CanonicalWebTransportConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<crate::canonical_websocket::CanonicalWebSocketConfig>,
    /// Explicit bounded local WebRTC media; WSS alone remains control-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webrtc: Option<crate::canonical_webrtc::CanonicalWebRtcConfig>,
    pub bind: SocketAddr,
    pub database: PathBuf,
    pub certificate: PathBuf,
    pub private_key: PathBuf,
    pub signing_key_id: String,
    pub signing_algorithm: String,
    pub signing_private_key: PathBuf,
    pub signing_key_not_before: chrono::DateTime<chrono::Utc>,
    pub signing_key_expires_at: chrono::DateTime<chrono::Utc>,
    pub http_bind: Option<SocketAddr>,
    /// Protected local MCP control; disabled unless explicitly configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<crate::config::McpConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_https: Option<crate::transports::record_https::RecordHttpsConfig>,
    #[serde(default)]
    pub record_retention: connect_protocol::record_retention::Policy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_assurance: Vec<connect_protocol::user_delegation::UserAssuranceRule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telephone: Option<crate::canonical_telephone::CanonicalTelephoneConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_vapi: Option<crate::canonical_assistants::CanonicalPublicVapiConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_voice: Option<crate::canonical_provider_voice::CanonicalProviderVoiceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_messages: Option<crate::canonical_messages::CanonicalMessagesConfig>,
    #[serde(default)]
    pub limits: connect_protocol::Limits,
    #[serde(default = "default_session_reconnect_grace")]
    pub session_reconnect_grace_seconds: u32,
}
fn default_session_reconnect_grace() -> u32 {
    30
}
impl CanonicalServerConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path).context("Read canonical server configuration")?;
        let mut value: Self = toml::from_str(&raw)
            .map_err(|_| anyhow::anyhow!("Invalid canonical server configuration"))?;
        for p in [
            &mut value.database,
            &mut value.certificate,
            &mut value.private_key,
            &mut value.signing_private_key,
        ] {
            if p.is_relative() {
                *p = path.parent().unwrap_or(Path::new(".")).join(&*p);
            }
        }
        if let Some(hosting) = &mut value.public_vapi {
            hosting.resolve_paths(path.parent().unwrap_or(Path::new(".")));
        }
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<()> {
        if let Some(mcp) = &self.mcp {
            mcp.validate()?;
        }
        anyhow::ensure!(
            self.user_assurance.len() <= 64,
            "Assurance policy capacity exceeded"
        );
        for rule in &self.user_assurance {
            rule.validate()?;
        }
        self.record_retention.validate()?;
        if let Some(webtransport) = &self.webtransport {
            webtransport.validate(self.network_profile)?;
        }
        if let Some(websocket) = &self.websocket {
            websocket.validate(self.network_profile)?;
        }
        if let Some(webrtc) = &self.webrtc {
            webrtc.validate(
                self.network_profile,
                self.websocket.is_some(),
                self.limits.connections,
            )?;
        }
        anyhow::ensure!(
            self.session_reconnect_grace_seconds <= 300,
            "Invalid Session reconnect grace"
        );
        if let Some(messages) = &self.provider_messages {
            messages.validate(self.limits.connections)?;
        }
        if let Some(records) = &self.record_https {
            records.validate()?;
        }
        let now = chrono::Utc::now();
        anyhow::ensure!(
            self.network_profile.permits_listener(self.bind)
                && self.http_bind.is_none_or(|a| a.ip().is_loopback()),
            "Canonical native listener requires its explicit network profile; plaintext HTTP remains loopback"
        );
        anyhow::ensure!(
            (1..=1024).contains(&self.limits.connections)
                && (1..=60_000).contains(&self.limits.request_timeout_ms),
            "Invalid canonical listener bounds"
        );
        anyhow::ensure!(
            matches!(self.signing_algorithm.as_str(), "EdDSA" | "ES256"),
            "Invalid canonical signing algorithm"
        );
        anyhow::ensure!(
            self.signing_key_not_before <= now
                && self.signing_key_expires_at > now
                && self.signing_key_not_before < self.signing_key_expires_at,
            "Canonical signing key is not current"
        );
        rvoip_uctp::v2::Identifier::try_from(self.signing_key_id.clone())
            .map_err(|_| anyhow::anyhow!("Invalid canonical signing key ID"))?;
        if let Some(telephone) = &self.telephone {
            telephone.validate(self.limits.connections)?;
        }
        if let Some(voice) = &self.provider_voice {
            voice.validate(self.limits.connections)?;
        }
        if let Some(hosting) = &self.public_vapi {
            hosting.validate(self.limits.connections)?;
        }
        Ok(())
    }
}
/// Trusted process-local adapters; never accepted from public configuration JSON.
#[derive(Default)]
pub struct CanonicalConnectors {
    pub messages: Option<Vec<Arc<dyn connect_connector_api::Connector>>>,
    pub sms_policy: Option<Arc<dyn crate::SmsPolicy>>,
    pub media: Vec<crate::connector_media::LiveTransport>,
    pub message_routes: Vec<crate::canonical_messages::MessageRoute>,
}
/// Owns the installation, native intake, authenticated peers, idle policy and
/// optional HTTP facade. Dropping it fences the installation and every peer.
pub struct RunningCanonicalServer {
    installation: crate::installation::InstallationLock,
    endpoint: Arc<quinn::Endpoint>,
    profile: ProfileServer,
    webtransport: Option<ProfileServer>,
    websocket: Option<crate::canonical_websocket_listener::Listener>,
    maximum_rtt_ms: Arc<AtomicU64>,
    stop: CancellationToken,
    intake: Option<JoinHandle<Result<()>>>,
    http: Option<crate::HttpListener>,
    mcp: Option<connect_mcp::RunningMcp>,
    record_https: Option<crate::transports::record_https::RecordHttpsListener>,
    idle: Option<crate::conversation_policy::Worker>,
    records: Option<crate::record_outcomes::Worker>,
    telephone: Option<crate::canonical_telephone::TelephoneTransport>,
    public_assistants: Option<crate::canonical_assistants::Hosting>,
    provider_voice: Option<crate::canonical_provider_voice::Worker>,
    provider_messages: Option<crate::canonical_messages::Host>,
    #[cfg(any(test, feature = "test-support"))]
    webrtc_host: Option<Arc<crate::canonical_webrtc::SignalingHost>>,
    contributed_media: Option<crate::connector_media::Host>,
    pub telnyx_callback_address: Option<SocketAddr>,
    pub address: SocketAddr,
    pub http_address: Option<SocketAddr>,
    pub mcp_address: Option<SocketAddr>,
    pub webtransport_url: Option<String>,
    pub websocket_address: Option<SocketAddr>,
    pub websocket_url: Option<String>,
    pub record_https_address: Option<SocketAddr>,
    pub store: Arc<Store>,
    pub service: Arc<Service>,
}
impl RunningCanonicalServer {
    /// Fixture observation only; startup and actual media use the production host.
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_webrtc_host(&self) -> Option<Arc<crate::canonical_webrtc::SignalingHost>> {
        self.webrtc_host.clone()
    }
    pub async fn start(config: CanonicalServerConfig) -> Result<Self> {
        Box::pin(Self::start_with_connectors(
            config,
            CanonicalConnectors::default(),
        ))
        .await
    }
    /// Protected adapter composition; public clients cannot inject credentials
    /// or providers. The same exact config/account/Endpoint checks still run.
    pub async fn start_with_message_drivers(
        config: CanonicalServerConfig,
        drivers: Option<Vec<Arc<dyn connect_connector_api::Connector>>>,
        policy: Option<Arc<dyn crate::SmsPolicy>>,
    ) -> Result<Self> {
        Box::pin(Self::start_with_connectors(
            config,
            CanonicalConnectors {
                messages: drivers,
                sms_policy: policy,
                media: vec![],
                message_routes: vec![],
            },
        ))
        .await
    }
    /// Own contributed transports on the same canonical Core before intake.
    /// Startup refusal observes bounded cleanup; cancellation requests fencing.
    pub async fn start_with_connectors(
        config: CanonicalServerConfig,
        connectors: CanonicalConnectors,
    ) -> Result<Self> {
        let media = crate::connector_media::Host::new(connectors.media);
        // Startup includes large native/provider futures. Heap-own each async
        // composition boundary so adding an adapter cannot exhaust peer/test stacks.
        match Box::pin(Self::start_inner(
            config,
            connectors.messages,
            connectors.sms_policy,
            connectors.message_routes,
            &media,
        ))
        .await
        {
            Ok(mut server) => {
                server.contributed_media = Some(media);
                Ok(server)
            }
            Err(error) => {
                media.drain().await.context(
                    "Canonical startup refused and contributed media cleanup is unconfirmed",
                )?;
                Err(error)
            }
        }
    }
    async fn start_inner(
        config: CanonicalServerConfig,
        drivers: Option<Vec<Arc<dyn connect_connector_api::Connector>>>,
        policy: Option<Arc<dyn crate::SmsPolicy>>,
        message_routes: Vec<crate::canonical_messages::MessageRoute>,
        contributed_media: &crate::connector_media::Host,
    ) -> Result<Self> {
        anyhow::ensure!(
            (drivers.is_none() && message_routes.is_empty()) || config.provider_messages.is_some(),
            "Message injection requires protected hosting configuration"
        );
        config.validate()?;
        let webrtc_host = config
            .webrtc
            .as_ref()
            .map(|profile| profile.host())
            .transpose()?;
        // All key/TLS validation precedes installation recovery and binding.
        let mut tls =
            (*crate::tls::server_config(&config.certificate, &config.private_key)?).clone();
        let mut websocket_tls = tls.clone();
        websocket_tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let websocket_tls = Arc::new(websocket_tls);
        tls.alpn_protocols = vec![RAW_QUIC_ALPN.to_vec()];
        if config.webtransport.is_some() {
            tls.alpn_protocols.push(b"h3".to_vec());
        }
        let secret = read_key(&config.signing_private_key)?;
        let signer = Arc::new(
            match config.signing_algorithm.as_str() {
                "EdDSA" => Sig9421Signer::ed25519(&config.signing_key_id, &secret),
                "ES256" => Sig9421Signer::es256(&config.signing_key_id, &secret),
                _ => unreachable!("validated algorithm"),
            }
            .map_err(|_| anyhow::anyhow!("Invalid canonical signing key"))?,
        );
        let configured_assistants = crate::canonical_assistants::Loaded::load(&config)?;
        let database = config.database.clone();
        let limits = config.limits.clone();
        let record_retention = config.record_retention.clone();
        let reconnect_grace = config.session_reconnect_grace_seconds;
        let (installation, store) = tokio::task::spawn_blocking(move || -> Result<_> {
            let installation = crate::installation::InstallationLock::acquire(&database)?;
            let store = Arc::new(Store::open_with_ownership(
                &database,
                limits,
                installation.proof(),
            )?);
            while store.fence_transition_workers(1000)? > 0 {}
            while store.recover_transition_work(1000)? > 0 {}
            while store.recover_native_credential_refreshes(1000)? > 0 {}
            store.integrity_check()?;
            store.configure_record_retention(&record_retention)?;
            store.configure_session_reconnect_grace(reconnect_grace)?;
            store.reconcile_interrupted_video_connections()?;
            Ok((installation, store))
        })
        .await??;
        // Core owns disposable peer views. Negotiation still requires the
        // current Store capability policy; accepting datagrams grants no route.
        let prepared_assistants = match configured_assistants {
            Some(hosting) => {
                let store = store.clone();
                Some(tokio::task::spawn_blocking(move || hosting.prepare(&store)).await??)
            }
            None => None,
        };
        let mut runtime = match &prepared_assistants {
            Some(hosting) => hosting.runtime.clone(),
            None => crate::canonical_assistants::native_runtime()?,
        };
        if let Some(messages) = &config.provider_messages {
            runtime = messages.runtime(runtime)?;
        }
        let service = Service::with_runtime(store.clone(), runtime);
        service.install_canonical_assurance(config.user_assurance.clone())?;
        service.install_canonical_runtime()?;
        if let Some(host) = &webrtc_host {
            service.install_canonical_webrtc(host.clone())?;
        }
        contributed_media.install(&service)?;
        let record_https = match &config.record_https {
            Some(records) => Some(
                crate::transports::record_https::RecordHttpsListener::bind(
                    records,
                    service.clone(),
                    tls.clone(),
                    &config.certificate,
                )
                .await?,
            ),
            None => None,
        };
        let record_https_address = record_https.as_ref().map(|r| r.address);
        let record_references = record_https.as_ref().map(|r| r.registry.clone());
        let authority = service
            .canonical_peer_authority(config.limits.connections, record_references)
            .await
            .map_err(|_| anyhow::anyhow!("Cannot establish canonical authority"))?;
        let routes =
            rvoip_uctp::v2::reachability::ReachabilityRegistry::new(config.limits.connections)
                .map_err(|_| anyhow::anyhow!("Invalid canonical route capacity"))?;
        service.install_canonical_invitation_routes(routes.clone())?;
        let route_authority = authority.clone();
        let peer = Arc::new(PeerConfig {
            methods: Arc::new(AuthenticationMethods::new(Some(StoreValidator::canonical(
                store.clone(),
            )))),
            verifier: Arc::new(Sig9421Verifier::new(StoreKeyResolver::new(store.clone()))),
            signer,
            authority,
            pending_capacity: config.limits.connections.min(256),
        });
        let profile = CanonicalProfile::new(
            peer,
            CanonicalHandler::new(service.clone()),
            config.limits.connections,
        )
        .map_err(|_| anyhow::anyhow!("Invalid canonical profile"))?
        .with_physical_handlers(CanonicalHandler::physical_factory(service.clone()))
        .with_reachability(
            routes,
            route_authority,
            std::time::Duration::from_secs(60),
            10,
        )
        .map_err(|_| anyhow::anyhow!("Invalid canonical invitation routes"))?;
        // Install on this exact Core before either HTTP or native intake opens.
        // The owner also drains SIP when any later startup step fails.
        if let (Some(telephone), Some(voice)) = (&config.telephone, &config.provider_voice) {
            let mut sip_targets = std::collections::BTreeSet::new();
            for route in &telephone.initial {
                sip_targets.insert(store.revalidate(&route.endpoint_credential_id)?.identity_id);
            }
            for route in &voice.routes {
                let (_, target) = store.provider_session_voice_binding(route)?;
                anyhow::ensure!(
                    !sip_targets.contains(&target),
                    "Ambiguous automatic telephone provider for one identity"
                );
            }
        }
        let provider_voice = config
            .provider_voice
            .as_ref()
            .map(|voice| crate::canonical_provider_voice::Worker::start(voice, &service))
            .transpose()?;
        let provider_messages = match &config.provider_messages {
            Some(messages) => Some(
                crate::canonical_messages::Host::start(
                    messages,
                    service.clone(),
                    drivers,
                    policy,
                    message_routes,
                )
                .await?,
            ),
            None => None,
        };
        let telnyx_callback_address = provider_messages
            .as_ref()
            .and_then(|h| h.callback_address());
        let telephone = match &config.telephone {
            Some(telephone) => Some(
                crate::canonical_telephone::TelephoneTransport::start(telephone, &store, &service)
                    .await?,
            ),
            None => None,
        };
        let mut transport = quinn::TransportConfig::default();
        if config.webtransport.is_some() {
            // HTTP/3 owns control/QPACK streams plus the CONNECT and canonical
            // signaling streams. Bound unread stream credit across both ALPNs.
            transport.max_concurrent_bidi_streams(8u32.into());
            transport.max_concurrent_uni_streams(8u32.into());
            transport.stream_receive_window((64 * 1024u32).into());
            transport.receive_window((512 * 1024u32).into());
        } else {
            transport.max_concurrent_bidi_streams(1u32.into());
            transport.max_concurrent_uni_streams(0u32.into());
        }
        transport.datagram_receive_buffer_size(Some(64 * 1024));
        transport.datagram_send_buffer_size(64 * 1024);
        transport.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
        let endpoint = Arc::new(rvoip_uctp::substrate::make_server_endpoint(
            config.bind,
            Arc::new(tls),
            transport,
        )?);
        let startup = crate::startup::EndpointStartup::new(&endpoint);
        let address = endpoint.local_addr()?;
        let public_assistants = prepared_assistants
            .map(|hosting| hosting.install(&service, address))
            .transpose()?;
        if public_assistants.is_none() {
            service.readiness.unconfigured("public-assistant-host");
        }
        let (sender, receiver) = tokio::sync::mpsc::channel(config.limits.connections);
        let (wt_sender, wt_receiver) = tokio::sync::mpsc::channel(config.limits.connections);
        let webtransport_url = config.webtransport.as_ref().map(|c| c.contact(address));
        let webtransport = match &config.webtransport {
            Some(wt) => Some(
                rvoip_webtransport::canonical::start(
                    wt_receiver,
                    profile.clone(),
                    wt.contact(address),
                    wt.mount_path.clone(),
                    Some(wt.allowed_origins.iter().cloned().collect()),
                )
                .map_err(|_| anyhow::anyhow!("Cannot start canonical WebTransport supervisor"))?,
            ),
            None => {
                drop(wt_receiver);
                None
            }
        };
        let websocket_profile = profile.clone();
        let profile = rvoip_quic::canonical::start(
            receiver,
            profile,
            format!("quic://localhost:{}", address.port()),
        )
        .map_err(|_| anyhow::anyhow!("Cannot start canonical supervisor"))?;
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let accepted = endpoint.clone();
        let peers = profile.clone();
        let wt_peers = webtransport.clone();
        let key_deadline = tokio::time::Instant::now()
            + (config.signing_key_expires_at - chrono::Utc::now())
                .to_std()
                .context("Canonical signing key expired during startup")?;
        let linked_endpoint = endpoint.clone();
        let linked_native = profile.clone();
        let linked_wt = webtransport.clone();
        let stop_other_transports: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            linked_native.begin_shutdown();
            if let Some(wt) = &linked_wt {
                wt.begin_shutdown();
            }
            linked_endpoint.close(0u32.into(), b"canonical linked intake stopped");
        });
        let websocket = match &config.websocket {
            Some(ws) => Some(
                crate::canonical_websocket_listener::Listener::start(
                    ws,
                    websocket_tls,
                    websocket_profile,
                    stop.clone(),
                    key_deadline,
                    &service,
                    stop_other_transports,
                )
                .await?,
            ),
            None => None,
        };
        let websocket_address = websocket.as_ref().map(|ws| ws.address);
        let websocket_url = websocket.as_ref().map(|ws| ws.contact.clone());
        let ws_peers = websocket.as_ref().map(|ws| ws.control());
        let discovery = connect_protocol::canonical_discovery::TransportDiscovery {
            profile: connect_protocol::canonical_http::PROFILE.into(),
            max_control_envelope_bytes: rvoip_uctp::v2::MAX_ENVELOPE_BYTES as u32,
            max_physical_peers: config.limits.connections as u32,
            webtransport: config.webtransport.as_ref().map(|wt| {
                connect_protocol::canonical_discovery::WebTransportEndpoint {
                    url: wt.contact(address),
                    origin_required: true,
                    datagrams:
                        connect_protocol::canonical_discovery::DatagramNegotiation::PerSession,
                }
            }),
            websocket: websocket_url.clone().map(|url| {
                connect_protocol::canonical_discovery::WebSocketEndpoint {
                    url,
                    origin_required: true,
                }
            }),
        };
        let http = match config.http_bind {
            Some(address) => Some(
                crate::HttpListener::bind_canonical(address, service.clone(), discovery).await?,
            ),
            None => None,
        };
        let http_address = http.as_ref().map(|h| h.address);
        let mcp = if let Some(config) = &config.mcp {
            let auth: Arc<dyn connect_mcp::McpAuthenticator> = match &config.binding {
                Some(binding) => Arc::new(connect_mcp::FixedBindingAuth::new(
                    service.clone(),
                    binding.clone(),
                )?),
                None => Arc::new(connect_mcp::LocalAuth::new(service.clone())),
            };
            Some(connect_mcp::RunningMcp::start_local(service.clone(), auth, config.bind).await?)
        } else {
            service.readiness.unconfigured("mcp");
            None
        };
        let mcp_address = mcp.as_ref().map(|mcp| mcp.address);
        let maximum_rtt_ms = Arc::new(AtomicU64::new(0));
        let observed_rtt = maximum_rtt_ms.clone();
        let readiness = service.readiness.clone();
        let intake = crate::worker::spawn(
            readiness,
            "canonical-intake",
            stop.clone(),
            async move {
                struct Intake(
                    Arc<quinn::Endpoint>,
                    ProfileServer,
                    Option<ProfileServer>,
                    Option<ProfileServer>,
                );
                impl Drop for Intake {
                    fn drop(&mut self) {
                        self.1.begin_drain();
                        if let Some(wt) = &self.2 {
                            wt.begin_drain();
                        }
                        if let Some(ws) = &self.3 {
                            ws.begin_shutdown();
                        }
                        self.0.close(0u32.into(), b"canonical intake stopped");
                    }
                }
                let _owned = Intake(
                    accepted.clone(),
                    peers.clone(),
                    wt_peers.clone(),
                    ws_peers.clone(),
                );
                let mut handshakes = JoinSet::new();
                loop {
                    tokio::select! {biased;
                        _=cancelled.cancelled()=>break,
                        _=tokio::time::sleep_until(key_deadline)=>{accepted.close(0u32.into(),b"canonical signing key expired");let _=peers.shutdown().await;if let Some(wt)=&wt_peers {let _=wt.shutdown().await;}anyhow::bail!("Canonical signing key expired");},
                        done=handshakes.join_next(),if !handshakes.is_empty()=>{if done.is_some_and(|r|r.is_err()){anyhow::bail!("Canonical TLS admission task failed");}},
                        incoming=accepted.accept()=>{
                            let Some(incoming)=incoming else{break;};
                            if handshakes.len()>=config.limits.connections{incoming.refuse();continue;}
                            let sender=sender.clone();let wt_sender=wt_sender.clone();let cancelled=cancelled.clone();let observed_rtt=observed_rtt.clone();
                            handshakes.spawn(async move{
                                let connection=tokio::select!{biased;_=cancelled.cancelled()=>return,result=tokio::time::timeout(Duration::from_secs(5),incoming)=>match result{Ok(Ok(c))=>c,_=>return}};
                                struct ObservedPath(quinn::Connection,Arc<AtomicU64>);
                                impl Drop for ObservedPath{fn drop(&mut self){self.1.fetch_max(self.0.stats().path.rtt.as_millis().min(u64::MAX as u128) as u64,Ordering::AcqRel);}}
                                let _path=ObservedPath(connection.clone(),observed_rtt);
                                let alpn = connection.handshake_data().and_then(|value| value.downcast::<quinn::crypto::rustls::HandshakeData>().ok()).and_then(|value| value.protocol.clone());
                                let target = if alpn.as_deref()==Some(RAW_QUIC_ALPN) { &sender } else if alpn.as_deref()==Some(b"h3".as_slice()) { if connection.max_datagram_size().is_none() { connection.close(0u32.into(),b"WebTransport requires QUIC datagrams"); return; } &wt_sender } else { connection.close(0u32.into(),b"unsupported canonical transport"); return; };
                                if let Err(error)=target.try_send(connection.clone()){error.into_inner().close(0u32.into(),b"canonical admission full");return;}
                                // Retain this physical slot through the entire peer lifetime.
                                tokio::select!{biased;_=cancelled.cancelled()=>{connection.close(0u32.into(),b"canonical admission stopped");},_=connection.closed()=>{}}
                            });
                        },
                    }
                }
                handshakes.abort_all();
                while handshakes.join_next().await.is_some() {}
                Ok(())
            },
        );
        let idle = Some(crate::conversation_policy::Worker::start(service.clone()));
        let records = Some(crate::record_outcomes::Worker::start(service.clone()));
        let server = Self {
            installation,
            endpoint,
            profile,
            webtransport,
            websocket,
            websocket_address,
            websocket_url,
            webtransport_url,
            maximum_rtt_ms,
            stop,
            intake: Some(intake),
            http,
            mcp,
            record_https,
            idle,
            records,
            telephone,
            public_assistants,
            provider_voice,
            provider_messages,
            #[cfg(any(test, feature = "test-support"))]
            webrtc_host,
            contributed_media: None,
            telnyx_callback_address,
            address,
            http_address,
            mcp_address,
            record_https_address,
            store,
            service,
        };
        startup.finish();
        Ok(server)
    }
    /// Protected runtime inspection. No configuration or provider key is exposed.
    pub fn public_assistant_host(
        &self,
    ) -> Option<&Arc<crate::assistant_vapi::PublicVapiCoordinator>> {
        self.public_assistants.as_ref().map(|h| &h.0)
    }
    pub async fn shutdown(mut self) -> Result<()> {
        self.stop.cancel();
        if let Some(http) = &self.http {
            http.begin_shutdown()
        }
        if let Some(records) = &self.record_https {
            records.begin_shutdown()
        }
        if let Some(ws) = &self.websocket {
            ws.begin_shutdown();
        }
        self.profile.begin_drain();
        if let Some(wt) = &self.webtransport {
            wt.begin_drain();
        }
        let mut failures = Vec::new();
        if let Some(media) = &self.contributed_media {
            media.cancel();
            if media.drain().await.is_err() {
                failures.push("contributed media");
            }
        }
        if let Some(mcp) = self.mcp.take() {
            if mcp.shutdown().await.is_err() {
                failures.push("MCP");
            }
        }
        if let Some(messages) = self.provider_messages.take() {
            if messages.shutdown().await.is_err() {
                failures.push("provider messages");
            }
        }

        let public_session_failure = self.service.drain_session_initialization().await.err();
        if public_session_failure.is_some() {
            failures.push("public Session jobs");
        }
        if self
            .service
            .drain_consultation_initialization()
            .await
            .is_err()
        {
            failures.push("private consultation jobs");
        }
        self.endpoint
            .close(0u32.into(), b"canonical server shutdown");
        if self.profile.shutdown().await.is_err() {
            failures.push("canonical peers");
        }
        if let Some(wt) = &self.webtransport {
            if wt.shutdown().await.is_err() {
                failures.push("WebTransport peers");
            }
        }
        if let Some(ws) = self.websocket.take() {
            if ws.shutdown().await.is_err() {
                failures.push("WebSocket peers/intake");
            }
        }
        if let Some(voice) = self.provider_voice.take() {
            if voice.shutdown().await.is_err() {
                failures.push("provider telephone");
            }
        }
        self.service.drain_canonical_runtime().await;
        if let Some(mut telephone) = self.telephone.take() {
            if telephone.shutdown().await.is_err() {
                failures.push("telephone");
            }
        }
        if crate::worker::drain(
            self.intake.take().into_iter().collect(),
            Duration::from_secs(5),
        )
        .await
        .is_err()
        {
            failures.push("canonical intake");
        }
        if let Some(records) = self.record_https.take() {
            if records.shutdown().await.is_err() {
                failures.push("record HTTPS");
            }
        }
        if let Some(http) = self.http.take() {
            if http.shutdown().await.is_err() {
                failures.push("HTTP");
            }
        }
        if let Some(idle) = self.idle.take() {
            if idle.shutdown().await.is_err() {
                failures.push("conversation policy");
            }
        }
        if let Some(records) = self.records.take() {
            if records.shutdown().await.is_err() {
                failures.push("Session records");
            }
        }
        // Current installation authority covers the actual owned drains. Stop
        // the policy worker before retirement so an in-flight check cannot
        // mistake requested shutdown for a runtime ownership failure.
        self.installation.retire();
        // Quinn retains closed paths for 3 PTOs. Its RTT estimator maintains
        // variance <= 2 * smoothed RTT, so the RTT part of that interval is
        // bounded by 3 * (RTT + 4 * 2 * RTT). Include local profile ACK delay,
        // millisecond rounding and scheduling margin. This is separate from
        // worker drain and remains capped even for an unresponsive peer.
        let maximum_rtt_ms = self.maximum_rtt_ms.load(Ordering::Acquire);
        let protocol_drain_budget =
            Duration::from_millis(maximum_rtt_ms.saturating_mul(27).saturating_add(250))
                .clamp(Duration::from_secs(5), Duration::from_secs(30));
        let protocol_drain_started = tokio::time::Instant::now();
        if tokio::time::timeout(protocol_drain_budget, self.endpoint.wait_idle())
            .await
            .is_err()
        {
            anyhow::bail!("Canonical protocol drain timed out after owned workers stopped: open={}, maximum observed RTT={}ms, protocol allowance={}ms; other failures={}",self.endpoint.open_connections(),maximum_rtt_ms,protocol_drain_budget.as_millis(),failures.join(", "));
        }
        tracing::debug!(
            elapsed_ms = protocol_drain_started.elapsed().as_millis(),
            allowance_ms = protocol_drain_budget.as_millis(),
            "Canonical protocol drain complete"
        );
        anyhow::ensure!(
            failures.is_empty(),
            "Canonical shutdown failed: {}; public Session failure: {:?}",
            failures.join(", "),
            public_session_failure
        );
        Ok(())
    }
}
impl Drop for RunningCanonicalServer {
    fn drop(&mut self) {
        if let Some(media) = &self.contributed_media {
            media.cancel();
        }
        self.service.cancel_session_initialization();
        self.service.cancel_consultation_initialization();
        self.installation.retire();
        self.stop.cancel();
        if let Some(ws) = &self.websocket {
            ws.begin_shutdown();
        }
        self.profile.begin_drain();
        if let Some(wt) = &self.webtransport {
            wt.begin_drain();
        }
        self.endpoint
            .close(0u32.into(), b"canonical server dropped");
        if let Some(task) = &self.intake {
            task.abort();
        }
    }
}
fn read_key(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).context("Read canonical signing key")?;
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        !bytes.is_empty() && bytes.len() <= 16_384,
        "Invalid canonical signing key size"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn configuration_errors_do_not_echo_private_file_contents() {
        let temp = tempfile::tempdir().expect("private config directory");
        let path = temp.path().join("server.toml");
        let canary = "private-config-canary-never-print";
        std::fs::write(&path, format!("credential = {canary:?}")).expect("write malformed config");
        let Err(error) = super::CanonicalServerConfig::load(&path) else {
            panic!("invalid configuration accepted")
        };
        assert!(!format!("{error:#}").contains(canary));
    }
}

pub use crate::transports::record_https::RecordHttpsConfig;
