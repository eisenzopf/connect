//! Disposable Core views use durable Session IDs and the dispatcher's real peer.
use crate::StoreInvocationAuthority;
use connect_protocol::{Error, Result};
use connect_store::PeerSessionProjection;
use rvoip_core::peer_admission::PeerAdmission;
use rvoip_core::{
    ids::{ConversationId, IdentityId, ParticipantId, SessionId, TenantId},
    orchestrator::peer_sessions::{PeerSessionAuthority, PeerSessionAuthorityLease},
    participant::{ParticipantKind, ParticipantRole},
    relationships::{IdentityBinding, Membership},
    session::SessionMedium,
    AuthenticatedPrincipal, Orchestrator, Transport,
};
use rvoip_uctp::v2::{
    auth_dispatch::AuthorizedEnvelope,
    native::{CanonicalNativeAdapter, NativePhysicalBinding},
    server::NativeTransportContext,
};
use std::sync::{Arc, Mutex, Weak};

/// Process-local counts. These observations confer no resource authority.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CanonicalRuntimeSnapshot {
    pub physical_peers: usize,
    pub peer_session_views: usize,
    pub live_sessions: usize,
    pub connections: usize,
    pub media_readers: usize,
    pub media_datagrams_accepted: u64,
    pub media_datagrams_rejected: u64,
    pub role_projections: usize,
    pub replacement_preparations: usize,
}
pub(crate) struct Runtime {
    pub(crate) routes: std::sync::OnceLock<Arc<rvoip_uctp::v2::reachability::ReachabilityRegistry>>,
    pub(crate) invitation_jobs: Mutex<std::collections::BTreeSet<String>>,
    pub(crate) session_reconnect_worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) session_lifecycle_worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) session_codec_worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    pub(crate) codec_origins: Mutex<
        std::collections::BTreeMap<
            String,
            Arc<rvoip_uctp::v2::native_requests::NativeRequestOrigin>,
        >,
    >,
    pub(crate) invitation_callers: Mutex<crate::canonical_invitations::Callers>,
    pub(crate) host_configuration_closed: Mutex<bool>,
    physical: Mutex<Vec<Weak<Physical>>>,
    capacity: usize,
    pub(crate) work: Mutex<tokio::task::JoinSet<()>>,
    pub(crate) credential_refresh_receipts: Mutex<
        std::collections::BTreeMap<
            String,
            Option<Arc<connect_store::NativeCredentialRefreshApplied>>,
        >,
    >,
    pub(crate) stop: tokio_util::sync::CancellationToken,
    _retirement: Arc<dyn connect_store::RuntimeRetirementSignal>,
    pub(crate) core: Arc<Orchestrator>,
    // Retain both registered instances. Media negotiation selects the actual
    // physical/Core transport and never a detached test/channel adapter.
    quic_native: Arc<CanonicalNativeAdapter>,
    pub(crate) webtransport_native: Arc<CanonicalNativeAdapter>,
    pub(crate) capability_listeners: Mutex<
        std::collections::BTreeMap<String, Weak<crate::canonical_capability_delivery::Listener>>,
    >,
    pub(crate) dtmf_listeners:
        Mutex<std::collections::BTreeMap<String, Weak<crate::canonical_native_dtmf::Listener>>>,
    pub(crate) telephone: Mutex<Option<Arc<crate::telephone_replacement::TelephoneRuntime>>>,
    pub(crate) initial_telephone_host: std::sync::OnceLock<Weak<crate::initial_telephone::Host>>,
    pub(crate) media: Arc<crate::canonical_media::Policy>,
    pub(crate) role_projection: tokio::sync::Mutex<()>,
    pub(crate) role_projections: std::sync::atomic::AtomicUsize,
    pub(crate) consultation_returns: Mutex<
        std::collections::BTreeMap<String, Arc<connect_store::NativeConsultationReturnApplied>>,
    >,
    pub(crate) handoffs:
        Mutex<std::collections::BTreeMap<String, crate::canonical_handoff::AppliedHandoff>>,
    pub(crate) replacements: Mutex<
        std::collections::BTreeMap<String, Arc<crate::canonical_replacement::PreparedReplacement>>,
    >,
    pub(crate) consultation_jobs: Mutex<std::collections::BTreeSet<String>>,
    pub(crate) consultation_initializer:
        std::sync::OnceLock<Arc<dyn crate::ConsultationInitializer>>,
    pub(crate) session_initializers: Mutex<crate::session_init::Registry>,
    pub(crate) consultation_audiences:
        Mutex<std::collections::BTreeMap<String, crate::canonical_consultation::AppliedAudience>>,
}
struct Retirement(tokio_util::sync::CancellationToken);
impl connect_store::RuntimeRetirementSignal for Retirement {
    fn retire(&self) {
        self.0.cancel();
    }
}
impl Runtime {
    /// Select only a registered canonical transport. The caller supplies sealed
    /// physical context or Core ownership, never a transport from a wire payload.
    pub(crate) fn native_adapter(
        &self,
        transport: Transport,
    ) -> Result<Arc<CanonicalNativeAdapter>> {
        match transport {
            Transport::Quic => Ok(self.quic_native.clone()),
            Transport::WebTransport => Ok(self.webtransport_native.clone()),
            _ => Err(Error::forbidden()),
        }
    }
    pub(crate) fn connection_native_adapter(
        &self,
        id: &rvoip_core::ids::ConnectionId,
    ) -> Result<Arc<CanonicalNativeAdapter>> {
        let transport = self
            .core
            .connection_transport(id)
            .map_err(|_| Error::forbidden())?;
        self.native_adapter(transport)
    }
    pub(crate) fn native_adapters(&self) -> [Arc<CanonicalNativeAdapter>; 2] {
        [self.quic_native.clone(), self.webtransport_native.clone()]
    }
    pub(crate) fn observe_native_audio_audience_commit(
        &self,
        id: &rvoip_core::ids::ConnectionId,
        commit: &rvoip_core::staged_streams::audience::AudioAudienceCommit,
    ) -> Result<()> {
        self.connection_native_adapter(id)?
            .observe_audio_audience_commit(id, commit)
            .map_err(|_| Error::forbidden())
    }
    pub(crate) fn native_connection_live(&self, id: &rvoip_core::ids::ConnectionId) -> bool {
        use rvoip_core::adapter::ConnectionAdapter;
        self.quic_native.is_connection_live(id) || self.webtransport_native.is_connection_live(id)
    }
    pub(crate) fn new(store: Arc<connect_store::Store>) -> Result<Arc<Self>> {
        let limits = store.limits();
        if limits.participants > 256 || limits.connections > 512 {
            return Err(Error::new(
                "unsupported-media-profile",
                "Canonical media capacity exceeds the supported engine profile",
            ));
        }
        let core = Orchestrator::new(rvoip_core::Config {
            vcon_store: Arc::new(crate::canonical_records::Records::new(store.clone())),
            max_concurrent_setups: limits.connections,
            max_direct_subscribers: limits.connections,
            ..Default::default()
        });
        let native = CanonicalNativeAdapter::new(Transport::Quic).map_err(|_| Error::storage())?;
        core.register(native.clone())
            .map_err(|_| Error::storage())?;
        let webtransport_native =
            CanonicalNativeAdapter::new(Transport::WebTransport).map_err(|_| Error::storage())?;
        core.register(webtransport_native.clone())
            .map_err(|_| Error::storage())?;
        store.recover_native_offer_cleanup(&crate::canonical_native::Recovery([
            native.clone(),
            webtransport_native.clone(),
        ]))?;
        for _ in 0..limits.connections.div_ceil(256) {
            if store.recover_native_streams(256)? < 256 {
                break;
            }
        }
        loop {
            if store.recover_native_data_writes(256)? < 256 {
                break;
            }
        }
        loop {
            if store.recover_native_dtmf(256)? < 256 {
                break;
            }
        }
        loop {
            if store.recover_received_dtmf(256)? < 256 {
                break;
            }
        }
        let stop = tokio_util::sync::CancellationToken::new();
        let retirement: Arc<dyn connect_store::RuntimeRetirementSignal> =
            Arc::new(Retirement(stop.clone()));
        store.register_runtime_retirement(&retirement)?;
        Ok(Arc::new(Self {
            routes: Default::default(),
            invitation_jobs: Mutex::new(Default::default()),
            session_lifecycle_worker: Mutex::new(None),
            session_codec_worker: Mutex::new(None),
            codec_origins: Mutex::new(Default::default()),
            session_reconnect_worker: Mutex::new(None),
            invitation_callers: Mutex::new(Default::default()),
            host_configuration_closed: Mutex::new(false),
            work: Mutex::new(tokio::task::JoinSet::new()),
            credential_refresh_receipts: Mutex::new(Default::default()),
            stop,
            _retirement: retirement,
            core,
            quic_native: native,
            webtransport_native,
            capability_listeners: Mutex::new(Default::default()),
            dtmf_listeners: Mutex::new(Default::default()),
            telephone: Mutex::new(None),
            initial_telephone_host: std::sync::OnceLock::new(),
            media: crate::canonical_media::Policy::new(store.clone(), limits.connections),
            physical: Mutex::new(vec![]),
            capacity: limits.connections,
            role_projection: tokio::sync::Mutex::new(()),
            role_projections: std::sync::atomic::AtomicUsize::new(0),
            consultation_returns: Mutex::new(Default::default()),
            handoffs: Mutex::new(Default::default()),
            replacements: Mutex::new(Default::default()),
            consultation_jobs: Mutex::new(Default::default()),
            consultation_initializer: std::sync::OnceLock::new(),
            session_initializers: Mutex::new(Default::default()),
            consultation_audiences: Mutex::new(Default::default()),
        }))
    }
}
pub(crate) struct Physical {
    pub(crate) lifecycle: rvoip_uctp::v2::native_lifecycle::NativeLifecycleMux,
    pub(crate) capabilities: Arc<crate::canonical_capability_delivery::Queue>,
    pub(crate) dtmf: rvoip_uctp::v2::native_dtmf_egress::NativeDtmfMux,
    pub(crate) dtmf_listeners:
        Mutex<std::collections::BTreeMap<String, Arc<crate::canonical_native_dtmf::Listener>>>,
    pub(crate) lifetime: tokio_util::sync::CancellationToken,
    peer: Mutex<Option<Weak<PeerAdmission>>>,
    pub(crate) binding: Arc<NativePhysicalBinding>,
}
impl Physical {
    pub(crate) fn new(context: NativeTransportContext) -> Arc<Self> {
        Arc::new(Self {
            lifetime: context.lifetime.clone(),
            binding: NativePhysicalBinding::new(context),
            peer: Mutex::new(None),
            lifecycle: Default::default(),
            capabilities: Default::default(),
            dtmf: Default::default(),
            dtmf_listeners: Mutex::new(Default::default()),
        })
    }
    pub(crate) fn validate(
        &self,
        a: &AuthorizedEnvelope,
        authority: &StoreInvocationAuthority,
    ) -> Result<()> {
        let proof = authority
            .peer_guard()
            .filter(|p| p.current())
            .ok_or_else(Error::forbidden)?;
        let peer = a.peer_admission().ok_or_else(Error::forbidden)?;
        peer.validate_binding(
            a.principal(),
            &IdentityId::from_string(proof.key.identity_id.clone()),
            &rvoip_core::ids::DeviceId::from_string(proof.key.device_id.clone()),
        )
        .map_err(|_| Error::forbidden())?;
        let mut bound = self.peer.lock().map_err(|_| Error::storage())?;
        if bound
            .as_ref()
            .is_some_and(|previous| !Weak::ptr_eq(previous, &Arc::downgrade(peer)))
        {
            return Err(Error::forbidden());
        }
        *bound = Some(Arc::downgrade(peer));
        if !matches!(
            self.binding.transport(),
            Transport::Quic | Transport::WebTransport | Transport::WebSocket
        ) {
            return Err(Error::forbidden());
        }
        // This admits the authenticated physical control transport only. Media
        // still requires its own registered adapter, current Endpoint and route.
        Ok(())
    }
}
struct Lease {
    value: PeerSessionAuthority,
    // The originating Store writer remains owned during Core's synchronous
    // calls. A facts-only Membership cannot manufacture another policy read.
    _held: StoreInvocationAuthority,
}
impl PeerSessionAuthorityLease for Lease {
    fn authority(&self) -> &PeerSessionAuthority {
        &self.value
    }
}
impl Runtime {
    pub(crate) fn physical(&self, context: NativeTransportContext) -> Result<Arc<Physical>> {
        let mut closed = self
            .host_configuration_closed
            .lock()
            .map_err(|_| Error::storage())?;
        *closed = true;
        let mut physical = self.physical.lock().map_err(|_| Error::storage())?;
        physical.retain(|p| p.strong_count() > 0);
        if physical.len() >= self.capacity {
            return Err(Error::new(
                "capacity-exceeded",
                "Canonical physical capacity exhausted",
            ));
        }
        let owner = Physical::new(context);
        physical.push(Arc::downgrade(&owner));
        Ok(owner)
    }
    pub(crate) fn physical_for_peer(
        &self,
        peer: &Arc<PeerAdmission>,
    ) -> Result<Option<Arc<Physical>>> {
        let peers = self.physical.lock().map_err(|_| Error::storage())?;
        let mut matches = Vec::new();
        for p in peers.iter().filter_map(Weak::upgrade) {
            if p.peer
                .lock()
                .map_err(|_| Error::storage())?
                .as_ref()
                .is_some_and(|old| Weak::ptr_eq(old, &Arc::downgrade(peer)))
            {
                matches.push(p);
            }
        }
        if matches.len() > 1 {
            return Err(Error::forbidden());
        }
        Ok(matches.pop())
    }
    pub(crate) fn snapshot(&self) -> CanonicalRuntimeSnapshot {
        let media = self
            .physical
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
            .filter_map(|p| p.binding.media())
            .map(|m| m.snapshot())
            .collect::<Vec<_>>();
        let peers = self
            .physical
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
            .filter_map(|p| {
                let guard = p.peer.lock().ok()?;
                guard.as_ref()?.upgrade()
            })
            .map(|p| p.snapshot())
            .filter(|p| !p.closed)
            .collect::<Vec<_>>();
        CanonicalRuntimeSnapshot {
            replacement_preparations: self
                .replacements
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .values()
                .filter(|p| !p.handle.is_committed() && !p.handle.cancellation().is_cancelled())
                .count(),
            role_projections: self
                .role_projections
                .load(std::sync::atomic::Ordering::Relaxed),
            media_readers: media.iter().filter(|m| m.reader_running).count(),
            media_datagrams_accepted: media
                .iter()
                .fold(0u64, |n, m| n.saturating_add(m.ingress_accepted)),
            media_datagrams_rejected: media
                .iter()
                .fold(0u64, |n, m| n.saturating_add(m.ingress_rejected)),
            physical_peers: peers.len(),
            peer_session_views: peers.iter().map(|p| p.sessions).sum(),
            connections: peers.iter().map(|p| p.connections).sum(),
            live_sessions: self.core.externally_projected_session_ids().len(),
        }
    }
    /// The caller retains the existing current command fence throughout this
    /// synchronous projection. Empty checked containers claim no physical peer,
    /// Participant presence, Connection, negotiated codec or media readiness.
    pub(crate) fn project_session_container(
        &self,
        projection: &PeerSessionProjection,
        tenant: &str,
    ) -> Result<()> {
        if self.stop.is_cancelled()
            || crate::canonical_sessions::terminal(&projection.session.state)
            || !projection.present
        {
            return Err(Error::forbidden());
        }
        let sid = SessionId::from_string(projection.session.id.clone());
        // This is a heterogeneous runtime container, not negotiated media or
        // Participant admission. Each future Connection must pass current
        // durable capability policy before it can publish/receive anything.
        self.core
            .project_external_session(
                ConversationId::from_string(projection.session.conversation_id.clone()),
                sid.clone(),
                TenantId::from_string(tenant.to_owned()),
                SessionMedium::Mixed,
                [],
            )
            .map_err(|_| Error::storage())?;
        // A retry may find the disposable Session after its first coordinator
        // allocation failed. Always require the checked managed coordinator;
        // a failed probe never permits attachment to an unguarded media graph.
        if self.core.participant_revision(&sid).is_err() {
            self.core
                .enable_staged_participant_control(
                    &sid,
                    self.media.clone(),
                    self.media.clone(),
                    rvoip_core::orchestrator::participant_control::ParticipantLimits {
                        participants: self.media.store.limits().participants,
                        connections: self.capacity,
                        // Preserve the approved 48 kHz Opus profile through
                        // mixing. Telephone codecs resample at their boundary;
                        // inheriting the 8 kHz default loses wideband content.
                        mix_rate_hz: 48_000,
                        ..Default::default()
                    },
                )
                .map_err(|_| Error::storage())?;
        }
        Ok(())
    }
    pub(crate) fn attach(
        &self,
        projection: &PeerSessionProjection,
        principal: &AuthenticatedPrincipal,
        peer: Arc<rvoip_core::peer_admission::PeerAdmission>,
        held: StoreInvocationAuthority,
    ) -> Result<()> {
        let sid = SessionId::from_string(projection.session.id.clone());
        if crate::canonical_sessions::terminal(&projection.session.state) || !projection.present {
            self.core.release_peer_session_view(&peer, &sid);
            return Ok(());
        }
        let member = current_member(principal, projection)?;
        let lease = Lease {
            value: PeerSessionAuthority {
                actor: member,
                invitees: vec![],
                may_create: false,
                may_attach: true,
            },
            _held: held,
        };
        self.project_session_container(
            projection,
            principal.tenant.as_deref().ok_or_else(Error::forbidden)?,
        )?;
        self.core
            .attach_peer_session(peer, principal, &sid, &lease)
            .map_err(|_| Error::forbidden())
    }
}

pub(crate) fn current_member(
    principal: &AuthenticatedPrincipal,
    projection: &PeerSessionProjection,
) -> Result<Membership> {
    let binding = IdentityBinding::from_authenticated(
        principal,
        IdentityId::from_string(projection.actor.identity_id.clone()),
        chrono::Utc::now(),
    )
    .map_err(|_| Error::forbidden())?;
    Membership::from_current(
        binding,
        ConversationId::from_string(projection.actor.conversation_id.clone()),
        ParticipantId::from_string(projection.actor.participant_id.clone()),
        match projection.actor.kind {
            connect_protocol::provisioning::IdentityKind::Human => ParticipantKind::Human,
            connect_protocol::provisioning::IdentityKind::Ai => ParticipantKind::Ai,
            connect_protocol::provisioning::IdentityKind::System => ParticipantKind::System,
            connect_protocol::provisioning::IdentityKind::External => ParticipantKind::External,
        },
        participant_role(&projection.actor.role),
        projection.actor.membership_epoch,
        projection.conversation_revision,
        projection.actor.active,
    )
    .map_err(|_| Error::forbidden())
}

/// The public observer role is receive-only in the native coordinator.
pub(crate) fn participant_role(value: &str) -> ParticipantRole {
    if value == "observer" {
        ParticipantRole::Observer
    } else {
        ParticipantRole::Custom(value.into())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Ok(worker) = self.session_codec_worker.get_mut() {
            if let Some(worker) = worker.take() {
                self.media.store.uninstall_session_codec_coordinator();
                worker.abort();
            }
        }
        if let Ok(worker) = self.session_reconnect_worker.get_mut() {
            if let Some(worker) = worker.take() {
                worker.abort();
            }
        }
        if let Ok(worker) = self.session_lifecycle_worker.get_mut() {
            if let Some(worker) = worker.take() {
                worker.abort();
            }
        }
    }
}

pub(crate) fn endpoint_transport(
    transport: Transport,
) -> Result<connect_protocol::provisioning::EndpointTransport> {
    match transport {
        Transport::Quic => Ok(connect_protocol::provisioning::EndpointTransport::Quic),
        Transport::WebTransport => {
            Ok(connect_protocol::provisioning::EndpointTransport::Webtransport)
        }
        _ => Err(Error::forbidden()),
    }
}
pub(crate) fn core_transport(
    transport: &connect_protocol::provisioning::EndpointTransport,
) -> Result<Transport> {
    match transport {
        connect_protocol::provisioning::EndpointTransport::Quic => Ok(Transport::Quic),
        connect_protocol::provisioning::EndpointTransport::Webtransport => {
            Ok(Transport::WebTransport)
        }
        _ => Err(Error::forbidden()),
    }
}
