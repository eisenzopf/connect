//! Isolated public-client behavior proofs; SQL is read-only test evidence.
use super::setup;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientConfig,
};
use connect_protocol::provisioning::*;
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::init,
    config::ServerConfig,
};
use connect_store::Scope;
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
#[path = "../../crates/connect-server/tests/support/canonical_listener_fixture.rs"]
mod canonical_listener_fixture;
use canonical_listener_fixture::*;

async fn wire(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(6), async {
        let mut bytes = Vec::new();
        let total = loop {
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0 && bytes.len() + n <= 2 * 1024 * 1024);
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(start) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                let headers = std::str::from_utf8(&bytes[..start]).unwrap();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break start + 4 + length;
            }
        };
        while bytes.len() < total {
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0 && bytes.len() + n <= 2 * 1024 * 1024);
            bytes.extend_from_slice(&buffer[..n]);
        }
        assert_eq!(bytes.len(), total);
        bytes
    })
    .await
    .unwrap()
}
struct LostResponseProxy {
    address: std::net::SocketAddr,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    lost: Arc<AtomicBool>,
    writes: Arc<AtomicUsize>,
}
impl LostResponseProxy {
    async fn start(destination: std::net::SocketAddr) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let closing = stop.clone();
        let lost = Arc::new(AtomicBool::new(false));
        let did_lose = lost.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let counted = writes.clone();
        let task = tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            loop {
                tokio::select! { biased;
                    _=closing.cancelled()=>break,
                    accepted=listener.accept()=>{
                        let (mut client,_)=accepted.unwrap();let lost=did_lose.clone();let counted=counted.clone();
                        workers.spawn(async move {
                            let request=wire(&mut client).await;
                            let identity=request.starts_with(b"POST /v2/identities ");
                            if request.starts_with(b"POST /v2/") {counted.fetch_add(1,Ordering::SeqCst);}
                            let mut upstream=tokio::net::TcpStream::connect(destination).await.unwrap();
                            // The owned loss proxy names the actual backend authority.
                            // Credential delivery correctly rejects the proxy's Host.
                            let boundary=request.windows(4).position(|x|x==b"\r\n\r\n").unwrap();
                            let headers=std::str::from_utf8(&request[..boundary]).unwrap().lines().map(|line| {
                                if line.split_once(':').is_some_and(|(name,_)|name.eq_ignore_ascii_case("host")) {
                                    format!("Host: {destination}")
                                } else {line.to_owned()}
                            }).collect::<Vec<_>>().join("\r\n");
                            let mut forwarded=headers.into_bytes();forwarded.extend_from_slice(b"\r\n\r\n");forwarded.extend_from_slice(&request[boundary+4..]);
                            upstream.write_all(&forwarded).await.unwrap();let response=wire(&mut upstream).await;
                            let success=response.starts_with(b"HTTP/1.1 2");
                            if identity && success && !lost.swap(true,Ordering::SeqCst) {return;}
                            let _=client.write_all(&response).await;
                        });
                    }
                    Some(result)=workers.join_next()=>result.unwrap(),
                }
            }
            while let Some(result) = workers.join_next().await {
                result.unwrap();
            }
        });
        Self {
            address,
            stop,
            task,
            lost,
            writes,
        }
    }
    async fn shutdown(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
fn config(token: &str, endpoint: std::net::SocketAddr) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint,
        credential: token.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 7000,
    }
}
fn actor(f: &Fixture, label: &str, algorithm: &str) -> setup::ActorPlan {
    let public = signing(&f.temp.path().join(format!("{label}.der")), algorithm);
    setup::ActorPlan {
        alias: format!("provision-{label}"),
        display_name: format!("Example {label}"),
        device: DeviceSpec {
            id: format!("provision-{label}-device"),
            kind: "desktop".into(),
            platform: "isolated-fixture".into(),
        },
        transport: EndpointTransport::Quic,
        address: format!("quic://localhost:{}/{label}", f.server.address.port()),
        capabilities: vec!["data".into()],
        key_id: format!("provision-{label}-key"),
        algorithm: if algorithm == "ES256" {
            SigningAlgorithm::ES256
        } else {
            SigningAlgorithm::EdDSA
        },
        public_key: URL_SAFE_NO_PAD.encode(public),
        enrollment_evidence: format!("fixture-owned-{label}-key"),
    }
}
fn setup_count(f: &Fixture) -> u64 {
    rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id LIKE 'provision-run-%'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_setup_recovers_lost_identity_response_replays_fifteen_originals_and_refuses_client_admin(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::new(algorithm, 8).await;
        let proxy = LostResponseProxy::start(f.server.http_address.unwrap()).await;
        let mut admin = ApplicationClient::open(config(&f.admin, proxy.address))
            .await
            .unwrap();
        let plan = setup::SetupPlan {
            setup_id: "provision-run".into(),
            not_before: chrono::Utc::now().timestamp() - 1,
            expires_at: chrono::Utc::now().timestamp() + 240,
            app: actor(&f, "app", algorithm),
            human: actor(&f, "human", algorithm),
            assistant: actor(&f, "assistant", algorithm),
        };
        let state = f.temp.path().join("trusted-setup");
        assert!(setup::setup(&mut admin, plan.clone(), &state)
            .await
            .is_err());
        assert!(proxy.lost.load(Ordering::SeqCst));
        assert_eq!(proxy.writes.load(Ordering::SeqCst), 1);
        assert_eq!(setup_count(&f), 1);
        let pending: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state.join("journal.json")).unwrap()).unwrap();
        assert_eq!(
            pending["steps"]["app.identity"]["outcome"],
            serde_json::Value::Null
        );
        assert_eq!(
            pending["steps"]["app.identity"]["request"]["command"]["operation_id"],
            "provision-run-app-identity"
        );
        let first = setup::setup(&mut admin, plan.clone(), &state)
            .await
            .unwrap();
        assert_eq!(first.actors.len(), 3);
        assert_eq!(setup_count(&f), 15);
        let replay = setup::setup(&mut admin, plan.clone(), &state)
            .await
            .unwrap();
        assert!(serde_json::to_value(&first).unwrap() == serde_json::to_value(&replay).unwrap());
        assert_eq!(setup_count(&f), 15);
        use std::os::unix::fs::PermissionsExt;
        for name in [
            "journal.json",
            "enrollments.private.json",
            "contacts.json",
            "setup.lock",
        ] {
            assert_eq!(
                std::fs::metadata(state.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        let public = std::fs::read_to_string(state.join("contacts.json")).unwrap();
        assert!(!public.contains(&f.admin));
        for actor in first.actors.values() {
            assert!(!public.contains(&actor.grant.credential));
            assert!(!actor
                .grant
                .operations
                .iter()
                .any(|x| matches!(x.as_str(), "*" | "admin" | "credential.delegate")));
            assert_eq!(actor.grant.recipients.len(), 2);
        }
        // Selecting a contact supplies its returned public ID; no route table or copied CID/SID.
        let human = &first.actors["human"];
        let selected = first
            .contacts
            .iter()
            .find(|x| x.key == "assistant")
            .unwrap();
        let mut signed = f.client.clone();
        signed.identity_id = human.identity_id.clone();
        signed.credential = human.grant.credential.clone();
        signed.device_id = human.device_id.clone();
        signed.device_kind = plan.human.device.kind.clone();
        signed.device_platform = plan.human.device.platform.clone();
        signed.signing_key_id = human.key_id.clone();
        signed.signing_private_key = f.temp.path().join("human.der");
        let mut peer = CanonicalClient::open(signed).await.unwrap();
        let mut create = connect_client::conversations::CreateRequest::persistent(vec![selected
            .identity_id
            .clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = peer
            .create_conversation("contact-choice", create)
            .await
            .unwrap();
        assert!(room
            .participants
            .iter()
            .any(|p| p.identity_id == selected.identity_id));
        peer.shutdown().await.unwrap();
        let mut changed = plan.clone();
        changed.expires_at += 1;
        assert!(setup::setup(&mut admin, changed, &state).await.is_err());
        assert_eq!(setup_count(&f), 15);
        // An actual untrusted-client credential cannot use backend provisioning.
        let mut child = ApplicationClient::open(config(&human.grant.credential, proxy.address))
            .await
            .unwrap();
        let refusal = child
            .provision(ProvisioningCommand::Identity {
                operation_id: "browser-cannot-provision".into(),
                alias: "browser-cannot-provision".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            })
            .await;
        assert!(
            matches!(refusal,Err(connect_client::ClientError::Application(ref error)) if error.code=="forbidden")
        );
        let browser = reqwest::Client::builder().no_proxy().build().unwrap();
        let forbidden = browser
            .post(format!("http://{}/v2/identities", proxy.address))
            .bearer_auth(&human.grant.credential)
            .header("Origin", "https://browser.fixture.invalid")
            .json(&ProvisioningCommand::Identity {
                operation_id: "browser-origin-cannot-provision".into(),
                alias: "browser-origin-cannot-provision".into(),
                kind: IdentityKind::Ai,
                display_name: None,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(forbidden.status(), reqwest::StatusCode::FORBIDDEN);
        // Observe current metadata through the exact saved public request.
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state.join("journal.json")).unwrap()).unwrap();
        let original: ProvisioningCommand =
            serde_json::from_value(saved["steps"]["human.endpoint"]["request"]["command"].clone())
                .unwrap();
        let observed = admin.provision(original).await.unwrap();
        let ProvisioningOutcome::Endpoint {
            revision: current_endpoint_revision,
            ..
        } = observed
        else {
            panic!("current endpoint receipt")
        };
        // Revocation observed on original replay never becomes a fresh success.
        let endpoint = admin
            .provision(ProvisioningCommand::RevokeEndpoint {
                operation_id: "revoke-owned-human".into(),
                endpoint_id: human.endpoint_id.clone(),
                expected_revision: current_endpoint_revision,
            })
            .await
            .unwrap();
        assert!(
            matches!(endpoint,ProvisioningOutcome::Endpoint {ref verification_state,..} if verification_state=="revoked")
        );
        assert!(setup::setup(&mut admin, plan, &state).await.is_err());
        assert_eq!(setup_count(&f), 15);
        proxy.shutdown().await;
        f.server.shutdown().await.unwrap();
        println!(
            "PROVISION_APP_EVIDENCE {}",
            serde_json::json!({
                "algorithm":algorithm,"original_setup_operations":15,"lost_identity_responses":1,
                "same_request_pending_before_recovery":true,"complete_rerun_identical":true,
                "explicit_recipient_count":2,"private_file_mode":"0600","contact_choice_opened_conversation":true,
                "changed_plan_refused":true,"ordinary_client_provisioning_refused":true,
                "browser_origin_provisioning_refused":true,"revoked_endpoint_not_revived":true,
                "proxy_and_server_joined":true
            })
        );
    }
}
