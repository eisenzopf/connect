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

async fn original_core_retired(reservation: &Reservation) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while reservation
            ._core
            .connection_transport(&reservation.id)
            .is_ok()
            || reservation._peer.peer().snapshot().connections != 0
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the original terminal event retires Core's original relationship");
    assert_eq!(reservation._peer.peer().snapshot().connections, 0);
}

async fn original_fixture_joined(reservation: &Reservation) {
    original_core_retired(reservation).await;
    reservation
        ._core
        .drain_prepared_outbound_connections()
        .await;
    reservation
        ._core
        .drain_connection_lifecycle_tasks()
        .await
        .unwrap();
    assert_eq!(reservation._core.connection_lifecycle_task_count(), 0);
    assert_eq!(reservation.adapter.route_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_survives_retired_core_lookup_race() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (reservation, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let original_rtc = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let (checked, checked_rx) = tokio::sync::oneshot::channel();
    let (resume, resume_rx) = tokio::sync::oneshot::channel();
    let core = reservation._core.clone();
    let id = reservation.id.clone();
    // Deterministically execute the old service's real two-lookup sequence.
    // The exact original terminal event is delivered between its two reads.
    let old_cleanup = tokio::spawn(async move {
        assert!(core.connection_transport(&id).is_ok());
        checked.send(()).unwrap();
        resume_rx.await.unwrap();
        core.end_connection(id, EndReason::Normal).await
    });
    tokio::time::timeout(Duration::from_secs(3), checked_rx)
        .await
        .unwrap()
        .unwrap();
    reservation
        .adapter
        .end(reservation.id.clone(), EndReason::Normal)
        .await
        .unwrap();
    original_core_retired(&reservation).await;
    resume.send(()).unwrap();
    let before = tokio::time::timeout(Duration::from_secs(3), old_cleanup)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(before, Err(rvoip_core::error::RvoipError::ConnectionNotFound(ref id))
        if id == &reservation.id),
        "old cleanup must expose the actual typed lookup race"
    );
    // The fixed production capability joins the same original owner after
    // Core has retired, without granting authority to a new route or attempt.
    reservation
        .adapter
        .end_original_binding(reservation.id.clone(), &binding, EndReason::Normal)
        .await
        .unwrap();
    binding.shutdown().await.unwrap();
    original_rtc.close().await.unwrap();
    assert_eq!(reservation.adapter.total_allocations(), 1);
    assert_eq!(slots.available_permits(), 1);
    original_fixture_joined(&reservation).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_rejects_other_binding_at_same_connection_id() {
    let id = ConnectionId::new();
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(2));
    let (old, old_binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        id.clone(),
    );
    let (current, current_binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        id.clone(),
    );
    old_binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    current_binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let old_rtc = old_binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let current_rtc = current_binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    old.adapter
        .end_original_binding(id.clone(), &old_binding, EndReason::Normal)
        .await
        .unwrap();
    original_core_retired(&old).await;
    assert!(matches!(
        current
            .adapter
            .end_original_binding(id.clone(), &old_binding, EndReason::Normal)
            .await,
        Err(rvoip_core::error::RvoipError::AdmissionRejected(
            "Original WebRTC cleanup binding differs"
        ))
    ));
    assert_eq!(current.adapter.route_count(), 1);
    assert!(current._core.connection_transport(&id).is_ok());
    let still_original = current_binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&still_original, &current_rtc));
    assert_eq!(
        slots.available_permits(),
        1,
        "the current original slot stays owned"
    );
    current
        .adapter
        .end_original_binding(id, &current_binding, EndReason::Normal)
        .await
        .unwrap();
    old_rtc.close().await.unwrap();
    current_rtc.close().await.unwrap();
    original_fixture_joined(&old).await;
    original_fixture_joined(&current).await;
    assert_eq!(slots.available_permits(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_joins_partial_start_without_creating_rtc() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (reservation, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    assert!(matches!(
        binding.peer(Instant::now() + Duration::from_secs(1)).await,
        Err(rvoip_core::error::RvoipError::InvalidState(
            "WebRTC allocation not started"
        ))
    ));
    reservation
        .adapter
        .end_original_binding(reservation.id.clone(), &binding, EndReason::Normal)
        .await
        .unwrap();
    original_fixture_joined(&reservation).await;
    assert_eq!(reservation.adapter.total_allocations(), 1);
    assert_eq!(slots.available_permits(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_delivers_terminal_after_core_normalizer_join() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (reservation, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let original_rtc = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    reservation
        ._core
        .drain_connection_lifecycle_tasks()
        .await
        .unwrap();
    assert_eq!(reservation._core.connection_lifecycle_task_count(), 0);
    assert!(reservation
        ._core
        .connection_transport(&reservation.id)
        .is_ok());
    reservation
        .adapter
        .end_original_binding(reservation.id.clone(), &binding, EndReason::Normal)
        .await
        .unwrap();
    // The closed normalizer receiver forces the existing direct terminal sink.
    assert!(reservation
        ._core
        .connection_transport(&reservation.id)
        .is_err());
    assert_eq!(reservation._peer.peer().snapshot().connections, 0);
    original_rtc.close().await.unwrap();
    assert_eq!(reservation.adapter.route_count(), 0);
    assert_eq!(slots.available_permits(), 1);
    // Reap/join the original fallback task accepted after normalizer closure.
    original_fixture_joined(&reservation).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_rejects_foreign_adapter_with_absent_route() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (original, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let rtc = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let foreign = CanonicalWebRtcAdapter::new();
    assert_eq!(foreign.route_count(), 0);
    assert!(matches!(
        foreign
            .end_original_binding(original.id.clone(), &binding, EndReason::Normal)
            .await,
        Err(rvoip_core::error::RvoipError::AdmissionRejected(
            "Original WebRTC cleanup binding differs"
        ))
    ));
    // Foreign absence cannot claim successful full cleanup of another
    // adapter's original backend/controller/Core relationship or release quota.
    assert_eq!(original.adapter.route_count(), 1);
    assert!(original._core.connection_transport(&original.id).is_ok());
    let still_original = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&rtc, &still_original));
    assert_eq!(slots.available_permits(), 0);
    original
        .adapter
        .end_original_binding(original.id.clone(), &binding, EndReason::Normal)
        .await
        .unwrap();
    rtc.close().await.unwrap();
    original_fixture_joined(&original).await;
    assert_eq!(slots.available_permits(), 1);
    assert_eq!(foreign.total_allocations(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_releases_joined_weak_owners() {
    let (binding_weak, adapter_weak, core_weak, rtc_weak) = {
        let physical = CancellationToken::new();
        let slots = Arc::new(tokio::sync::Semaphore::new(1));
        let (reservation, binding) = Reservation::new(
            &physical,
            slots.clone().acquire_owned().await.unwrap(),
            ConnectionId::new(),
        );
        binding
            .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
            .unwrap();
        let rtc = binding
            .peer(Instant::now() + Duration::from_secs(3))
            .await
            .unwrap();
        let weak = (
            Arc::downgrade(&binding),
            Arc::downgrade(&reservation.adapter),
            Arc::downgrade(&reservation._core),
            Arc::downgrade(&rtc),
        );
        reservation
            .adapter
            .end_original_binding(reservation.id.clone(), &binding, EndReason::Normal)
            .await
            .unwrap();
        original_fixture_joined(&reservation).await;
        assert_eq!(slots.available_permits(), 1);
        weak
    };
    // The saved terminal acknowledgment must not retain its original Core,
    // adapter, physical binding or actual peer after all original joins.
    assert!(binding_weak.upgrade().is_none());
    assert!(adapter_weak.upgrade().is_none());
    assert!(core_weak.upgrade().is_none());
    assert!(rtc_weak.upgrade().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_cancelled_observer_and_retries_join_one_terminal() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (reservation, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let rtc = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let gate = reservation
        ._core
        .install_original_terminal_test_gate(rvoip_core::Transport::WebRtc, reservation.id.clone())
        .unwrap();
    let close = |adapter: Arc<CanonicalWebRtcAdapter>, binding: Arc<WebRtcPhysicalBinding>, id| {
        tokio::spawn(async move {
            adapter
                .end_original_binding(id, &binding, EndReason::Normal)
                .await
        })
    };
    let first = close(
        reservation.adapter.clone(),
        binding.clone(),
        reservation.id.clone(),
    );
    tokio::time::timeout(Duration::from_secs(3), gate.wait_entered())
        .await
        .unwrap();
    assert_eq!(reservation.adapter.route_count(), 0);
    assert_eq!(
        slots.available_permits(),
        1,
        "actual physical resources joined before terminal activation"
    );
    assert!(reservation
        ._core
        .connection_transport(&reservation.id)
        .is_ok());
    first.abort();
    assert!(
        first.await.unwrap_err().is_cancelled(),
        "only the result observer was cancelled"
    );
    let mut second = close(
        reservation.adapter.clone(),
        binding.clone(),
        reservation.id.clone(),
    );
    let mut third = close(
        reservation.adapter.clone(),
        binding.clone(),
        reservation.id.clone(),
    );
    // Both observers must await the already accepted original task. Neither
    // route absence nor a new terminal task may manufacture an early receipt.
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut second)
        .await
        .is_err());
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut third)
        .await
        .is_err());
    assert!(reservation
        ._core
        .connection_transport(&reservation.id)
        .is_ok());
    assert_eq!(reservation._peer.peer().snapshot().connections, 1);
    gate.release();
    tokio::time::timeout(Duration::from_secs(3), gate.wait_joined())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), second)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), third)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    original_fixture_joined(&reservation).await;
    assert_eq!(reservation.adapter.total_allocations(), 1);
    assert_eq!(slots.available_permits(), 1);
    rtc.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_last_observer_drop_leaves_only_owned_core_terminal() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (reservation, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let rtc = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let binding_weak = Arc::downgrade(&binding);
    let rtc_weak = Arc::downgrade(&rtc);
    let adapter_weak = Arc::downgrade(&reservation.adapter);
    let core_weak = Arc::downgrade(&reservation._core);
    let gate = reservation
        ._core
        .install_original_terminal_test_gate(rvoip_core::Transport::WebRtc, reservation.id.clone())
        .unwrap();
    let adapter = reservation.adapter.clone();
    let owned_binding = binding.clone();
    let connection = reservation.id.clone();
    let observer = tokio::spawn(async move {
        adapter
            .end_original_binding(connection, &owned_binding, EndReason::Normal)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), gate.wait_entered())
        .await
        .unwrap();
    assert_eq!(reservation.adapter.route_count(), 0);
    observer.abort();
    assert!(observer.await.unwrap_err().is_cancelled());
    drop(binding);
    drop(rtc);
    // Keep the actual Core owner to join its accepted task explicitly, but
    // drop every external binding/RTC observer. The terminal receipt must not
    // retain either physical owner while Core's original handler is paused.
    assert!(binding_weak.upgrade().is_none());
    assert!(rtc_weak.upgrade().is_none());
    assert_eq!(slots.available_permits(), 1);
    assert!(reservation
        ._core
        .connection_transport(&reservation.id)
        .is_ok());
    gate.release();
    tokio::time::timeout(Duration::from_secs(3), gate.wait_joined())
        .await
        .unwrap();
    original_fixture_joined(&reservation).await;
    drop(reservation);
    assert!(adapter_weak.upgrade().is_none());
    assert!(core_weak.upgrade().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn original_binding_cleanup_retires_adapter_after_core_peer_owner_is_gone() {
    let physical = CancellationToken::new();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (reservation, binding) = Reservation::new(
        &physical,
        slots.clone().acquire_owned().await.unwrap(),
        ConnectionId::new(),
    );
    binding
        .start_peer(WebRtcConfig::loopback(), PeerRole::Answerer)
        .unwrap();
    let rtc = binding
        .peer(Instant::now() + Duration::from_secs(3))
        .await
        .unwrap();
    let Reservation {
        adapter,
        _core: core,
        _peer: owner,
        id,
    } = reservation;
    let admitted_peer = owner.peer();
    // This is an actual production shutdown ordering, not an injected map
    // deletion. Owner Drop retires Core synchronously. After ordinary workers
    // stop, its deferred adapter cleanup cannot spawn, so original quarantine
    // and adapter resource ownership must remain until the custodian joins them.
    core.drain_connection_lifecycle_tasks().await.unwrap();
    drop(owner);
    assert!(core.connection_transport(&id).is_err());
    assert_eq!(core.adapter_cleanup_quarantine_count(), 1);
    assert_eq!(admitted_peer.snapshot().connections, 1);
    assert!(adapter.is_connection_live(&id));
    binding.shutdown().await.unwrap();
    assert_eq!(adapter.route_count(), 1);
    assert_eq!(
        slots.available_permits(),
        0,
        "physical closure cannot refund a retained adapter route"
    );
    // The old service guard skips its Core end call in exactly this state and
    // immediately reports cleanup-unconfirmed because the adapter stays live.
    if core.connection_transport(&id).is_ok() {
        core.end_connection(id.clone(), EndReason::Normal)
            .await
            .unwrap();
    }
    assert!(
        adapter.is_connection_live(&id),
        "old guarded cleanup leaves the original route owned"
    );
    // The historical original binding remains the cleanup capability after
    // its Core relationship and peer authority retired. This publishes no new
    // connection or movement: its terminal receipt resolves original quarantine.
    adapter
        .end_original_binding(id.clone(), &binding, EndReason::Normal)
        .await
        .unwrap();
    assert!(!adapter.is_connection_live(&id));
    assert!(core.connection_transport(&id).is_err());
    assert_eq!(core.adapter_cleanup_quarantine_count(), 0);
    assert_eq!(admitted_peer.snapshot().connections, 0);
    assert_eq!(adapter.total_allocations(), 1);
    assert_eq!(slots.available_permits(), 1);
    rtc.close().await.unwrap();
    core.drain_prepared_outbound_connections().await;
    core.drain_connection_lifecycle_tasks().await.unwrap();
    assert_eq!(core.connection_lifecycle_task_count(), 0);
}
