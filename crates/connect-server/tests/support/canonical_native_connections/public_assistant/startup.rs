//! Real TOML startup uses the same bound profile for public discovery and IO.
use super::*;
use connect_client::application::{ApplicationClient, ApplicationOperation};
use connect_protocol::routing::*;
use connect_server::canonical_assistants::{
    CanonicalPublicVapiConfig, CanonicalPublicVapiTarget, ContextTiming,
};
struct Environment(String);
impl Environment {
    fn new() -> Self {
        let name = format!("CONNECT_G6_HOST_{}", uuid::Uuid::new_v4().simple());
        std::env::set_var(&name, "supervised-fixture-token");
        Self(name)
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        std::env::remove_var(&self.0);
    }
}
fn protected(path: &std::path::Path, value: impl AsRef<[u8]>) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, value).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
fn provider_endpoint(f: &Fixture, admin: &str, id: &str, operation: &str) -> String {
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
        .server
        .store
        .execute_provisioning(
            admin,
            ProvisioningCommand::Endpoint {
                operation_id: format!("{operation}-endpoint"),
                identity_id: id.into(),
                device: DeviceSpec {
                    id: format!("{operation}-provider-device"),
                    kind: "service".into(),
                    platform: "designated-startup-fixture".into(),
                },
                transport: EndpointTransport::VapiVoice,
                address: if operation == "unhosted" {
                    "12345678-1234-1234-1234-123456789014"
                } else {
                    "12345678-1234-1234-1234-123456789012"
                }
                .into(),
                capabilities: vec!["audio".into()],
                secret_ref: Some("supervised_fixture_secret".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.server
        .store
        .execute_provisioning(
            admin,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("verify-{operation}"),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "designated-startup-fixture".into(),
            },
        )
        .unwrap();
    endpoint_id
}
fn identity(f: &Fixture, admin: &str, name: &str) -> String {
    let ProvisioningOutcome::Identity { identity_id, .. } = f
        .server
        .store
        .execute_provisioning(
            admin,
            ProvisioningCommand::Identity {
                operation_id: format!("{name}-identity"),
                alias: name.into(),
                kind: IdentityKind::Ai,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    identity_id
}
async fn setup() -> (
    codecs::Running,
    CanonicalClientConfig,
    String,
    String,
    Provider,
    tokio::task::JoinHandle<()>,
    Environment,
) {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let id = identity(&f, &admin, "startup-ai");
    let mut peer = party_with_capabilities(&f, &id, 401, true, vec!["audio".into()]);
    let endpoint = provider_endpoint(&f, &admin, &id, "startup");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let p = Provider {
        base: format!("http://{}", listener.local_addr().unwrap()),
        creates: Arc::new(AtomicUsize::new(0)),
        exports: Arc::new(AtomicUsize::new(0)),
        ends: Arc::new(AtomicUsize::new(0)),
        audio: Arc::new(AtomicUsize::new(0)),
        unconfirmed_end: false,
        record: None,
        record_exports: Arc::new(AtomicUsize::new(0)),
        record_deadline: Arc::new(AtomicI64::new(0)),
    };
    let app = Router::new()
        .route("/call", post(create_call))
        .route("/supervised-public-call/transport", get(upgrade))
        .with_state(p.clone());
    let fixture = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let environment = Environment::new();
    let dir = f.temp.path().join("host");
    std::fs::create_dir(&dir).unwrap();
    protected(&dir.join("native.toml"), toml::to_string(&peer).unwrap());
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        &peer.signing_private_key,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    protected(&dir.join("secrets.toml"),format!("version=1\n[references.supervised_fixture_secret]\nstorage='environment'\nvariable='{}'\n[references.UNSELECTED]\nstorage='environment'\nvariable='{}_UNSELECTED'\n",environment.0,environment.0));
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer: other,
        admin: raw_admin,
        key_revision,
    } = f;
    server.shutdown().await.unwrap();
    config.public_vapi = Some(CanonicalPublicVapiConfig {
        secret_references: "host/secrets.toml".into(),
        targets: vec![CanonicalPublicVapiTarget {
            native_peer: "host/native.toml".into(),
            provider_endpoint: endpoint.clone(),
            secret_ref: "supervised_fixture_secret".into(),
            api_base: format!("{}/", p.base),
            websocket_origin: format!("{}/", p.base.replace("http:", "ws:")),
            loopback_fixture: true,
            model: "designated-startup-model".into(),
            application_prompt: "Use only supplied authorized public history.".into(),
            context_timing: ContextTiming::AfterCreation,
            reachability: ReachabilityPolicy::AllowUnknown,
            observations: None,
        }],
    });
    let path = temp.path().join("canonical-with-host.toml");
    std::fs::write(&path, toml::to_string(&config).unwrap()).unwrap();
    let config = CanonicalServerConfig::load(&path).unwrap();
    assert!(config.public_vapi.as_ref().unwrap().targets[0]
        .native_peer
        .is_absolute());
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    assert!(server.public_assistant_host().is_some());
    client.endpoint = server.address;
    peer.endpoint = server.address;
    let f = Fixture {
        temp,
        config,
        server,
        client,
        peer: other,
        admin: raw_admin,
        key_revision,
    };
    let (running, ()) =
        codecs::setup_started(f, (), "codec-target-offer", "codec-source-offer").await;
    (running, peer, id, endpoint, p, fixture, environment)
}
async fn route(app: &mut ApplicationClient, request: ParticipantRouteRequest) -> ParticipantRoute {
    let Outcome::ParticipantRoute(r) = app
        .execute(ApplicationOperation::try_from(Operation::ParticipantRoute { request }).unwrap())
        .await
        .unwrap()
    else {
        panic!()
    };
    r
}
async fn capabilities(
    app: &mut ApplicationClient,
    cid: &str,
    name: &str,
) -> connect_protocol::Capabilities {
    let Outcome::Capabilities(c) = app
        .execute(
            ApplicationOperation::try_from(Operation::Capabilities {
                address: format!("connect:{name}@local"),
                conversation_id: Some(cid.into()),
            })
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    c
}
async fn add(
    running: &mut codecs::Running,
    id: &str,
    operation: &str,
) -> connect_client::membership::MemberView {
    let room = running.clients[1]
        .list_conversations(Default::default())
        .await
        .unwrap()
        .conversations
        .into_iter()
        .find(|r| r.id == running.cid)
        .unwrap();
    let connect_client::membership::MembershipOutcome::Participant { participant, .. } = running
        .clients[1]
        .participant_change(
            &running.cid,
            operation,
            Some(room.revision),
            payload(
                "conversation.participant.add",
                json!({"identity_id":id,"kind":"ai","role":"participant"}),
            ),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let admin = running
        .f
        .server
        .store
        .authenticate(&running.f.admin)
        .unwrap()
        .credential_id;
    running
        .f
        .server
        .store
        .configure_voice_route(
            &admin,
            &VoiceRoute {
                id: format!("{operation}-voice"),
                conversation_id: running.cid.clone(),
                identity_id: id.into(),
                assistant_id: if operation == "unhosted-member" {
                    "12345678-1234-1234-1234-123456789014"
                } else {
                    "12345678-1234-1234-1234-123456789012"
                }
                .into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550100".into()],
                enabled: true,
            },
        )
        .unwrap();
    participant
}
#[tokio::test]
async fn configured_startup_exposes_only_hosted_endpoint_and_owns_http_accepted_provider_job() {
    case(0).await;
}
#[tokio::test]
async fn configured_host_key_revocation_changes_public_discovery_and_refuses_provider_io() {
    case(1).await;
}
#[tokio::test]
async fn configured_host_ambiguous_native_endpoints_refuse_discovery_and_provider_io() {
    case(2).await;
}
#[tokio::test]
async fn missing_selected_secret_cannot_use_another_reference_or_open_a_second_installation() {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let id = identity(&f, &admin, "secret-failure-ai");
    let peer = party_with_capabilities(&f, &id, 402, true, vec!["audio".into()]);
    let endpoint = provider_endpoint(&f, &admin, &id, "secret-failure");
    let environment = Environment::new();
    let native = f.temp.path().join("secret-failure-native.toml");
    protected(&native, toml::to_string(&peer).unwrap());
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        &peer.signing_private_key,
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let refs = f.temp.path().join("secret-failure-references.toml");
    protected(
        &refs,
        format!(
            "version=1\n[references.OTHER]\nstorage='environment'\nvariable='{}'\n",
            environment.0
        ),
    );
    let mut config = f.config.clone();
    config.public_vapi = Some(CanonicalPublicVapiConfig {
        secret_references: refs,
        targets: vec![CanonicalPublicVapiTarget {
            native_peer: native,
            provider_endpoint: endpoint,
            secret_ref: "supervised_fixture_secret".into(),
            api_base: "http://127.0.0.1:1/".into(),
            websocket_origin: "ws://127.0.0.1:1/".into(),
            loopback_fixture: true,
            model: "designated-fixture".into(),
            application_prompt: "private-config-prompt-canary".into(),
            context_timing: ContextTiming::AfterCreation,
            reachability: ReachabilityPolicy::AllowUnknown,
            observations: None,
        }],
    });
    let error = match RunningCanonicalServer::start(config).await {
        Ok(_) => panic!("missing reference accepted"),
        Err(e) => e,
    };
    let text = format!("{error:#}");
    assert!(text.contains("Selected public assistant secret reference is missing"));
    assert!(!text.contains(&peer.credential));
    assert!(!text.contains("private-config-prompt-canary"));
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    client.keepalive().await.unwrap();
    client.close();
    f.server.shutdown().await.unwrap();
}
async fn case(refusal: u8) {
    let rejected = refusal != 0;
    let (mut running, peer, id, endpoint, p, fixture, _environment) = setup().await;
    let participant = add(&mut running, &id, "startup-member").await;
    let f = &running.f;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let other = identity(f, &admin, "unhosted-ai");
    let _other_endpoint = provider_endpoint(f, &admin, &other, "unhosted");
    let other_participant = add(&mut running, &other, "unhosted-member").await;
    let f = &running.f;
    let cid = &running.cid;
    let mut app = ApplicationClient::open(canonical_http_config(f))
        .await
        .unwrap();
    let request = ParticipantRouteRequest {
        conversation_id: cid.clone(),
        participant_id: participant.participant_id.clone(),
        purpose: RoutePurpose::Session,
        direction: RouteDirection::Outgoing,
        media: vec![RouteMedium::Audio],
        transports: vec![EndpointTransport::VapiVoice],
        preferred_device_id: None,
        failover: FailoverPolicy::None,
        reachability: ReachabilityPolicy::AllowUnknown,
    };
    let selected = route(&mut app, request.clone()).await;
    assert_eq!(selected.reason, RouteReason::AttemptAllowed);
    assert_eq!(selected.selected.unwrap().endpoint_id, endpoint);
    let mut strict = request.clone();
    strict.reachability = ReachabilityPolicy::RequireFresh;
    assert_eq!(route(&mut app, strict).await.reason, RouteReason::Unknown);
    let mut unhosted = request.clone();
    unhosted.participant_id = other_participant.participant_id;
    assert_eq!(
        route(&mut app, unhosted).await.reason,
        RouteReason::Unconfigured
    );
    assert!(capabilities(&mut app, cid, "startup-ai")
        .await
        .routes
        .iter()
        .any(|r| r.id == "vapi-duplex" && r.readiness == "configured"));
    assert!(capabilities(&mut app, cid, "unhosted-ai")
        .await
        .routes
        .iter()
        .all(|r| r.id != "vapi-duplex" || r.readiness == "unconfigured"));
    let mut decision = canonical_http_config(f);
    decision.credential = f
        .server
        .store
        .issue(
            &id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .token;
    let mut ai = CanonicalHttpClient::open(decision).await.unwrap();
    if refusal == 2 {
        let ProvisioningOutcome::Endpoint { endpoint_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Endpoint {
                    operation_id: "ambiguous-host-endpoint".into(),
                    identity_id: id.clone(),
                    device: DeviceSpec {
                        id: peer.device_id.clone(),
                        kind: peer.device_kind.clone(),
                        platform: peer.device_platform.clone(),
                    },
                    transport: EndpointTransport::Quic,
                    address: "quic://localhost:1/ambiguous-host".into(),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        f.server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::VerifyEndpoint {
                    operation_id: "verify-ambiguous-host".into(),
                    endpoint_id,
                    expected_revision: 0,
                    evidence_ref: "designated-startup-fixture".into(),
                },
            )
            .unwrap();
    }
    if refusal == 1 {
        f.server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::RevokeSigningKey {
                    operation_id: "revoke-host-key".into(),
                    key_id: peer.signing_key_id.clone(),
                    expected_revision: 0,
                },
            )
            .unwrap();
    }
    if rejected {
        assert_eq!(
            route(&mut app, request).await.reason,
            RouteReason::PermissionRequired
        );
        assert!(capabilities(&mut app, cid, "startup-ai")
            .await
            .routes
            .iter()
            .all(|r| r.id != "vapi-duplex" || r.readiness == "unconfigured"));
    }
    running.clients[1]
        .send_message(
            "startup-history",
            &running.cid,
            "startup-history",
            connect_client::messages::MessageDraft::text_all("supervised public history marker"),
        )
        .await
        .unwrap();
    let owner = f
        .server
        .store
        .authenticate(&f.client.credential)
        .unwrap()
        .credential_id;
    let session = f
        .server
        .store
        .session_view(&owner, cid, "native-session")
        .unwrap();
    running.clients[1]
        .session_command(Request {
            operation_id: "startup-invite".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(session.topology_revision),
            action: Action::Invite {
                participants: vec![participant.participant_id.clone()],
                medium: "voice".into(),
                intent: session.intent.unwrap(),
                capabilities: session.capabilities_offer.unwrap(),
            },
        })
        .await
        .unwrap();
    let revision = f
        .server
        .store
        .session_view(&owner, cid, "native-session")
        .unwrap()
        .topology_revision;
    let accept = Request {
        operation_id: "startup-accept".into(),
        conversation_id: cid.clone(),
        session_id: "native-session".into(),
        by_participant: Some(participant.participant_id),
        expected_revision: Some(revision),
        action: Action::Accept {
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    ai.session_command(accept.clone()).await.unwrap();
    let host = f.server.public_assistant_host().unwrap().clone();
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        match host.status(cid, "native-session", &id, participant.membership_epoch) {
            Some(AssistantVapiStatus::ContextReady) => {
                assert!(!rejected);
                break;
            }
            Some(AssistantVapiStatus::Stopped { problem }) => {
                assert!(rejected, "startup host: {problem:?}");
                assert_eq!(problem.as_deref(), Some("public-native-peer-failed"));
                break;
            }
            _ => {}
        }
        assert!(
            tokio::time::Instant::now() < until,
            "startup host did not settle"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        ai.session_command(accept.clone()).await.unwrap(),
        ai.session_command(accept).await.unwrap()
    );
    assert_eq!(
        p.creates.load(Ordering::SeqCst),
        u32::from(!rejected) as usize
    );
    let f = running.f;
    f.server.shutdown().await.unwrap();
    assert_eq!(p.ends.load(Ordering::SeqCst), usize::from(!rejected));
    for c in running.clients {
        c.close();
    }
    fixture.abort();
}
