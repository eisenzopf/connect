//! Actual Core scene tick + Store fences + Service's contended leaf callback.
//! Relationship/transport fixtures are synthetic; this is not native IO proof.
use super::*;
use connect_store::{AuthorityGuardSet, AuthorityMode, AuthorityScope};
use rvoip_core::{
    identity::AuthenticationMethod,
    ids::{ConversationId, ParticipantId, StreamId},
    participant::{ParticipantKind, ParticipantRole},
    relationships::Membership,
    staged_streams::{test_support::ControlledAudioScene, *},
    stream::{MediaFrame, StreamKind},
    stream_lifecycle::*,
};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    mpsc::{sync_channel, Receiver, SyncSender},
};
use tokio_util::sync::CancellationToken;

struct Cleanup;
#[async_trait::async_trait]
impl MediaPortCleanup for Cleanup {
    async fn close(&self) -> ControlResult<()> {
        Ok(())
    }
}
#[derive(Default)]
struct Adapter(Mutex<Option<AdapterMediaPort>>);
#[async_trait::async_trait]
impl StagedMediaAdapter for Adapter {
    async fn prepare(
        &self,
        ports: Vec<PortPreparation>,
        _: tokio::time::Instant,
        _: CancellationToken,
    ) -> ControlResult<()> {
        for p in ports {
            if p.is_new {
                p.port.install_cleanup(Arc::new(Cleanup))?;
                p.port
                    .observe_ready(&p.port.binding().negotiated.format, true, true, Some(0))?;
            }
            *self.0.lock().unwrap() = Some(p.port);
        }
        Ok(())
    }
}
fn principal(identity: &str) -> AuthenticatedPrincipal {
    AuthenticatedPrincipal {
        subject: identity.into(),
        tenant: Some("fixture".into()),
        issuer: Some("fixture".into()),
        scopes: vec![],
        expires_at: Some(chrono::Utc::now() + chrono::Duration::minutes(5)),
        method: AuthenticationMethod::Bearer,
        assurance: IdentityAssurance::Anonymous,
    }
}
fn owner(
    identity: &str,
    cid: &ConversationId,
    sid: &SessionId,
    connection: &ConnectionId,
) -> MediaOwner {
    let now = chrono::Utc::now();
    let identity = IdentityBinding::from_authenticated(
        &principal(identity),
        IdentityId::from_string(identity),
        now,
    )
    .unwrap();
    let member = Membership::admit(
        identity.clone(),
        cid.clone(),
        ParticipantId::from_string(connection.as_str()),
        ParticipantKind::Human,
        ParticipantRole::Agent,
    )
    .unwrap();
    let presence = SessionAppearance::admit(&member, sid.clone(), None).unwrap();
    let mut endpoint = EndpointBinding::provision(
        identity.clone(),
        EndpointId::from_string(format!("ep-{}", connection.as_str())),
        DeviceId::from_string(format!("dev-{}", connection.as_str())),
        Transport::Quic,
        "fixture://local".into(),
    )
    .unwrap();
    endpoint
        .verify(&identity, "synthetic-resource-proof".into(), now)
        .unwrap();
    let mut binding =
        ConnectionBinding::prepare(&member, &presence, &endpoint, connection.clone()).unwrap();
    binding
        .observe_ready(&member, &presence, &endpoint, 1)
        .unwrap();
    binding.activate(&member, &presence, &endpoint).unwrap();
    let capability = MediaCapability {
        medium: Medium::Audio,
        formats: vec![MediaFormat::Encoded {
            name: "pcmu".into(),
            clock_rate_hz: 8000,
            channels: 1,
            fmtp: None,
        }],
    };
    let profile = EndpointMediaCapabilities {
        send: vec![capability.clone()],
        receive: vec![capability],
        max_streams: 1,
        renegotiation: true,
    };
    MediaOwner {
        actor: member.clone(),
        member,
        presence,
        endpoint,
        connection: binding,
        may_control: true,
        send_media: [Medium::Audio].into_iter().collect(),
        receive_media: [Medium::Audio].into_iter().collect(),
        local: profile.clone(),
        remote: profile,
        may_route_source: true,
    }
}
type Active = Arc<Mutex<BTreeMap<ConnectionId, usize>>>;
struct Lease {
    owner: MediaOwner,
    _read: Arc<AuthorityGuardSet>,
    tracked: Option<(Active, ConnectionId)>,
}
impl MediaAuthorityLease for Lease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some((active, id)) = &self.tracked {
            let mut active = active.lock().unwrap();
            let count = active.get_mut(id).unwrap();
            *count -= 1;
            if *count == 0 {
                active.remove(id);
            }
        }
    }
}
struct Probe {
    service: Arc<Policy>,
    owners: BTreeMap<ConnectionId, MediaOwner>,
    reads: Vec<AuthorityScope>,
    active: Active,
    armed: AtomicBool,
    legacy: bool,
    start: Mutex<Option<(SyncSender<()>, Receiver<()>)>>,
    triggered: AtomicBool,
    callback_us: AtomicU64,
    contended: AtomicUsize,
}
impl Probe {
    fn grant(
        &self,
        scope: &MediaScope<'_>,
        queued: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let owner = self
            .owners
            .get(scope.connection)
            .ok_or(StreamControlError::Authority)?
            .clone();
        if scope.principal.subject != owner.member.identity().as_str() {
            return Err(StreamControlError::Authority);
        }
        if let MediaPurpose::Receive { source } = scope.purpose {
            let origin = self
                .owners
                .get(&source.connection)
                .ok_or(StreamControlError::Authority)?;
            if source.conversation != *scope.conversation
                || source.session != *scope.session
                || source.participant != *origin.member.participant()
                || source.membership_epoch != origin.member.epoch()
                || source.connection_generation != origin.connection.generation()
            {
                return Err(StreamControlError::Authority);
            }
        }
        // Actual Store ownership, not a replacement NativeMediaOrigin grant.
        // The fixture owns both real identities and the whole Conversation.
        let read = self
            .service
            .store
            .try_acquire_authority(&self.reads)
            .map_err(packet_authority_error)?;
        let tracked = if queued && matches!(scope.purpose, MediaPurpose::Publish) {
            *self
                .active
                .lock()
                .unwrap()
                .entry(scope.connection.clone())
                .or_default() += 1;
            Some((self.active.clone(), scope.connection.clone()))
        } else {
            None
        };
        Ok(Box::new(Lease {
            owner,
            _read: read,
            tracked,
        }))
    }
}
impl MediaPolicy for Probe {
    fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.grant(scope, false)
    }
    fn current_queued(
        &self,
        scope: &MediaScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        // This synchronous test calls only scene.tick() while armed. A lease
        // for another source can therefore be held only by this actual tick.
        let other = self
            .active
            .lock()
            .unwrap()
            .keys()
            .any(|id| id != scope.connection);
        if self.armed.load(Ordering::Acquire)
            && other
            && matches!(scope.purpose, MediaPurpose::Publish)
            && !self.triggered.swap(true, Ordering::AcqRel)
        {
            let _registry = self.service.entries.lock().unwrap();
            let (start, started) = self.start.lock().unwrap().take().unwrap();
            start.send(()).unwrap();
            started.recv_timeout(Duration::from_secs(1)).unwrap();
            let began = std::time::Instant::now();
            let result = if self.legacy {
                self.service.current(scope)
            } else {
                self.service.current_queued(scope)
            };
            self.callback_us
                .store(began.elapsed().as_micros() as u64, Ordering::Release);
            assert!(matches!(result, Err(StreamControlError::NotReady)));
            self.contended.fetch_add(1, Ordering::AcqRel);
            return result;
        }
        self.grant(scope, true)
    }
}
fn publish(adapter: &Adapter, timestamp: u32) {
    let port = adapter.0.lock().unwrap().as_ref().unwrap().clone();
    port.try_publish(&MediaFrame {
        stream_id: port.binding().transport_id.clone(),
        kind: StreamKind::Audio,
        payload: vec![0xd5; 160].into(),
        timestamp_rtp: timestamp,
        source_clock: None,
        captured_at: chrono::Utc::now(),
        payload_type: Some(0),
    })
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn actual_mixer_releases_store_reader_before_writer_budget_when_retained_service_callback_is_queued(
) {
    for legacy in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Default::default()).unwrap(),
        );
        let alice = store.create_identity("alice").unwrap().id;
        let bob = store.create_identity("bob").unwrap().id;
        let key = store
            .issue(
                &alice,
                connect_store::Scope::operator(),
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        let connect_protocol::Outcome::Conversation(conversation) = store
            .execute(
                &key.id,
                &connect_protocol::Operation::Create {
                    request_id: "mixer".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        bob.clone()
                    ]),
                },
            )
            .unwrap()
        else {
            panic!("conversation");
        };
        let cid = ConversationId::from_string(&conversation.id);
        let sid = SessionId::from_string("mixer-session");
        let ids = [
            ConnectionId::from_string("a"),
            ConnectionId::from_string("b"),
        ];
        let scopes = |mode| {
            vec![
                AuthorityScope::identity(&alice, mode).unwrap(),
                AuthorityScope::identity(&bob, mode).unwrap(),
                AuthorityScope::conversation(&conversation.id, mode).unwrap(),
            ]
        };
        let (start, requested) = sync_channel(1);
        let (started, ready) = sync_channel(1);
        let (finished, result) = sync_channel(1);
        let probe = Arc::new(Probe {
            service: Policy::new(store.clone(), 8),
            owners: [
                (ids[0].clone(), owner(&alice, &cid, &sid, &ids[0])),
                (ids[1].clone(), owner(&bob, &cid, &sid, &ids[1])),
            ]
            .into_iter()
            .collect(),
            reads: scopes(AuthorityMode::Read),
            active: Arc::new(Mutex::new(BTreeMap::new())),
            armed: AtomicBool::new(false),
            legacy,
            start: Mutex::new(Some((start, ready))),
            triggered: AtomicBool::new(false),
            callback_us: AtomicU64::new(0),
            contended: AtomicUsize::new(0),
        });
        let scene = ControlledAudioScene::new(cid.clone(), sid.clone(), probe.clone());
        let handles = Arc::new(StreamHandleAllocator::default());
        let mut controllers = Vec::new();
        let mut adapters = Vec::new();
        let mut outputs = Vec::new();
        for (id, identity) in ids.iter().zip([&alice, &bob]) {
            let adapter = Arc::new(Adapter::default());
            let controller = StreamController::new(
                cid.clone(),
                sid.clone(),
                id.clone(),
                principal(identity),
                probe.clone(),
                adapter.clone(),
                handles.clone(),
                2,
            )
            .unwrap();
            controller
                .change(
                    controller.revision(),
                    &[DesiredStream {
                        logical_id: StreamId::from_string("audio"),
                        medium: Medium::Audio,
                        direction: StreamDirection::Both,
                    }],
                    &BTreeSet::new(),
                    tokio::time::Instant::now() + Duration::from_secs(2),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            scene
                .attach(
                    controller.clone(),
                    tokio::time::Instant::now() + Duration::from_secs(2),
                )
                .await
                .unwrap();
            outputs.push(
                adapter
                    .0
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .take_egress()
                    .unwrap(),
            );
            controllers.push(controller);
            adapters.push(adapter);
        }
        for (index, controller) in controllers.iter().enumerate() {
            scene
                .change_subscriptions(
                    &ids[index],
                    controller.revision(),
                    audio_subscriptions::AudioSubscriptionChange::Subscribe(vec![
                        audio_subscriptions::AudioSubscription::Connection(ids[1 - index].clone()),
                    ]),
                    tokio::time::Instant::now() + Duration::from_secs(2),
                )
                .await
                .unwrap();
        }
        // Establish actual decoded mixer output before introducing contention.
        for adapter in &adapters {
            publish(adapter, 0);
        }
        scene.tick();
        for output in &mut outputs {
            output
                .try_recv()
                .expect("actual mixer output missing")
                .deliver(|frame| async move {
                    assert_eq!(frame.payload.len(), 160);
                    Ok(())
                })
                .await
                .unwrap();
        }
        assert!(probe.active.lock().unwrap().is_empty());
        for adapter in &adapters {
            publish(adapter, 160);
        }
        let writer_store = store.clone();
        let writes = scopes(AuthorityMode::Write);
        let writer = std::thread::spawn(move || {
            requested.recv_timeout(Duration::from_secs(5)).unwrap();
            let began = std::time::Instant::now();
            let wait = Store::replacement_projection_wait_for_test();
            started.send(()).unwrap();
            let acquired = writer_store.acquire_authority(&writes, &wait);
            let outcome = acquired.as_ref().map(|_| ()).map_err(|e| e.code.clone());
            drop(acquired);
            finished.send((outcome, began.elapsed())).unwrap();
        });
        probe.armed.store(true, Ordering::Release);
        let began = std::time::Instant::now();
        scene.tick();
        let tick = began.elapsed();
        probe.armed.store(false, Ordering::Release);
        assert!(
            probe.triggered.load(Ordering::Acquire),
            "tick did not own a reader while admitting another source"
        );
        assert_eq!(probe.contended.load(Ordering::Acquire), 1);
        assert!(
            probe.active.lock().unwrap().is_empty(),
            "mixer retained an authority reader after its tick"
        );
        let (outcome, elapsed) = result.recv_timeout(Duration::from_secs(1)).unwrap();
        writer.join().unwrap();
        let callback = probe.callback_us.load(Ordering::Acquire);
        if legacy {
            assert_eq!(outcome.unwrap_err(), "authority-busy");
            assert!(
                callback >= 8000,
                "fresh ingress no longer exercised its retry budget"
            );
        } else {
            outcome.expect("queued callback prevented the actual Store writer from draining within its original5ms budget");
            assert!(callback < 5000);
            assert!(tick < Duration::from_millis(50));
        }
        eprintln!(
            "mixer_writer_drain legacy={legacy} callback_us={callback} tick_us={} writer_us={}",
            tick.as_micros(),
            elapsed.as_micros()
        );
        drop(scene);
        drop(outputs);
        for controller in controllers {
            assert_eq!(
                controller
                    .shutdown(tokio::time::Instant::now() + Duration::from_secs(2))
                    .await,
                0
            );
        }
        store
            .try_acquire_authority(&scopes(AuthorityMode::Write))
            .expect("fixture leaked Store custody");
    }
}
