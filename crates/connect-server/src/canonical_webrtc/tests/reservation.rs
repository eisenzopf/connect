//! Transport fixtures also require an actual typed Core reservation. They do
//! not stage media or claim Store/Core participant admission.
use super::*;
use rvoip_core::{
    adapter::{ConnectionAdapter, EndReason},
    identity::{AuthenticatedPrincipal, AuthenticationMethod, CredentialKind, IdentityAssurance},
    ids::*,
    orchestrator::{peer_native::*, peer_sessions::*},
    participant::{ParticipantKind, ParticipantRole},
    peer_admission::PeerAdmissionOwner,
    relationships::*,
    session::SessionMedium,
    Orchestrator,
};
use rvoip_webrtc::canonical::CanonicalWebRtcAdapter;
use tokio::sync::OwnedSemaphorePermit;
struct Authority {
    native: PeerNativeAuthority,
    session: PeerSessionAuthority,
}
impl PeerNativeAuthorityLease for Authority {
    fn authority(&self) -> &PeerNativeAuthority {
        &self.native
    }
}
impl PeerSessionAuthorityLease for Authority {
    fn authority(&self) -> &PeerSessionAuthority {
        &self.session
    }
}
pub(in crate::canonical_webrtc) struct Reservation {
    adapter: Arc<CanonicalWebRtcAdapter>,
    _core: Arc<Orchestrator>,
    _peer: PeerAdmissionOwner,
    id: ConnectionId,
}
impl Reservation {
    fn new(
        physical: &CancellationToken,
        permit: OwnedSemaphorePermit,
        id: ConnectionId,
    ) -> (Self, Arc<WebRtcPhysicalBinding>) {
        let principal = AuthenticatedPrincipal {
            subject: "fixture".into(),
            tenant: Some("fixture-tenant".into()),
            issuer: Some("fixture-issuer".into()),
            scopes: vec![],
            expires_at: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Identified {
                credential_kind: CredentialKind::Oidc,
            },
        };
        let identity = IdentityId::new();
        let device = DeviceId::new();
        let peer = PeerAdmissionOwner::new(
            &principal,
            identity.clone(),
            device.clone(),
            physical.child_token(),
            Default::default(),
        )
        .unwrap();
        let core = Orchestrator::new(Default::default());
        let adapter = CanonicalWebRtcAdapter::new();
        core.register(adapter.clone()).unwrap();
        let conversation = ConversationId::new();
        let session = SessionId::new();
        core.project_external_session(
            conversation.clone(),
            session.clone(),
            TenantId::from_string("fixture-tenant"),
            SessionMedium::Voice,
            [],
        )
        .unwrap();
        let identity =
            IdentityBinding::from_authenticated(&principal, identity, chrono::Utc::now()).unwrap();
        let member = Membership::admit(
            identity.clone(),
            conversation,
            ParticipantId::new(),
            ParticipantKind::Human,
            ParticipantRole::Agent,
        )
        .unwrap();
        let mut endpoint = EndpointBinding::provision(
            identity.clone(),
            EndpointId::new(),
            device,
            rvoip_core::Transport::WebRtc,
            "fixture".into(),
        )
        .unwrap();
        endpoint
            .verify(
                &identity,
                "isolated-fixture-authority".into(),
                chrono::Utc::now(),
            )
            .unwrap();
        let authority = Authority {
            native: PeerNativeAuthority {
                actor: member.clone(),
                endpoint,
                may_prepare: true,
            },
            session: PeerSessionAuthority {
                actor: member,
                invitees: vec![],
                may_create: false,
                may_attach: true,
            },
        };
        core.attach_peer_session(peer.peer(), &principal, &session, &authority)
            .unwrap();
        let binding =
            WebRtcPhysicalBinding::with_capacity(&peer.peer(), physical.child_token(), permit);
        core.prepare_peer_webrtc_connection(
            peer.peer(),
            &principal,
            PeerWebRtcIntent {
                session,
                connection: id.clone(),
            },
            &authority,
            binding.clone(),
        )
        .unwrap()
        .retain(&principal, &authority)
        .unwrap();
        assert_eq!(peer.peer().snapshot().connections, 1);
        assert_eq!(adapter.total_allocations(), 1);
        (
            Self {
                adapter,
                _core: core,
                _peer: peer,
                id,
            },
            binding,
        )
    }
    pub(in crate::canonical_webrtc) async fn close(self) -> Result<()> {
        self.adapter
            .end(self.id.clone(), EndReason::Normal)
            .await
            .map_err(|_| engine_error())?;
        if self.adapter.route_count() != 0 {
            return Err(engine_error());
        }
        Ok(())
    }
}
impl Peer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        store: Arc<Store>,
        origin: Arc<NativeMediaOrigin>,
        attempt: NativeOfferLease,
        config: WebRtcConfig,
        physical: &CancellationToken,
        permit: OwnedSemaphorePermit,
        deadline: Instant,
    ) -> Self {
        Self::start_fixture_scoped(
            store,
            origin,
            attempt,
            config,
            physical,
            permit,
            deadline,
            ReadContext::new(None),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_fixture_scoped(
        store: Arc<Store>,
        origin: Arc<NativeMediaOrigin>,
        attempt: NativeOfferLease,
        config: WebRtcConfig,
        physical: &CancellationToken,
        permit: OwnedSemaphorePermit,
        deadline: Instant,
        initial: InitialRead,
    ) -> Self {
        let (reservation, binding) = Reservation::new(
            physical,
            permit,
            ConnectionId::from_string(origin.runtime_connection_id().to_owned()),
        );
        Self::start_scoped(
            store,
            origin,
            attempt,
            config,
            physical,
            binding,
            deadline,
            initial,
            Some(reservation),
        )
    }
}
