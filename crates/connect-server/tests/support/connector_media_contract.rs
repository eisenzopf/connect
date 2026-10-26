//! Actual adapter observations, without provider accounts or media effects.
use super::*;
use connect_connector_api::{manifest::*, media::*, Descriptor};
use connect_server::connector_media::{adapter_runtime, validate_adapter};
use std::sync::Arc;

#[tokio::test]
async fn connector_media_checks_real_sip_and_vapi_adapters_without_conflating_native_hosts() {
    let coordinator = rvoip_sip::UnifiedCoordinator::new(rvoip_sip::api::unified::Config::local(
        "connector-contract",
        0,
    ))
    .await
    .unwrap();
    let sip = rvoip_sip::SipAdapter::new(coordinator.clone())
        .await
        .unwrap();
    let contract = connect_connectors::provider_mapping::sip_live_manifest();
    validate_adapter(&contract, sip.as_ref()).unwrap();
    let runtime = adapter_runtime(sip.as_ref()).unwrap();
    assert_eq!(
        runtime.binding,
        LiveBinding::Transport {
            transport: "Sip".into()
        }
    );
    assert_eq!(runtime.codecs, LiveCodecs::Negotiated);
    let vapi = rvoip_vapi::VapiAdapter::new(connect_server::media_profile::vapi_config(
        rvoip_vapi::VapiApiKey::new(String::from("isolated-unused-fixture-key")).unwrap(),
    ))
    .unwrap();
    assert_eq!(
        validate_adapter(&contract, vapi.as_ref()),
        Err(ContractError::MediaRuntime)
    );
    // A signed native provider job is a different binding from a Core adapter,
    // even when both happen to communicate with the same external provider.
    let native = connect_connectors::provider_mapping::vapi_native_manifest();
    assert_eq!(
        validate_adapter(&native, vapi.as_ref()),
        Err(ContractError::MediaRuntime)
    );
    let observed = adapter_runtime(vapi.as_ref()).unwrap();
    let mut independent = Manifest::messaging(Descriptor::one_way("independent-transport"));
    independent.messaging = false;
    independent.operations = [ConnectorOperation::LiveMedia].into();
    independent.media = MediaSupport::Live {
        binding: observed.binding.clone(),
        codecs: observed.codecs.clone(),
        controls: [
            LiveControl::End,
            LiveControl::InboundAdmission,
            LiveControl::StagedOriginate,
        ]
        .into(),
        context: ContextHandling::Unsupported,
        records: RecordHandling::HostAuthorizedStreams,
    };
    validate_adapter(&independent, vapi.as_ref()).unwrap();
    let mut fixture = Manifest::messaging(Descriptor::one_way("fixture"));
    fixture.operations.extend([
        ConnectorOperation::VoiceStart,
        ConnectorOperation::VoiceStatus,
        ConnectorOperation::VoiceEnd,
        ConnectorOperation::MediaExchange,
    ]);
    fixture.media = MediaSupport::LifecycleFixture {
        audio: AudioFormat {
            codec: "pcm-s16le".into(),
            sample_rate: 16000,
            channels: 1,
            frame_ms: 20,
        },
    };
    assert_eq!(
        validate_adapter(&fixture, sip.as_ref()),
        Err(ContractError::UnsupportedMedia)
    );
    sip.shutdown().await.unwrap();
    coordinator
        .shutdown_gracefully(Some(Duration::from_secs(5)))
        .await
        .unwrap();
}

#[test]
fn connector_media_registration_preserves_current_credentials_and_cannot_provision_identity() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        connect_store::Store::initialize(
            &directory.path().join("state.sqlite3"),
            Default::default(),
        )
        .unwrap(),
    );
    let identity = store.create_identity("unprivileged").unwrap();
    let key = store
        .issue(
            &identity.id,
            Scope::messaging(vec![identity.id.clone()]),
            chrono::Utc::now().timestamp() + 60,
        )
        .unwrap();
    let before = store.revalidate(&key.id).unwrap().scope;
    let manifest = connect_connectors::provider_mapping::sip_live_manifest();
    store.register_extension_manifest(&manifest).unwrap();
    store.register_extension_manifest(&manifest).unwrap();
    let after = store.revalidate(&key.id).unwrap().scope;
    assert_eq!(before.operations, after.operations);
    assert_eq!(before.recipients, after.recipients);
    assert_eq!(
        store
            .execute_provisioning(
                &key.id,
                ProvisioningCommand::Identity {
                    operation_id: "manifest-cannot-provision".into(),
                    alias: "forged".into(),
                    kind: IdentityKind::Human,
                    display_name: None,
                }
            )
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let mut drift = manifest.clone();
    let MediaSupport::Live { records, .. } = &mut drift.media else {
        panic!()
    };
    *records = RecordHandling::Unsupported;
    assert_eq!(
        store.register_extension_manifest(&drift).unwrap_err().code,
        "conflict"
    );
}

struct FixtureGate;
impl connect_connector_api::dispatch::Authority for FixtureGate {
    fn current(&self) -> bool {
        true
    }
    fn deadline(&self) -> std::time::Instant {
        std::time::Instant::now() + Duration::from_secs(5)
    }
}
fn fixture_request(
    adapter: &connect_media_connector_template::Adapter,
    operation: &str,
) -> rvoip_core::adapter::OriginateRequest {
    use rvoip_core::adapter::ConnectionAdapter;
    rvoip_core::adapter::OriginateRequest::new(
        rvoip_core::ids::SessionId::from_string("isolated-media-fixture"),
        rvoip_core::ids::ParticipantId::from_string("fixture-speaker"),
        "fixture:duplex",
        rvoip_core::Direction::Outbound,
        adapter.capabilities(),
    )
    .with_transport(rvoip_core::Transport::InProcessAi)
    .with_context(connect_media_connector_template::Admission {
        operation_id: operation.into(),
        permit: connect_connector_api::DispatchPermit::from_host(Arc::new(FixtureGate)),
    })
}
fn composition(
    adapter: Arc<connect_media_connector_template::Adapter>,
) -> connect_server::canonical::CanonicalConnectors {
    connect_server::canonical::CanonicalConnectors {
        media: vec![connect_server::connector_media::LiveTransport::new(
            connect_media_connector_template::manifest(),
            Default::default(),
            adapter,
        )
        .unwrap()],
        ..Default::default()
    }
}

#[tokio::test]
async fn connector_media_owned_startup_registers_exact_adapter_without_authority_and_shutdown_fences_it(
) {
    use rvoip_core::adapter::ConnectionAdapter;
    let f = Fixture::new("EdDSA", 8).await;
    let config = f.config.clone();
    let credential = f.server.store.authenticate(&f.client.credential).unwrap();
    let before = f
        .server
        .store
        .revalidate(&credential.credential_id)
        .unwrap()
        .scope;
    f.server.shutdown().await.unwrap();
    let adapter = connect_media_connector_template::Adapter::open(Arc::new(Default::default()))
        .await
        .unwrap();
    let server =
        RunningCanonicalServer::start_with_connectors(config, composition(adapter.clone()))
            .await
            .unwrap();
    assert_eq!(
        server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .connections,
        0
    );
    assert_eq!(
        server
            .store
            .revalidate(&credential.credential_id)
            .unwrap()
            .scope
            .operations,
        before.operations
    );
    assert_eq!(
        server
            .service
            .install_canonical_transport(adapter.clone())
            .unwrap_err()
            .code,
        "conflict"
    );
    // Real signed peer intake closes startup registration; its credential and
    // installed manifest do not confer access to this simulated media route.
    let mut client_config = f.client;
    client_config.endpoint = server.address;
    let mut client = CanonicalClient::open(client_config).await.unwrap();
    client
        .list_conversations(serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(
        server
            .service
            .install_canonical_transport(adapter.clone())
            .unwrap_err()
            .code,
        "forbidden"
    );
    let h = adapter
        .originate(fixture_request(&adapter, "owned-worker"))
        .await
        .unwrap();
    adapter
        .activate_outbound(h.connection.id.clone())
        .await
        .unwrap();
    let stream = adapter
        .streams(h.connection.id.clone())
        .await
        .unwrap()
        .remove(0);
    let mut input = stream.try_frames_in().unwrap();
    assert!(!input.recv().await.unwrap().payload.is_empty());
    drop(client);
    server.shutdown().await.unwrap();
    assert!(!adapter.is_connection_live(&h.connection.id));
    assert!(!stream.source_ready());
    assert!(adapter
        .originate(fixture_request(&adapter, "after-shutdown"))
        .await
        .is_err());
    let mut buffered = 0;
    while input.recv().await.is_some() {
        buffered += 1;
        assert!(buffered <= connect_media_connector_template::QUEUE_CAPACITY);
    }
}

#[tokio::test]
async fn connector_media_late_http_bind_failure_drains_and_closes_contributed_adapter() {
    use rvoip_core::adapter::ConnectionAdapter;
    let f = Fixture::new("EdDSA", 8).await;
    let mut config = f.config.clone();
    f.server.shutdown().await.unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    config.http_bind = Some(occupied.local_addr().unwrap());
    let adapter = connect_media_connector_template::Adapter::open(Arc::new(Default::default()))
        .await
        .unwrap();
    assert!(
        RunningCanonicalServer::start_with_connectors(config, composition(adapter.clone()))
            .await
            .is_err()
    );
    assert!(adapter
        .originate(fixture_request(&adapter, "after-failure"))
        .await
        .is_err());
    adapter.drain().await.unwrap();
}

#[tokio::test]
async fn connector_media_duplicate_composition_drains_all_owners_and_cancelled_startup_fences_admission(
) {
    use futures_util::FutureExt;
    use rvoip_core::adapter::ConnectionAdapter;
    let f = Fixture::new("EdDSA", 8).await;
    let config = f.config.clone();
    f.server.shutdown().await.unwrap();
    let first = connect_media_connector_template::Adapter::open(Arc::new(Default::default()))
        .await
        .unwrap();
    let second = connect_media_connector_template::Adapter::open(Arc::new(Default::default()))
        .await
        .unwrap();
    let mut composed = composition(first.clone());
    composed.media.extend(composition(second.clone()).media);
    assert!(
        RunningCanonicalServer::start_with_connectors(config.clone(), composed)
            .await
            .is_err()
    );
    for adapter in [&first, &second] {
        assert!(adapter
            .originate(fixture_request(adapter, "after-duplicate"))
            .await
            .is_err());
        adapter.drain().await.unwrap();
    }
    let cancelled = connect_media_connector_template::Adapter::open(Arc::new(Default::default()))
        .await
        .unwrap();
    // Store recovery awaits an owned blocking task; cancellation at this real
    // startup boundary drops the composition guard and requests terminal stop.
    assert!(
        RunningCanonicalServer::start_with_connectors(config, composition(cancelled.clone()))
            .now_or_never()
            .is_none()
    );
    assert!(cancelled
        .originate(fixture_request(&cancelled, "after-cancel"))
        .await
        .is_err());
    cancelled.drain().await.unwrap();
}

struct CleanupSink {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl rvoip_core::adapter::AdapterLifecycleSink for CleanupSink {
    async fn deliver_terminal(&self, _: rvoip_core::adapter::AdapterEvent) {
        self.entered.notify_one();
        self.release.notified().await;
    }
}
#[tokio::test]
async fn connector_media_cleanup_deadline_is_an_error_and_original_worker_can_be_joined_on_retry() {
    use rvoip_core::adapter::ConnectionAdapter;
    let f = Fixture::new("EdDSA", 8).await;
    let mut config = f.config.clone();
    f.server.shutdown().await.unwrap();
    let adapter = connect_media_connector_template::Adapter::open(Arc::new(Default::default()))
        .await
        .unwrap();
    let events = adapter.subscribe_orchestrator_events();
    let sink = Arc::new(CleanupSink {
        entered: Default::default(),
        release: Default::default(),
    });
    adapter.install_lifecycle_sink(sink.clone()).unwrap();
    let h = adapter
        .originate(fixture_request(&adapter, "original-cleanup"))
        .await
        .unwrap();
    adapter
        .activate_outbound(h.connection.id.clone())
        .await
        .unwrap();
    drop(events);
    let mut manifest = connect_media_connector_template::manifest();
    manifest.limits.teardown_ms = 10;
    let transport = connect_server::connector_media::LiveTransport::new(
        manifest,
        Default::default(),
        adapter.clone(),
    )
    .unwrap();
    config.limits.connections = 0; // Refuse startup before installing any routes.
    let error = RunningCanonicalServer::start_with_connectors(
        config,
        connect_server::canonical::CanonicalConnectors {
            media: vec![transport],
            ..Default::default()
        },
    )
    .await
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("cleanup is unconfirmed"));
    assert!(!adapter.is_connection_live(&h.connection.id));
    tokio::time::timeout(Duration::from_secs(1), sink.entered.notified())
        .await
        .unwrap();
    sink.release.notify_one();
    adapter.drain().await.unwrap();
    assert!(adapter
        .originate(fixture_request(&adapter, "after-deadline"))
        .await
        .is_err());
}
