//! Signed native and HTTP Session controls against the owned normalized worker.
use super::*;
use connect_connector_api::{
    lifecycle::*, manifest::ConnectorOperation, Failure, RemoteCall, VoiceConnector, VoiceQuery,
    VoiceStart,
};
use connect_server::canonical_provider_voice::{ProviderVoiceRoute, Worker};
use connect_store::ProviderSessionVoicePolicy;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
#[cfg(unix)]
#[path = "canonical_provider_voice/crash.rs"]
mod crash;
const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";
const PHONE: &str = "12345678-1234-1234-1234-123456789013";
const REMOTE: &str = "12345678-1234-1234-1234-123456789015";
struct Driver {
    starts: AtomicUsize,
    ends: AtomicUsize,
    ended: AtomicBool,
    uncertain: bool,
    no_start: bool,
}
impl Driver {
    fn new(uncertain: bool, no_start: bool) -> Arc<Self> {
        Arc::new(Self {
            starts: AtomicUsize::new(0),
            ends: AtomicUsize::new(0),
            ended: AtomicBool::new(false),
            uncertain,
            no_start,
        })
    }
    fn remote(&self, state: &str) -> RemoteCall {
        RemoteCall {
            id: REMOTE.into(),
            state: state.into(),
            control_url: Some(format!("https://fixture.vapi.ai/{REMOTE}/control")),
            ended_reason: None,
        }
    }
}
#[async_trait::async_trait]
impl VoiceConnector for Driver {
    fn voice_manifest(&self) -> connect_connector_api::manifest::Manifest {
        let mut m = connect_connectors::provider_mapping::vapi_telephone_manifest();
        m.operations.retain(|op| {
            *op != ConnectorOperation::VoiceDiscover
                && *op != ConnectorOperation::VoiceReconcile
                && (!self.no_start || *op != ConnectorOperation::VoiceStart)
        });
        m
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        connect_connectors::provider_mapping::references("private-key", "OFFLINE_FIXTURE")
    }
    fn normalize(&self, id: &str, call: &RemoteCall) -> std::result::Result<Observation, Failure> {
        Ok(Observation {
            mutation_id: id.into(),
            external_id: Some(call.id.clone()),
            event_id: format!("{}:{}", call.id, call.state),
            state: State::Voice(connect_connectors::provider_mapping::vapi_voice_state(
                &call.state,
            )),
            verification: Verification::AuthenticatedResponse,
        })
    }
    async fn inbound(
        &self,
        _: &VoiceQuery,
    ) -> std::result::Result<Vec<connect_connector_api::InboundCall>, Failure> {
        panic!("VoiceDiscover was not declared")
    }
    async fn start(&self, r: &VoiceStart) -> std::result::Result<RemoteCall, Failure> {
        assert_eq!(r.assistant_id, ASSISTANT);
        assert_eq!(r.phone_number_id, PHONE);
        assert_eq!(r.destination, "+15555550111");
        self.starts.fetch_add(1, Ordering::SeqCst);
        if self.uncertain {
            Err(Failure::Uncertain)
        } else {
            Ok(self.remote("queued"))
        }
    }
    async fn status(&self, id: &str) -> std::result::Result<RemoteCall, Failure> {
        assert_eq!(id, REMOTE);
        Ok(self.remote(if self.ended.load(Ordering::SeqCst) {
            "ended"
        } else {
            "in-progress"
        }))
    }
    async fn end(
        &self,
        id: &str,
        control: Option<&str>,
    ) -> std::result::Result<RemoteCall, Failure> {
        assert_eq!(id, REMOTE);
        assert_eq!(
            control,
            Some(format!("https://fixture.vapi.ai/{REMOTE}/control").as_str())
        );
        self.ends.fetch_add(1, Ordering::SeqCst);
        self.ended.store(true, Ordering::SeqCst);
        Ok(self.remote("ended"))
    }
}
fn endpoint(
    f: &Fixture,
    key: &str,
    id: &str,
    name: &str,
    transport: EndpointTransport,
    address: &str,
    secret: bool,
) -> String {
    let store = &f.server.store;
    let ProvisioningOutcome::Endpoint { endpoint_id, .. } = store
        .execute_provisioning(
            key,
            ProvisioningCommand::Endpoint {
                operation_id: format!("{name}-enroll"),
                identity_id: id.into(),
                device: DeviceSpec {
                    id: format!("{name}-device"),
                    kind: "service".into(),
                    platform: "isolated-provider-fixture".into(),
                },
                transport,
                address: address.into(),
                capabilities: vec![if secret { "audio" } else { "data" }.into()],
                secret_ref: secret.then(|| "OFFLINE_FIXTURE".into()),
            },
        )
        .unwrap()
    else {
        panic!()
    };
    store
        .execute_provisioning(
            key,
            ProvisioningCommand::VerifyEndpoint {
                operation_id: format!("{name}-verify"),
                endpoint_id: endpoint_id.clone(),
                expected_revision: 0,
                evidence_ref: "isolated-fixture".into(),
            },
        )
        .unwrap();
    endpoint_id
}
async fn setup() -> (Fixture, ProviderSessionVoicePolicy) {
    setup_for("EdDSA").await
}
async fn setup_for(algorithm: &str) -> (Fixture, ProviderSessionVoicePolicy) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("connect_server::canonical_provider_voice=debug")
        .with_test_writer()
        .try_init();
    let mut f = Fixture::with_membership_scope(algorithm, 12, true).await;
    let store = &f.server.store;
    let admin = store.authenticate(&f.admin).unwrap().credential_id;
    let ProvisioningOutcome::Identity {
        identity_id: ai, ..
    } = store
        .execute_provisioning(
            &admin,
            ProvisioningCommand::Identity {
                operation_id: "provider-ai".into(),
                alias: "provider-ai".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let key = store
        .issue(&ai, Scope::operator(), chrono::Utc::now().timestamp() + 600)
        .unwrap();
    let phone = store
        .issue(
            &f.peer.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let ai_endpoint = endpoint(
        &f,
        &key.id,
        &ai,
        "provider-ai",
        EndpointTransport::VapiVoice,
        ASSISTANT,
        true,
    );
    let phone_endpoint = endpoint(
        &f,
        &key.id,
        &f.peer.identity_id,
        "provider-phone",
        EndpointTransport::Telephone,
        "+15555550111",
        true,
    );
    endpoint(
        &f,
        &key.id,
        &ai,
        "provider-native",
        EndpointTransport::Quic,
        &format!(
            "quic://localhost:{}/provider-native",
            f.server.address.port()
        ),
        false,
    );
    let private = f.temp.path().join("provider-ai.der");
    let public = signing(&private, algorithm);
    store
        .execute_provisioning(
            &key.id,
            ProvisioningCommand::SigningKey {
                operation_id: "provider-ai-key".into(),
                identity_id: ai.clone(),
                credential_id: key.id.clone(),
                device_id: "provider-native-device".into(),
                key_id: "provider-ai-key".into(),
                algorithm: if algorithm == "ES256" {
                    SigningAlgorithm::ES256
                } else {
                    SigningAlgorithm::EdDSA
                },
                public_key: URL_SAFE_NO_PAD.encode(public),
                not_before: chrono::Utc::now().timestamp() - 1,
                expires_at: chrono::Utc::now().timestamp() + 600,
                expected_revision: None,
            },
        )
        .unwrap();
    f.client.identity_id = ai;
    f.client.credential = key.token;
    f.client.device_id = "provider-native-device".into();
    f.client.device_kind = "service".into();
    f.client.device_platform = "isolated-provider-fixture".into();
    f.client.signing_key_id = "provider-ai-key".into();
    f.client.signing_private_key = private;
    let policy = ProviderSessionVoicePolicy {
        endpoint_id: phone_endpoint,
        endpoint_credential_id: phone.id,
        route_credential_id: key.id.clone(),
        route_reference: "OFFLINE_FIXTURE".into(),
        provider: "vapi-telephone".into(),
        assistant_id: ASSISTANT.into(),
        assistant_endpoint_id: ai_endpoint,
        assistant_credential_id: key.id,
        phone_number_id: PHONE.into(),
        timeout_seconds: 30,
    };
    (f, policy)
}
async fn invite(
    f: &Fixture,
    client: &mut CanonicalClient,
) -> (connect_protocol::session_control::Request, String) {
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("provider-room", input)
        .await
        .unwrap();
    let baseline = client
        .subscribe_conversation(
            &room.id,
            serde_json::from_value(json!({"live":false})).unwrap(),
        )
        .await
        .unwrap();
    let cursor = baseline.ack().unwrap().details.as_ref().unwrap()["cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let part = |identity: &str| {
        room.participants
            .iter()
            .find(|p| p.identity_id == identity)
            .unwrap()
            .participant_id
            .clone()
    };
    let r = connect_protocol::session_control::Request {
        operation_id: "provider-invite".into(),
        conversation_id: room.id,
        session_id: "provider-session".into(),
        by_participant: Some(part(&f.client.identity_id)),
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![part(&f.peer.identity_id)],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    client.session_command(r.clone()).await.unwrap();
    (r, cursor)
}
async fn view(
    client: &mut CanonicalClient,
    cid: &str,
    want: connect_protocol::events::ProviderCallState,
) -> connect_protocol::events::ProviderSessionVoiceView {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let snapshot = client
            .conversation_snapshot(cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains(REMOTE));
        assert!(!encoded.contains("/control"));
        assert!(!snapshot.items.iter().any(|v| v["kind"] == "connection"));
        if let Some(value) = snapshot.items.into_iter().find_map(|v| {
            let item: connect_protocol::events::SnapshotItem = serde_json::from_value(v).unwrap();
            match item {
                connect_protocol::events::SnapshotItem::ProviderCall(c) if c.state == want => {
                    Some(c)
                }
                _ => None,
            }
        }) {
            assert!(!value.local_media);
            return value;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing provider state {want:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn quic_http_replay_starts_once_and_end_reaches_journal() {
    let (f, policy) = setup().await;
    let driver = Driver::new(false, false);
    let worker = Worker::with_routes(
        &f.server.service,
        vec![ProviderVoiceRoute {
            policy,
            driver: driver.clone(),
            automatic_outbound: true,
        }],
    )
    .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (r, cursor) = invite(&f, &mut native).await;
    view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    let mut http = connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let receipt = http.session_command(r.clone()).await.unwrap();
    assert_eq!(receipt, native.session_command(r.clone()).await.unwrap());
    assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
    let mut end = r.clone();
    end.operation_id = "provider-end".into();
    end.expected_revision = Some(match &receipt {
        Outcome::SessionControl(control) => control.session.topology_revision,
        _ => panic!("Session receipt"),
    });
    end.action = connect_protocol::session_control::Action::End {
        reason_code: 200,
        reason: "done".into(),
    };
    http.session_command(end).await.unwrap();
    let ended = view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    assert!(ended.closed_at.is_some());
    let journal = native
        .subscribe_conversation(
            &r.conversation_id,
            serde_json::from_value(json!({"after":cursor,"live":false})).unwrap(),
        )
        .await
        .unwrap();
    let facts: Vec<_> = journal
        .frames
        .iter()
        .filter(|f| f.kind() == "conversation.event")
        .map(|f| f.original().clone())
        .collect();
    let encoded = serde_json::to_string(&facts).unwrap();
    assert!(encoded.contains("provider_call"));
    assert!(encoded.contains("ended"));
    assert!(!encoded.contains(REMOTE));
    assert!(!encoded.contains("/control"));
    assert!(!encoded.contains("connection_ready"));
    worker.shutdown().await.unwrap();
    assert_eq!(driver.ends.load(Ordering::SeqCst), 1);
    native.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncertain_provider_start_is_never_retried_after_worker_restart() {
    let (f, policy) = setup().await;
    let driver = Driver::new(true, false);
    let route = ProviderVoiceRoute {
        policy,
        driver: driver.clone(),
        automatic_outbound: true,
    };
    let worker = Worker::with_routes(&f.server.service, vec![route.clone()]).unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (r, _cursor) = invite(&f, &mut native).await;
    view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Unknown,
    )
    .await;
    assert_eq!(
        worker.shutdown().await.unwrap_err().code,
        "reconciliation-required"
    );
    let restarted = Worker::with_routes(&f.server.service, vec![route]).unwrap();
    native.session_command(r.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
    view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Unknown,
    )
    .await;
    assert_eq!(
        restarted.shutdown().await.unwrap_err().code,
        "reconciliation-required"
    );
    native.close();
    assert!(f.server.shutdown().await.is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_without_voice_start_refuses_before_provider_io() {
    let (f, policy) = setup().await;
    let driver = Driver::new(false, true);
    let worker = Worker::with_routes(
        &f.server.service,
        vec![ProviderVoiceRoute {
            policy,
            driver: driver.clone(),
            automatic_outbound: true,
        }],
    )
    .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (r, _cursor) = invite(&f, &mut native).await;
    let failed = view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Failed,
    )
    .await;
    assert!(failed
        .problem
        .as_deref()
        .is_some_and(|p| p.starts_with("unsupported")));
    assert_eq!(driver.starts.load(Ordering::SeqCst), 0);
    assert_eq!(driver.ends.load(Ordering::SeqCst), 0);
    worker.shutdown().await.unwrap();
    native.close();
    f.server.shutdown().await.unwrap();
}

struct IncomingDriver {
    control: Arc<Driver>,
    scans: AtomicUsize,
    created_at: std::sync::OnceLock<i64>,
}
#[async_trait::async_trait]
impl VoiceConnector for IncomingDriver {
    fn voice_manifest(&self) -> connect_connector_api::manifest::Manifest {
        let mut m = self.control.voice_manifest();
        m.operations.insert(ConnectorOperation::VoiceDiscover);
        m
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        self.control.voice_configuration()
    }
    fn normalize(&self, id: &str, call: &RemoteCall) -> std::result::Result<Observation, Failure> {
        self.control.normalize(id, call)
    }
    async fn inbound(
        &self,
        q: &VoiceQuery,
    ) -> std::result::Result<Vec<connect_connector_api::InboundCall>, Failure> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        let call = self
            .control
            .remote(if self.control.ended.load(Ordering::SeqCst) {
                "ended"
            } else {
                "in-progress"
            });
        Ok(vec![connect_connector_api::InboundCall {
            provider_id: call.id,
            assistant_id: q.assistant_id.clone(),
            phone_number_id: q.phone_number_id.clone(),
            caller: "+15555550111".into(),
            // One immutable provider resource observed by all ingress queries.
            created_at: *self
                .created_at
                .get_or_init(|| chrono::Utc::now().timestamp()),
            state: call.state,
            control_url: call.control_url,
            ended_reason: None,
        }])
    }
    async fn start(&self, _: &VoiceStart) -> std::result::Result<RemoteCall, Failure> {
        panic!("incoming observation cannot dial")
    }
    async fn status(&self, id: &str) -> std::result::Result<RemoteCall, Failure> {
        self.control.status(id).await
    }
    async fn end(&self, id: &str, url: Option<&str>) -> std::result::Result<RemoteCall, Failure> {
        self.control.end(id, url).await
    }
}
async fn incoming_setup(
    no_start: bool,
) -> (
    Fixture,
    ProviderVoiceRoute,
    connect_store::ProviderVoiceIngressPolicy,
    Arc<IncomingDriver>,
    String,
) {
    let (f, policy) = setup().await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = client
        .create_conversation("incoming-provider-room", input)
        .await
        .unwrap();
    client.close();
    let driver = Arc::new(IncomingDriver {
        control: Driver::new(false, no_start),
        scans: AtomicUsize::new(0),
        created_at: std::sync::OnceLock::new(),
    });
    let incoming = connect_store::ProviderVoiceIngressPolicy {
        id: "incoming-provider".into(),
        conversation_id: room.id.clone(),
        policy: policy.clone(),
    };
    (
        f,
        ProviderVoiceRoute {
            policy,
            driver: driver.clone(),
            automatic_outbound: no_start,
        },
        incoming,
        driver,
        room.id,
    )
}
async fn provider_session_revision(client: &mut CanonicalClient, cid: &str, sid: &str) -> u64 {
    let snapshot = client
        .conversation_snapshot(cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    snapshot
        .items
        .into_iter()
        .find_map(|item| {
            match serde_json::from_value::<connect_protocol::events::SnapshotItem>(item).unwrap() {
                connect_protocol::events::SnapshotItem::Session(s) if s.id == sid => {
                    Some(s.topology_revision)
                }
                _ => None,
            }
        })
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_inbound_rooms_remain_private_across_scanners_and_worker_restart() {
    let (f, route, incoming, driver, first_cid) = incoming_setup(false).await;
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let second = client
        .create_conversation("overlapping-provider-room", input)
        .await
        .unwrap();
    let other = connect_store::ProviderVoiceIngressPolicy {
        id: "overlapping-provider".into(),
        conversation_id: second.id.clone(),
        policy: incoming.policy.clone(),
    };
    let worker = Worker::with_routes_and_ingress(
        &f.server.service,
        vec![route.clone()],
        vec![incoming.clone(), other],
    )
    .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let held = loop {
        let contacts = f
            .server
            .store
            .incoming_contacts(&route.policy.route_credential_id, None, 32)
            .unwrap();
        if driver.scans.load(Ordering::SeqCst) >= 2 && contacts.contacts.len() == 1 {
            break contacts.contacts[0].clone();
        }
        assert!(
            tokio::time::Instant::now() < until,
            "both owned scanners must reach private custody"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        held.reason,
        connect_protocol::incoming_contacts::IncomingContactReason::AmbiguousRoute
    );
    assert_eq!(held.candidates.len(), 2);
    let unrelated = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    assert!(f
        .server
        .store
        .incoming_contact(&unrelated, &held.id)
        .is_err());
    for cid in [&first_cid, &second.id] {
        let snapshot = client
            .conversation_snapshot(cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert!(!snapshot.items.iter().any(|item| matches!(
            item["kind"].as_str(),
            Some("session" | "provider_voice" | "session_participant")
        )));
    }
    worker.shutdown().await.unwrap();
    assert_eq!(driver.control.starts.load(Ordering::SeqCst), 0);
    assert_eq!(driver.control.ends.load(Ordering::SeqCst), 0);
    let scans = driver.scans.load(Ordering::SeqCst);
    let restarted =
        Worker::with_routes_and_ingress(&f.server.service, vec![route.clone()], vec![incoming])
            .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while driver.scans.load(Ordering::SeqCst) == scans {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        f.server
            .store
            .incoming_contact(&route.policy.route_credential_id, &held.id)
            .unwrap(),
        held
    );
    for cid in [&first_cid, &second.id] {
        let snapshot = client
            .conversation_snapshot(cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert!(!snapshot.items.iter().any(|item| matches!(
            item["kind"].as_str(),
            Some("session" | "provider_voice" | "session_participant")
        )));
    }
    restarted.shutdown().await.unwrap();
    client.close();
    f.server.shutdown().await.unwrap();
    assert_eq!(driver.control.starts.load(Ordering::SeqCst), 0);
    assert_eq!(driver.control.ends.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_admin_voice_selection_reaches_owned_origin_and_one_native_room() {
    use connect_client::application::{ApplicationClient, ApplicationClientConfig};
    use connect_protocol::incoming_contacts::*;
    let (f, route, incoming, driver, first_cid) = incoming_setup(false).await;
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let second = native
        .create_conversation("selected-incoming-room", input)
        .await
        .unwrap();
    let other = connect_store::ProviderVoiceIngressPolicy {
        id: "selected-incoming".into(),
        conversation_id: second.id.clone(),
        policy: incoming.policy.clone(),
    };
    let worker =
        Worker::with_routes_and_ingress(&f.server.service, vec![route], vec![incoming, other])
            .unwrap();
    let config = ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: f.client.credential.clone(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 5000,
    };
    let mut admin = ApplicationClient::open(config.clone()).await.unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let held = loop {
        let IncomingContactOutcome::Page { page } = admin
            .incoming_contact_command(IncomingContactCommand::List {
                after: None,
                limit: 32,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        if let Some(contact) = page.contacts.into_iter().next() {
            break contact;
        }
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(held.candidates.len(), 2);
    let raw_http = reqwest::Client::new();
    let raw_url = format!("http://{}/v2/incoming-contacts", config.endpoint);
    let request = json!({"command":"get","contact_id":held.id});
    let (status, _) = super::incoming_contact_http::record(
        "voice-held-get",
        request.clone(),
        raw_http
            .post(&raw_url)
            .bearer_auth(&config.credential)
            .json(&request)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let selected = held
        .candidates
        .iter()
        .find(|c| c.conversation_id == second.id)
        .unwrap();
    let command = IncomingContactCommand::Resolve(IncomingContactSelection {
        operation_id: "public-voice-choice".into(),
        contact_id: held.id.clone(),
        candidate_id: selected.id.clone(),
        expected_revision: 0,
    });
    let IncomingContactOutcome::Contact { contact } = admin
        .incoming_contact_command(command.clone())
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(contact.state, IncomingContactState::Resolved);
    let chosen = view(
        &mut native,
        &second.id,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    assert_eq!(
        contact.decision.as_ref().unwrap().session_id.as_deref(),
        Some(chosen.session_id.as_str())
    );
    assert!(!chosen.remote_identity_verified);
    assert!(!chosen.local_media);
    assert_eq!(
        admin
            .incoming_contact_command(command.clone())
            .await
            .unwrap(),
        IncomingContactOutcome::Contact {
            contact: contact.clone()
        }
    );
    let request = serde_json::to_value(&command).unwrap();
    let (status, _) = super::incoming_contact_http::record(
        "voice-resolve-replay",
        request.clone(),
        raw_http
            .post(&raw_url)
            .bearer_auth(&config.credential)
            .json(&request)
            .send()
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let mut conflicting = match command {
        IncomingContactCommand::Resolve(s) => s,
        _ => unreachable!(),
    };
    conflicting.candidate_id = held
        .candidates
        .iter()
        .find(|c| c.conversation_id == first_cid)
        .unwrap()
        .id
        .clone();
    assert!(
        matches!(admin.incoming_contact_command(IncomingContactCommand::Resolve(conflicting)).await,Err(connect_client::ClientError::Application(error)) if error.code=="conflict")
    );
    let other = native
        .conversation_snapshot(&first_cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert!(!other
        .items
        .iter()
        .any(|item| matches!(item["kind"].as_str(), Some("session" | "provider_voice"))));
    let raw = serde_json::to_string(&contact).unwrap();
    assert!(!raw.contains(REMOTE));
    assert!(!raw.contains("/control"));
    let http = reqwest::Client::new();
    let url = format!("http://{}/v2/incoming-contacts", config.endpoint);
    assert_eq!(
        http.post(&url).body("{").send().await.unwrap().status(),
        401
    );
    assert_eq!(
        http.post(&url)
            .bearer_auth(&config.credential)
            .header("origin", "http://localhost")
            .json(&IncomingContactCommand::Get {
                contact_id: held.id.clone()
            })
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(&url)
            .bearer_auth(&config.credential)
            .json(&json!({"command":"get","contact_id":held.id,"body":"spoof"}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let mut wrong = ApplicationClient::open(ApplicationClientConfig {
        credential: f.admin.clone(),
        ..config
    })
    .await
    .unwrap();
    assert!(
        matches!(wrong.incoming_contact_command(IncomingContactCommand::Get {contact_id:held.id}).await,Err(connect_client::ClientError::Application(error)) if error.code=="forbidden")
    );
    assert_eq!(driver.control.starts.load(Ordering::SeqCst), 0);
    worker.shutdown().await.unwrap();
    assert_eq!(driver.control.ends.load(Ordering::SeqCst), 1);
    native.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn inbound_discovery_reaches_native_sdk_and_restart_never_redials() {
    let (f, route, incoming, driver, cid) = incoming_setup(false).await;
    let worker = Worker::with_routes_and_ingress(
        &f.server.service,
        vec![route.clone()],
        vec![incoming.clone()],
    )
    .unwrap();
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let first = view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    assert_eq!(
        first.direction,
        connect_protocol::events::ProviderCallDirection::Inbound
    );
    assert!(!first.remote_identity_verified);
    let snapshot = client
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert!(snapshot
        .items
        .iter()
        .any(|item| item["kind"] == "session_participant"
            && item["data"]["participant_id"] == first.participant_id
            && item["data"]["state"] == "invited"));
    worker.shutdown().await.unwrap();
    assert_eq!(driver.control.ends.load(Ordering::SeqCst), 1);
    let scans = driver.scans.load(Ordering::SeqCst);
    let restarted =
        Worker::with_routes_and_ingress(&f.server.service, vec![route], vec![incoming]).unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while driver.scans.load(Ordering::SeqCst) <= scans {
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let ended = view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    assert_eq!(ended.id, first.id);
    let snapshot = client
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert_eq!(
        snapshot
            .items
            .iter()
            .filter(|item| item["kind"] == "provider_call")
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .items
            .iter()
            .filter(|item| item["kind"] == "session")
            .count(),
        1
    );
    restarted.shutdown().await.unwrap();
    assert_eq!(driver.control.starts.load(Ordering::SeqCst), 0);
    assert_eq!(driver.control.ends.load(Ordering::SeqCst), 1);
    client.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn normalized_voice_without_start_refuses_creation_but_retains_inbound_status_and_cleanup() {
    let (f, route, incoming, driver, cid) = incoming_setup(true).await;
    let worker =
        Worker::with_routes_and_ingress(&f.server.service, vec![route], vec![incoming]).unwrap();
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let call = view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    let snapshot = client
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let phone = snapshot
        .items
        .iter()
        .find(|item| {
            item["kind"] == "participant" && item["data"]["identity_id"] == f.peer.identity_id
        })
        .unwrap()["data"]["participant_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let outbound = connect_protocol::session_control::Request {
        operation_id: "incoming-profile-outbound-refusal".into(),
        conversation_id: cid.clone(),
        session_id: "incoming-profile-outbound".into(),
        by_participant: None,
        expected_revision: None,
        action: connect_protocol::session_control::Action::Invite {
            participants: vec![phone],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    };
    client.session_command(outbound).await.unwrap();
    let refused = view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Failed,
    )
    .await;
    assert_eq!(
        refused.problem.as_deref(),
        Some("unsupported-connector-operation")
    );
    let revision = provider_session_revision(&mut client, &cid, &call.session_id).await;
    let mut http = connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    http.session_command(connect_protocol::session_control::Request {
        operation_id: "incoming-provider-end".into(),
        conversation_id: cid.clone(),
        session_id: call.session_id,
        by_participant: None,
        expected_revision: Some(revision),
        action: connect_protocol::session_control::Action::End {
            reason_code: 200,
            reason: "done".into(),
        },
    })
    .await
    .unwrap();
    view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    worker.shutdown().await.unwrap();
    assert_eq!(driver.control.starts.load(Ordering::SeqCst), 0);
    assert_eq!(driver.control.ends.load(Ordering::SeqCst), 1);
    client.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn normalized_voice_without_discovery_does_not_call_inbound_but_retains_outbound_control() {
    let (f, profile, incoming, _unused, cid) = incoming_setup(false).await;
    let driver = Driver::new(false, false);
    let worker = Worker::with_routes_and_ingress(
        &f.server.service,
        vec![ProviderVoiceRoute {
            policy: profile.policy,
            driver: driver.clone(),
            automatic_outbound: true,
        }],
        vec![incoming],
    )
    .unwrap();
    let mut client = CanonicalClient::open(f.client.clone()).await.unwrap();
    let snapshot = client
        .conversation_snapshot(&cid, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let phone = snapshot
        .items
        .iter()
        .find(|item| {
            item["kind"] == "participant" && item["data"]["identity_id"] == f.peer.identity_id
        })
        .unwrap()["data"]["participant_id"]
        .as_str()
        .unwrap()
        .to_owned();
    client
        .session_command(connect_protocol::session_control::Request {
            operation_id: "no-discovery-outbound".into(),
            conversation_id: cid.clone(),
            session_id: "no-discovery-session".into(),
            by_participant: None,
            expected_revision: None,
            action: connect_protocol::session_control::Action::Invite {
                participants: vec![phone],
                medium: "voice".into(),
                intent: "call".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let call = view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    assert_eq!(
        call.direction,
        connect_protocol::events::ProviderCallDirection::Outbound
    );
    let revision = provider_session_revision(&mut client, &cid, &call.session_id).await;
    client
        .session_command(connect_protocol::session_control::Request {
            operation_id: "no-discovery-end".into(),
            conversation_id: cid.clone(),
            session_id: call.session_id,
            by_participant: None,
            expected_revision: Some(revision),
            action: connect_protocol::session_control::Action::End {
                reason_code: 200,
                reason: "done".into(),
            },
        })
        .await
        .unwrap();
    view(
        &mut client,
        &cid,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    worker.shutdown().await.unwrap();
    assert_eq!(driver.starts.load(Ordering::SeqCst), 1);
    assert_eq!(driver.ends.load(Ordering::SeqCst), 1);
    client.close();
    f.server.shutdown().await.unwrap();
}

struct CorrelatedDriver {
    inner: Arc<Driver>,
    ready: AtomicBool,
    queries: AtomicUsize,
    original: std::sync::Mutex<Option<String>>,
}
#[async_trait::async_trait]
impl VoiceConnector for CorrelatedDriver {
    fn voice_manifest(&self) -> connect_connector_api::manifest::Manifest {
        let mut m = self.inner.voice_manifest();
        m.operations.insert(ConnectorOperation::VoiceReconcile);
        m
    }
    fn voice_configuration(&self) -> std::collections::BTreeMap<String, String> {
        self.inner.voice_configuration()
    }
    fn normalize(&self, id: &str, call: &RemoteCall) -> std::result::Result<Observation, Failure> {
        self.inner.normalize(id, call)
    }
    async fn start_receipt(
        &self,
        id: &str,
        r: &VoiceStart,
    ) -> std::result::Result<connect_connector_api::VoiceReceipt, Failure> {
        {
            let mut original = self.original.lock().unwrap();
            assert!(original.is_none());
            *original = Some(id.into());
        }
        // The fixture resource exists, but the original create response is lost.
        self.inner.start(r).await?;
        unreachable!()
    }
    async fn reconcile_receipt(
        &self,
        id: &str,
        q: &connect_connector_api::VoiceReconcileQuery,
    ) -> std::result::Result<connect_connector_api::VoiceReceipt, Failure> {
        assert_eq!(self.original.lock().unwrap().as_deref(), Some(id));
        assert_eq!(q.request.assistant_id, ASSISTANT);
        assert_eq!(q.request.phone_number_id, PHONE);
        assert_eq!(q.request.destination, "+15555550111");
        assert_eq!(q.until - q.since, 150);
        self.queries.fetch_add(1, Ordering::SeqCst);
        if !self.ready.load(Ordering::SeqCst) {
            return Err(Failure::Uncertain);
        }
        let call = self.inner.status(REMOTE).await?;
        let mut observation = self.normalize(id, &call)?;
        observation.verification = Verification::Reconciliation;
        observation.event_id = format!("correlated:{}", observation.event_id);
        Ok(connect_connector_api::VoiceReceipt { call, observation })
    }
    async fn inbound(
        &self,
        q: &VoiceQuery,
    ) -> std::result::Result<Vec<connect_connector_api::InboundCall>, Failure> {
        self.inner.inbound(q).await
    }
    async fn start(&self, _: &VoiceStart) -> std::result::Result<RemoteCall, Failure> {
        panic!("original creation must retain mutation correlation")
    }
    async fn status(&self, id: &str) -> std::result::Result<RemoteCall, Failure> {
        self.inner.status(id).await
    }
    async fn end(&self, id: &str, url: Option<&str>) -> std::result::Result<RemoteCall, Failure> {
        self.inner.end(id, url).await
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncertain_creation_reconciles_after_restart_and_http_end_cleans_the_original_resource_once(
) {
    let (f, policy) = setup().await;
    let driver = Arc::new(CorrelatedDriver {
        inner: Driver::new(true, false),
        ready: AtomicBool::new(false),
        queries: AtomicUsize::new(0),
        original: std::sync::Mutex::new(None),
    });
    let route = ProviderVoiceRoute {
        policy,
        driver: driver.clone(),
        automatic_outbound: true,
    };
    let worker = Worker::with_routes(&f.server.service, vec![route.clone()]).unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (r, cursor) = invite(&f, &mut native).await;
    let unknown = view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Unknown,
    )
    .await;
    assert_eq!(
        worker.shutdown().await.unwrap_err().code,
        "reconciliation-required"
    );
    assert_eq!(driver.inner.ends.load(Ordering::SeqCst), 0);
    driver.ready.store(true, Ordering::SeqCst);
    let restarted = Worker::with_routes(&f.server.service, vec![route]).unwrap();
    let active = view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    assert_eq!(unknown.id, active.id);
    assert_eq!(unknown.operation_id, active.operation_id);
    assert_eq!(active.operation_id, r.operation_id);
    let receipt = native.session_command(r.clone()).await.unwrap();
    let mut http = connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let revision = match receipt {
        Outcome::SessionControl(control) => control.session.topology_revision,
        _ => panic!("Session receipt"),
    };
    http.session_command(connect_protocol::session_control::Request {
        operation_id: "correlated-end".into(),
        expected_revision: Some(revision),
        action: connect_protocol::session_control::Action::End {
            reason_code: 200,
            reason: "done".into(),
        },
        ..r.clone()
    })
    .await
    .unwrap();
    view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    let events = native
        .subscribe_conversation(
            &r.conversation_id,
            serde_json::from_value(json!({"after":cursor,"live":false})).unwrap(),
        )
        .await
        .unwrap();
    let facts: Vec<_> = events
        .frames
        .iter()
        .filter(|f| f.kind() == "conversation.event")
        .map(|f| f.original().clone())
        .collect();
    let encoded = serde_json::to_string(&facts).unwrap();
    assert!(encoded.contains("provider_voice"));
    assert!(!encoded.contains(REMOTE));
    assert!(!encoded.contains("/control"));
    restarted.shutdown().await.unwrap();
    assert_eq!(driver.inner.starts.load(Ordering::SeqCst), 1);
    assert_eq!(driver.inner.ends.load(Ordering::SeqCst), 1);
    assert!(driver.queries.load(Ordering::SeqCst) >= 2);
    native.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_vapi_sdk_reconciles_lost_creation_after_restart_without_another_post_or_invented_session_acceptance(
) {
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    let (f, policy) = setup().await;
    let provider = MockServer::start().await;
    let original = Arc::new(std::sync::Mutex::new(None::<String>));
    let indexed = Arc::new(AtomicBool::new(false));
    let captured = original.clone();
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path("/call"))
        .respond_with(move |r: &wiremock::Request| {
            let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            let mut original = captured.lock().unwrap();
            assert!(original.is_none(), "provider create was retried");
            *original = Some(v["name"].as_str().unwrap().into());
            ResponseTemplate::new(503)
        })
        .mount(&provider)
        .await;
    let make_row = {
        let original = original.clone();
        move || json!({"id":REMOTE,"name":original.lock().unwrap().clone().unwrap(),"type":"outboundPhoneCall","assistantId":ASSISTANT,"phoneNumberId":PHONE,"customer":{"number":"+15555550111"},"createdAt":chrono::Utc::now().to_rfc3339(),"status":"ended","endedReason":"customer-ended-call"})
    };
    let list_row = make_row.clone();
    let listed = indexed.clone();
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path("/call"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(if listed.load(Ordering::SeqCst) {
                vec![list_row()]
            } else {
                vec![]
            })
        })
        .mount(&provider)
        .await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path(format!("/call/{REMOTE}")))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(make_row())
        })
        .mount(&provider)
        .await;
    let driver = Arc::new(
        connect_connectors::Vapi::with_reference(
            "isolated-fixture-key",
            &provider.uri(),
            "OFFLINE_FIXTURE",
        )
        .unwrap(),
    );
    let route = ProviderVoiceRoute {
        policy,
        driver,
        automatic_outbound: true,
    };
    let worker = Worker::with_routes(&f.server.service, vec![route.clone()]).unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (r, _) = invite(&f, &mut native).await;
    let unknown = view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Unknown,
    )
    .await;
    let original_receipt = native.session_command(r.clone()).await.unwrap();
    assert_eq!(
        worker.shutdown().await.unwrap_err().code,
        "reconciliation-required"
    );
    indexed.store(true, Ordering::SeqCst);
    let restarted = Worker::with_routes(&f.server.service, vec![route]).unwrap();
    let ended = view(
        &mut native,
        &r.conversation_id,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    assert_eq!(unknown.id, ended.id);
    assert!(ended.closed_at.is_some());
    assert_eq!(ended.operation_id, r.operation_id);
    assert_eq!(
        native.session_command(r.clone()).await.unwrap(),
        original_receipt
    );
    restarted.shutdown().await.unwrap();
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        1
    );
    assert!(requests
        .iter()
        .any(|r| r.url.path() == format!("/call/{REMOTE}")));
    assert!(requests
        .iter()
        .all(|r| r.url.path() == "/call" || r.url.path() == format!("/call/{REMOTE}")));
    native.close();
    f.server.shutdown().await.unwrap();
}

async fn session_provider_view(
    client: &mut CanonicalClient,
    cid: &str,
    sid: &str,
    want: connect_protocol::events::ProviderCallState,
) -> connect_protocol::events::ProviderSessionVoiceView {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = client
            .conversation_snapshot(cid, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains(REMOTE));
        assert!(!encoded.contains("12345678-1234-1234-1234-123456789016"));
        assert!(!encoded.contains("/control"));
        assert!(!snapshot.items.iter().any(|v| v["kind"] == "connection"));
        for value in snapshot.items {
            if let connect_protocol::events::SnapshotItem::ProviderCall(call) =
                serde_json::from_value(value).unwrap()
            {
                if call.session_id == sid && call.state == want {
                    assert!(!call.local_media && !call.remote_identity_verified);
                    return call;
                }
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "missing provider Session {sid} state {want:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_vapi_correlated_quarantine_resolution_preserves_the_foreign_call_and_original_receipt(
) {
    use wiremock::{
        matchers::{header, method, path},
        Mock, MockServer, ResponseTemplate,
    };
    const SECOND: &str = "12345678-1234-1234-1234-123456789016";
    let (f, policy) = setup().await;
    let provider = MockServer::start().await;
    let names = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let indexed = Arc::new(AtomicBool::new(false));
    let first_ended = Arc::new(AtomicBool::new(false));
    let sent = names.clone();
    Mock::given(method("POST"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path("/call"))
        .respond_with(move |r: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            let mut sent = sent.lock().unwrap();
            assert!(sent.len() < 2, "creation was repeated");
            sent.push(body["name"].as_str().unwrap().into());
            // Both transport replies incorrectly claim the first resource. The
            // second original mutation has its own distinct actual account resource.
            ResponseTemplate::new(200).set_body_json(json!({"id":REMOTE,"status":"in-progress"}))
        })
        .mount(&provider)
        .await;
    let make_row = {
        let names = names.clone();
        let first_ended = first_ended.clone();
        move |second: bool| {
            let id = if second { SECOND } else { REMOTE };
            let name = names.lock().unwrap()[usize::from(second)].clone();
            let ended = second || first_ended.load(Ordering::SeqCst);
            json!({"id":id,"name":name,"type":"outboundPhoneCall","assistantId":ASSISTANT,"phoneNumberId":PHONE,"customer":{"number":"+15555550111"},"createdAt":chrono::Utc::now().to_rfc3339(),"status":if ended {"ended"}else{"in-progress"},"endedReason":if second {Some(format!("old-conflict-{REMOTE}"))}else{None}})
        }
    };
    let list_row = make_row.clone();
    let listed = indexed.clone();
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer isolated-fixture-key"))
        .and(path("/call"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(200).set_body_json(if listed.load(Ordering::SeqCst) {
                vec![list_row(false), list_row(true)]
            } else {
                vec![]
            })
        })
        .mount(&provider)
        .await;
    for (id, second) in [(REMOTE, false), (SECOND, true)] {
        let row = make_row.clone();
        Mock::given(method("GET"))
            .and(header("authorization", "Bearer isolated-fixture-key"))
            .and(path(format!("/call/{id}")))
            .respond_with(move |_: &wiremock::Request| {
                ResponseTemplate::new(200).set_body_json(row(second))
            })
            .mount(&provider)
            .await;
    }
    let driver = Arc::new(
        connect_connectors::Vapi::with_reference(
            "isolated-fixture-key",
            &provider.uri(),
            "OFFLINE_FIXTURE",
        )
        .unwrap(),
    );
    let worker = Worker::with_routes(
        &f.server.service,
        vec![ProviderVoiceRoute {
            policy,
            driver,
            automatic_outbound: true,
        }],
    )
    .unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let (first, _) = invite(&f, &mut native).await;
    session_provider_view(
        &mut native,
        &first.conversation_id,
        &first.session_id,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    let second = connect_protocol::session_control::Request {
        operation_id: "quarantine-second-invite".into(),
        session_id: "quarantine-second-session".into(),
        ..first.clone()
    };
    let original = native.session_command(second.clone()).await.unwrap();
    let conflict = session_provider_view(
        &mut native,
        &second.conversation_id,
        &second.session_id,
        connect_protocol::events::ProviderCallState::Unknown,
    )
    .await;
    assert_eq!(
        conflict.problem.as_deref(),
        Some("provider-binding-conflict")
    );
    indexed.store(true, Ordering::SeqCst);
    let resolved = session_provider_view(
        &mut native,
        &second.conversation_id,
        &second.session_id,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    assert_eq!(resolved.id, conflict.id);
    assert!(resolved.problem.is_none() && resolved.ended_reason.is_none());
    assert_eq!(
        native.session_command(second.clone()).await.unwrap(),
        original
    );
    session_provider_view(
        &mut native,
        &first.conversation_id,
        &first.session_id,
        connect_protocol::events::ProviderCallState::Active,
    )
    .await;
    let captured_names = names.lock().unwrap().clone();
    assert_eq!(captured_names.len(), 2);
    assert_ne!(captured_names[0], captured_names[1]);
    first_ended.store(true, Ordering::SeqCst);
    session_provider_view(
        &mut native,
        &first.conversation_id,
        &first.session_id,
        connect_protocol::events::ProviderCallState::Ended,
    )
    .await;
    worker.shutdown().await.unwrap();
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method.as_str() == "POST")
            .count(),
        2
    );
    assert!(requests
        .iter()
        .any(|r| r.url.path() == format!("/call/{SECOND}")));
    native.close();
    f.server.shutdown().await.unwrap();
}
