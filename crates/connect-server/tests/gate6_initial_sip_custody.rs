//! Engine custody failures, not a substitute for canonical public SIP/media proof.
#![allow(clippy::unwrap_used)]
use async_trait::async_trait;
use chrono::Utc;
use rvoip_core::{
    adapter::*, capability::*, connection::*, identity::*, ids::*, message::Message,
    orchestrator::participant_control::*, participant::*, participant_transitions::*,
    relationships::*, session::SessionMedium, staged_streams, stream::MediaStream, Orchestrator,
    Result, RvoipError,
};
use std::{
    any::Any,
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Adapter {
    live: Mutex<HashSet<ConnectionId>>,
    rx: Mutex<Option<mpsc::Receiver<AdapterEvent>>>,
    _tx: mpsc::Sender<AdapterEvent>,
    activated: AtomicUsize,
    ended: AtomicUsize,
    block: AtomicBool,
    entered: tokio::sync::Notify,
}
impl Adapter {
    fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel(8);
        Arc::new(Self {
            live: Mutex::new(HashSet::new()),
            rx: Mutex::new(Some(rx)),
            _tx: tx,
            activated: AtomicUsize::new(0),
            ended: AtomicUsize::new(0),
            block: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
        })
    }
}
#[async_trait]
impl ConnectionAdapter for Adapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }
    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }
    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            staged_outbound_activation: true,
            ..Default::default()
        }
    }
    fn is_connection_live(&self, id: &ConnectionId) -> bool {
        self.live.lock().unwrap().contains(id)
    }
    async fn originate(&self, r: OriginateRequest) -> Result<ConnectionHandle> {
        let id = ConnectionId::new();
        self.live.lock().unwrap().insert(id.clone());
        Ok(ConnectionHandle::new(Connection {
            id,
            session_id: r.session_id,
            participant_id: r.participant_id,
            transport: Transport::Sip,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: r.capabilities,
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        }))
    }
    async fn activate_outbound(&self, _: ConnectionId) -> Result<()> {
        self.activated.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        if self.block.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
    async fn accept(&self, _: ConnectionId) -> Result<()> {
        Err(RvoipError::InvalidState("no inbound fixture"))
    }
    async fn reject(&self, id: ConnectionId, _: RejectReason) -> Result<()> {
        self.end(id, EndReason::Normal).await
    }
    async fn end(&self, id: ConnectionId, _: EndReason) -> Result<()> {
        if self.live.lock().unwrap().remove(&id) {
            self.ended.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
    async fn hold(&self, _: ConnectionId) -> Result<()> {
        Ok(())
    }
    async fn resume(&self, _: ConnectionId) -> Result<()> {
        Ok(())
    }
    async fn transfer(&self, _: ConnectionId, _: TransferTarget) -> Result<()> {
        Err(RvoipError::InvalidState("no transfer fixture"))
    }
    async fn streams(&self, _: ConnectionId) -> Result<Vec<Arc<dyn MediaStream>>> {
        Ok(vec![])
    }
    async fn send_message(&self, _: ConnectionId, _: Message) -> Result<()> {
        Ok(())
    }
    async fn send_dtmf(&self, _: ConnectionId, _: &str, _: u32) -> Result<()> {
        Ok(())
    }
    async fn renegotiate_media(
        &self,
        _: ConnectionId,
        _: CapabilityDescriptor,
    ) -> Result<NegotiatedCodecs> {
        Err(RvoipError::InvalidState("no renegotiation fixture"))
    }
    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.rx.lock().unwrap().take().unwrap()
    }
    fn capabilities(&self) -> CapabilityDescriptor {
        CapabilityDescriptor::default()
    }
    async fn verify_request_signature(
        &self,
        _: ConnectionId,
        _: SignatureHeaders,
    ) -> Result<IdentityAssurance> {
        Err(RvoipError::InvalidState("no signature fixture"))
    }
}
struct Policy {
    domain: Arc<()>,
    member: Membership,
    endpoint: EndpointBinding,
    denied: Arc<AtomicBool>,
}
#[derive(Clone)]
struct Invocation {
    domain: Arc<()>,
    operation: String,
    connection: ConnectionId,
    member: Membership,
    endpoint: EndpointBinding,
    denied: Arc<AtomicBool>,
}
impl ParticipantControlLease for Invocation {
    fn current(&self, s: &ParticipantScope<'_>) -> ControlResult<ParticipantAuthority> {
        if self.denied.load(Ordering::SeqCst)
            || s.mutation.operation_id != self.operation
            || s.mutation.participant != *self.member.participant()
            || !matches!(&s.mutation.change,ParticipantChange::Admit{connection,..} if connection==&self.connection)
        {
            return Err(ParticipantControlError::Authority);
        }
        Ok(ParticipantAuthority {
            actor: self.member.clone(),
            target: self.member.clone(),
            endpoint: Some(self.endpoint.clone()),
            may_send_audio: true,
            may_receive_audio: true,
            confirmation_required: false,
            confirmed_generation: None,
        })
    }
}
impl ParticipantPolicy for Policy {
    fn current(
        &self,
        _: &ParticipantScope<'_>,
    ) -> ControlResult<Box<dyn ParticipantAuthorityLease + '_>> {
        Err(ParticipantControlError::Authority)
    }
    fn current_control(
        &self,
        s: &ParticipantScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ControlResult<ParticipantControl> {
        let i = held
            .downcast_ref::<Invocation>()
            .or_else(|| held.downcast_ref::<InitialInvocation>().map(|v| &v.intent))
            .or_else(|| {
                held.downcast_ref::<AdmissionInvocation>()
                    .map(|v| &v.intent)
            })
            .filter(|i| Arc::ptr_eq(&i.domain, &self.domain))
            .ok_or(ParticipantControlError::Authority)?;
        i.current(s)?;
        ParticipantControl::new(
            Box::new(i.clone()),
            CancellationToken::new(),
            tokio::time::Instant::now() + Duration::from_secs(2),
        )
    }
}
impl staged_streams::MediaPolicy for Policy {
    fn current(
        &self,
        _: &staged_streams::MediaScope<'_>,
    ) -> staged_streams::ControlResult<Box<dyn staged_streams::MediaAuthorityLease>> {
        Err(staged_streams::StreamControlError::Authority)
    }
}
struct Fixture {
    core: Arc<Orchestrator>,
    adapter: Arc<Adapter>,
    policy: Arc<Policy>,
    principal: AuthenticatedPrincipal,
    sid: SessionId,
}
impl Fixture {
    fn new() -> Self {
        let core = Orchestrator::new(Default::default());
        let adapter = Adapter::new();
        core.register(adapter.clone()).unwrap();
        let principal = AuthenticatedPrincipal {
            subject: "phone".into(),
            tenant: Some("tenant".into()),
            issuer: Some("fixture".into()),
            scopes: vec![],
            expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Identified {
                credential_kind: CredentialKind::Bearer,
            },
        };
        let identity = IdentityBinding::from_authenticated(
            &principal,
            IdentityId::from_string("phone"),
            Utc::now(),
        )
        .unwrap();
        let cid = ConversationId::from_string("cid");
        let sid = SessionId::from_string("sid");
        let member = Membership::admit(
            identity.clone(),
            cid.clone(),
            ParticipantId::from_string("pid"),
            ParticipantKind::Human,
            ParticipantRole::Customer,
        )
        .unwrap();
        let mut endpoint = EndpointBinding::provision(
            identity.clone(),
            EndpointId::from_string("ep"),
            DeviceId::from_string("dev"),
            Transport::Sip,
            "sip:phone@127.0.0.1:5099".into(),
        )
        .unwrap();
        endpoint
            .verify(&identity, "fixture-proof".into(), Utc::now())
            .unwrap();
        let policy = Arc::new(Policy {
            domain: Arc::new(()),
            member,
            endpoint,
            denied: Arc::new(AtomicBool::new(false)),
        });
        core.project_external_session(
            cid,
            sid.clone(),
            TenantId::from_string("tenant"),
            SessionMedium::Mixed,
            [],
        )
        .unwrap();
        core.enable_staged_participant_control(
            &sid,
            policy.clone(),
            policy.clone(),
            ParticipantLimits {
                connections: 1,
                ..Default::default()
            },
        )
        .unwrap();
        Self {
            core,
            adapter,
            policy,
            principal,
            sid,
        }
    }
    async fn prepare(
        &self,
        operation: &str,
    ) -> (
        rvoip_core::orchestrator::PreparedOutboundConnection,
        ParticipantMutation,
        Invocation,
    ) {
        let ticket = self
            .core
            .prepare_outbound_connection(
                OriginateRequest::new(
                    self.sid.clone(),
                    self.policy.member.participant().clone(),
                    self.policy.endpoint.address(),
                    Direction::Outbound,
                    CapabilityDescriptor::default(),
                )
                .with_transport(Transport::Sip),
            )
            .await
            .unwrap();
        let mutation = ParticipantMutation {
            operation_id: operation.into(),
            correlation_id: operation.into(),
            expected_revision: 1,
            participant: self.policy.member.participant().clone(),
            change: ParticipantChange::Admit {
                connection: ticket.connection_id().clone(),
                role_override: None,
            },
        };
        let invocation = Invocation {
            domain: self.policy.domain.clone(),
            operation: operation.into(),
            connection: ticket.connection_id().clone(),
            member: self.policy.member.clone(),
            endpoint: self.policy.endpoint.clone(),
            denied: self.policy.denied.clone(),
        };
        (ticket, mutation, invocation)
    }
    fn empty(&self) {
        assert_eq!(self.core.participant_capacity(&self.sid).unwrap(), (0, 0));
        let session = self.core.session(&self.sid).unwrap();
        let session = session.read().unwrap();
        assert!(session.participants.is_empty());
        assert!(session.connections.is_empty());
    }
    async fn finish(&self) {
        self.core.drain_prepared_outbound_connections().await;
        self.core.drain_connection_lifecycle_tasks().await;
        self.empty();
        assert!(self.adapter.live.lock().unwrap().is_empty());
    }
}
fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_millis(300)
}

#[tokio::test]
async fn wrong_domain_or_current_withdrawal_performs_zero_activation_and_owned_cleanup() {
    for denied in [false, true] {
        let f = Fixture::new();
        let (ticket, mutation, mut i) = f.prepare("refused").await;
        if denied {
            f.policy.denied.store(true, Ordering::SeqCst);
        } else {
            i.domain = Arc::new(());
        }
        assert!(ticket
            .commit_staged_participant_with_held(
                f.principal.clone(),
                mutation,
                deadline(),
                CancellationToken::new(),
                Box::new(i),
                |_| async { panic!("no media after refusal") }
            )
            .await
            .is_err());
        assert_eq!(f.adapter.activated.load(Ordering::SeqCst), 0);
        assert_eq!(f.adapter.ended.load(Ordering::SeqCst), 1);
        f.finish().await;
    }
}
#[tokio::test]
async fn another_route_cannot_consume_the_owned_ticket() {
    let f = Fixture::new();
    let (ticket, mut mutation, i) = f.prepare("wrong-route").await;
    mutation.change = ParticipantChange::Admit {
        connection: ConnectionId::new(),
        role_override: None,
    };
    assert!(ticket
        .commit_staged_participant_with_held(
            f.principal.clone(),
            mutation,
            deadline(),
            CancellationToken::new(),
            Box::new(i),
            |_| async { panic!("no foreign media") }
        )
        .await
        .is_err());
    assert_eq!(f.adapter.activated.load(Ordering::SeqCst), 0);
    f.finish().await;
}
#[tokio::test]
async fn failed_media_drops_actual_route_and_releases_slot_even_with_retained_stale_metadata() {
    let f = Fixture::new();
    let retained = Arc::new(Mutex::new(Vec::new()));
    for n in 0..2 {
        let (ticket, mutation, i) = f.prepare(&format!("media-{n}")).await;
        let seen = retained.clone();
        let expected = i.connection.clone();
        assert!(ticket
            .commit_staged_participant_with_held(
                f.principal.clone(),
                mutation,
                deadline(),
                CancellationToken::new(),
                Box::new(i),
                move |p| async move {
                    assert!(p.is_current());
                    assert_eq!(p.member().participant().as_str(), "pid");
                    assert_eq!(p.binding().connection(), &expected);
                    seen.lock().unwrap().push(p);
                    Err(ParticipantControlError::Unsupported)
                }
            )
            .await
            .is_err());
        f.empty();
        assert_eq!(f.adapter.activated.load(Ordering::SeqCst), n + 1);
        assert_eq!(f.adapter.ended.load(Ordering::SeqCst), n + 1);
    }
    assert!(retained.lock().unwrap().iter().all(|p| !p.is_current()));
    f.finish().await;
}
#[tokio::test]
async fn callback_success_cannot_invent_a_guarded_controller_or_media_readiness() {
    let f = Fixture::new();
    let (ticket, mutation, i) = f.prepare("no-controller").await;
    let admission = Arc::new(i.clone()) as Arc<dyn Any + Send + Sync>;
    assert!(ticket
        .commit_staged_participant_with_held(
            f.principal.clone(),
            mutation,
            deadline(),
            CancellationToken::new(),
            Box::new(i),
            move |_| async move { Ok(admission) }
        )
        .await
        .is_err());
    assert_eq!(f.adapter.activated.load(Ordering::SeqCst), 1);
    f.finish().await;
}
#[tokio::test]
async fn dropped_activation_keeps_cleanup_owned_by_the_original_supervisor() {
    let f = Fixture::new();
    f.adapter.block.store(true, Ordering::SeqCst);
    let (ticket, mutation, i) = f.prepare("drop").await;
    let principal = f.principal.clone();
    let task = tokio::spawn(async move {
        ticket
            .commit_staged_participant_with_held(
                principal,
                mutation,
                deadline(),
                CancellationToken::new(),
                Box::new(i),
                |_| async { panic!("activation never completed") },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), f.adapter.entered.notified())
        .await
        .unwrap();
    task.abort();
    let Err(error) = task.await else {
        panic!("task should cancel")
    };
    assert!(error.is_cancelled());
    f.finish().await;
    assert_eq!(f.adapter.activated.load(Ordering::SeqCst), 1);
    assert_eq!(f.adapter.ended.load(Ordering::SeqCst), 1);
}

#[path = "support/initial_sip_media.rs"]
mod initial_sip_media;

struct InitialInvocation {
    intent: Invocation,
    _read: tokio::sync::OwnedRwLockReadGuard<()>,
}
struct AdmissionInvocation {
    intent: Invocation,
    _read: tokio::sync::OwnedRwLockReadGuard<()>,
}
#[tokio::test]
async fn initial_reader_drops_before_answer_writer_and_fresh_admission_read_is_not_retained() {
    for withdrawn in [false, true] {
        let f = Fixture::new();
        let (ticket, mutation, intent) = f.prepare("answer-fence").await;
        let lock = Arc::new(tokio::sync::RwLock::new(()));
        let initial = InitialInvocation {
            intent: intent.clone(),
            _read: lock.clone().read_owned().await,
        };
        let answer_lock = lock.clone();
        let denied = f.policy.denied.clone();
        let observed = Arc::new(AtomicBool::new(false));
        let seen = observed.clone();
        let result = ticket
            .commit_staged_participant_with_held(
                f.principal.clone(),
                mutation,
                deadline(),
                CancellationToken::new(),
                Box::new(initial),
                move |_| async move {
                    let writer = answer_lock
                        .clone()
                        .try_write_owned()
                        .expect("initial IO reader must release before answer recording");
                    seen.store(true, Ordering::SeqCst);
                    drop(writer);
                    if withdrawn {
                        denied.store(true, Ordering::SeqCst);
                    }
                    Ok(Arc::new(AdmissionInvocation {
                        intent,
                        _read: answer_lock.read_owned().await,
                    }) as Arc<dyn Any + Send + Sync>)
                },
            )
            .await;
        assert!(result.is_err()); // No guarded controller installed in this fixture.
        assert!(observed.load(Ordering::SeqCst));
        assert!(lock.try_write().is_ok());
        assert_eq!(f.adapter.activated.load(Ordering::SeqCst), 1);
        assert_eq!(f.adapter.ended.load(Ordering::SeqCst), 1);
        f.finish().await;
    }
}

#[tokio::test]
async fn one_session_slot_is_reserved_before_any_second_physical_activation() {
    let f = Fixture::new();
    f.adapter.block.store(true, Ordering::SeqCst);
    let (first, first_mutation, first_intent) = f.prepare("first-slot").await;
    let principal = f.principal.clone();
    let task = tokio::spawn(async move {
        first
            .commit_staged_participant_with_held(
                principal,
                first_mutation,
                tokio::time::Instant::now() + Duration::from_secs(3),
                CancellationToken::new(),
                Box::new(first_intent),
                |_| async { panic!("blocked activation") },
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), f.adapter.entered.notified())
        .await
        .unwrap();
    let (second, mutation, intent) = f.prepare("second-slot").await;
    assert!(second
        .commit_staged_participant_with_held(
            f.principal.clone(),
            mutation,
            deadline(),
            CancellationToken::new(),
            Box::new(intent),
            |_| async { panic!("no room for a second activation") }
        )
        .await
        .is_err());
    assert_eq!(f.adapter.activated.load(Ordering::SeqCst), 1);
    assert_eq!(f.adapter.ended.load(Ordering::SeqCst), 1);
    task.abort();
    let Err(error) = task.await else {
        panic!("aborted first attempt")
    };
    assert!(error.is_cancelled());
    f.finish().await;
    assert_eq!(f.adapter.ended.load(Ordering::SeqCst), 2);
}
