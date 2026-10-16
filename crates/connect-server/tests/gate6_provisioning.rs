//! Public trusted-backend HTTP boundary over the ordinary application service.
#![allow(clippy::unwrap_used)]
use connect_protocol::{provisioning::*, Limits};
use connect_server::HttpListener as Listener;
use connect_service::Service;
use connect_store::{Scope, Store};
use std::sync::Arc;

struct Fixture {
    _dir: tempfile::TempDir,
    server: Listener,
    store: Arc<Store>,
    root: connect_store::IssuedCredential,
    client: reqwest::Client,
}
impl Fixture {
    async fn new() -> Self {
        Self::with_capacity(64).await
    }
    async fn with_capacity(connections: usize) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(
                &dir.path().join("state.sqlite3"),
                Limits {
                    connections,
                    ..Limits::default()
                },
            )
            .unwrap(),
        );
        let root = store.create_identity("root").unwrap();
        let root = store
            .issue(
                &root.id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        let server = Listener::bind(
            "127.0.0.1:0".parse().unwrap(),
            Service::new(store.clone()),
            None,
        )
        .await
        .unwrap();
        Self {
            _dir: dir,
            server,
            store,
            root,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.server.address)
    }
    async fn post(&self, path: &str, command: &ProvisioningCommand) -> reqwest::Response {
        self.client
            .post(self.url(path))
            .bearer_auth(&self.root.token)
            .json(command)
            .send()
            .await
            .unwrap()
    }
    async fn identity(&self) -> String {
        let response = self
            .post(
                "/v2/identities",
                &ProvisioningCommand::Identity {
                    operation_id: "alice".into(),
                    alias: "alice".into(),
                    kind: IdentityKind::Human,
                    display_name: None,
                },
            )
            .await;
        assert_eq!(response.status(), 200);
        match response
            .json::<ProvisioningResponse>()
            .await
            .unwrap()
            .result
            .unwrap()
        {
            ProvisioningOutcome::Identity { identity_id, .. } => identity_id,
            _ => panic!("identity"),
        }
    }
}
fn phone(owner: &str) -> ProvisioningCommand {
    ProvisioningCommand::Endpoint {
        operation_id: "phone".into(),
        identity_id: owner.into(),
        device: DeviceSpec {
            id: "device_phone".into(),
            kind: "telephone".into(),
            platform: "pstn".into(),
        },
        transport: EndpointTransport::Telephone,
        address: "+12025550101".into(),
        capabilities: vec!["audio".into(), "dtmf".into()],
        secret_ref: Some("env_TEST_TELNYX_KEY".into()),
    }
}

#[tokio::test]
async fn trusted_backend_provisions_verifies_replays_and_revokes_without_disclosing_routes() {
    let f = Fixture::new().await;
    let alice = f.identity().await;
    let path = format!("/v2/identities/{alice}/endpoints");
    let command = phone(&alice);
    let response = f.post(&path, &command).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let outcome = response
        .json::<ProvisioningResponse>()
        .await
        .unwrap()
        .result
        .unwrap();
    let endpoint = match outcome {
        ProvisioningOutcome::Endpoint {
            endpoint_id,
            verification_state,
            ..
        } => {
            assert_eq!(verification_state, "unverified");
            endpoint_id
        }
        _ => panic!("endpoint"),
    };
    let proof = ProvisioningCommand::VerifyEndpoint {
        operation_id: "verify".into(),
        endpoint_id: endpoint.clone(),
        expected_revision: 0,
        evidence_ref: "trusted_fixture_proof".into(),
    };
    let response = f
        .post(&format!("/v2/endpoints/{endpoint}/verify"), &proof)
        .await;
    assert_eq!(response.status(), 200);
    let text = f.post(&path, &command).await.text().await.unwrap();
    assert!(!text.contains("+12025550101"));
    assert!(!text.contains("env_TEST_TELNYX_KEY"));
    let replay: ProvisioningResponse = serde_json::from_str(&text).unwrap();
    assert!(
        matches!(replay.result.unwrap(),ProvisioningOutcome::Endpoint { revision:1,verification_state,.. } if verification_state=="verified")
    );
    let revoke = ProvisioningCommand::RevokeEndpoint {
        operation_id: "revoke".into(),
        endpoint_id: endpoint.clone(),
        expected_revision: 1,
    };
    assert_eq!(
        f.post(&format!("/v2/endpoints/{endpoint}/revoke"), &revoke)
            .await
            .status(),
        200
    );
    let late = ProvisioningCommand::VerifyEndpoint {
        operation_id: "late".into(),
        endpoint_id: endpoint.clone(),
        expected_revision: 2,
        evidence_ref: "late".into(),
    };
    assert_eq!(
        f.post(&format!("/v2/endpoints/{endpoint}/verify"), &late)
            .await
            .status(),
        409
    );
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn public_credentials_browser_origins_and_body_selected_authority_cannot_provision() {
    let f = Fixture::new().await;
    let alice = f.identity().await;
    let ordinary = f
        .store
        .issue(
            &alice,
            Scope::communications(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    for (token, status) in [(&ordinary.token, 403), (&"invalid".into(), 401)] {
        // Admission happens before parsing even a deliberately malformed body.
        let response = f
            .client
            .post(f.url("/v2/identities"))
            .bearer_auth(token)
            .header("content-type", "application/json")
            .body("invalid-json")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let response=f.client.post(f.url("/v2/identities")).bearer_auth(&f.root.token).header("origin","http://localhost:8000").json(&serde_json::json!({"type":"identity","operation_id":"browser","alias":"browser","kind":"human"})).send().await.unwrap();
    assert_eq!(response.status(), 403);
    let mut forged = serde_json::to_value(phone(&alice)).unwrap();
    forged["verified"] = serde_json::json!(true);
    let response = f
        .client
        .post(f.url(&format!("/v2/identities/{alice}/endpoints")))
        .bearer_auth(&f.root.token)
        .json(&forged)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        f.post("/v2/identities/id_other/endpoints", &phone(&alice))
            .await
            .status(),
        400
    );
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn logical_id_conflicts_revocation_and_payload_bounds_have_typed_responses() {
    let f = Fixture::new().await;
    let _alice = f.identity().await;
    let conflict = ProvisioningCommand::Identity {
        operation_id: "alice".into(),
        alias: "other".into(),
        kind: IdentityKind::Ai,
        display_name: None,
    };
    assert_eq!(f.post("/v2/identities", &conflict).await.status(), 409);
    let oversized = f
        .client
        .post(f.url("/v2/identities"))
        .bearer_auth(&f.root.token)
        .header("content-type", "application/json")
        .body(vec![b'x'; 65_537])
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), 413);
    assert_eq!(oversized.headers()["cache-control"], "no-store");
    assert_eq!(
        oversized
            .json::<ProvisioningResponse>()
            .await
            .unwrap()
            .result
            .unwrap_err()
            .code,
        "payload-too-large"
    );
    let admin = f.store.authenticate(&f.root.token).unwrap();
    f.store.revoke(&admin, &f.root.id).unwrap();
    assert_eq!(f.post("/v2/identities", &conflict).await.status(), 401);
    f.server.shutdown().await.unwrap();
}

async fn stalled_upload(f: &Fixture) -> tokio::net::TcpStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut socket = tokio::net::TcpStream::connect(f.server.address)
        .await
        .unwrap();
    let headers=format!("POST /v2/identities HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",f.server.address,f.root.token);
    socket.write_all(headers.as_bytes()).await.unwrap();
    let mut continued = vec![0; 25];
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        socket.read_exact(&mut continued),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&continued, b"HTTP/1.1 100 Continue\r\n\r\n");
    socket.write_all(b"1\r\n{\r\n").await.unwrap();
    socket
}
async fn read_upload(mut socket: tokio::net::TcpStream) -> String {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(7),
        socket.read_to_end(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    String::from_utf8(bytes).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_administrative_bodies_hold_capacity_and_shutdown_cancels_them() {
    let f = Fixture::with_capacity(1).await;
    let socket = stalled_upload(&f).await;
    let command = ProvisioningCommand::Identity {
        operation_id: "capacity".into(),
        alias: "capacity".into(),
        kind: IdentityKind::Human,
        display_name: None,
    };
    let response = f.post("/v2/identities", &command).await;
    assert_eq!(response.status(), 503);
    f.server.shutdown().await.unwrap();
    let response = read_upload(socket).await;
    assert!(response.starts_with("HTTP/1.1 503"));
    assert!(response.contains("transport-uncertain"));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_administrative_body_deadline_releases_capacity_without_a_mutation() {
    let f = Fixture::with_capacity(1).await;
    let start = std::time::Instant::now();
    let socket = stalled_upload(&f).await;
    let response = read_upload(socket).await;
    assert!(response.starts_with("HTTP/1.1 503"));
    assert!(response.contains("transport-uncertain"));
    assert!(start.elapsed() < std::time::Duration::from_secs(7));
    let _alice = f.identity().await;
    f.server.shutdown().await.unwrap();
}

fn capture_sample(
    name: &str,
    route: &str,
    schema: &str,
    request: &ProvisioningCommand,
    status: u16,
    result: &serde_json::Value,
) {
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let sample = serde_json::json!({"route":route,"request_schema":schema,"request":request,"status":status,"response":result});
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec(&sample).unwrap(),
        )
        .unwrap();
    }
}
#[tokio::test]
async fn http_contract_samples_cover_all_trusted_provisioning_routes() {
    let f = Fixture::new().await;
    let command = ProvisioningCommand::Identity {
        operation_id: "schema_identity".into(),
        alias: "schema-contact".into(),
        kind: IdentityKind::External,
        display_name: None,
    };
    let response = f.post("/v2/identities", &command).await;
    let status = response.status().as_u16();
    assert_eq!(status, 200);
    let value: serde_json::Value = response.json().await.unwrap();
    capture_sample(
        "identity",
        "/v2/identities",
        "IdentityRequest",
        &command,
        status,
        &value,
    );
    let identity = value["result"]["Ok"]["identity_id"].as_str().unwrap();
    let command = phone(identity);
    let path = format!("/v2/identities/{identity}/endpoints");
    let response = f.post(&path, &command).await;
    let status = response.status().as_u16();
    assert_eq!(status, 200);
    let value: serde_json::Value = response.json().await.unwrap();
    capture_sample(
        "endpoint",
        "/v2/identities/{id}/endpoints",
        "EndpointRequest",
        &command,
        status,
        &value,
    );
    let endpoint = value["result"]["Ok"]["endpoint_id"]
        .as_str()
        .unwrap()
        .to_string();
    let command = ProvisioningCommand::VerifyEndpoint {
        operation_id: "schema_verify".into(),
        endpoint_id: endpoint.clone(),
        expected_revision: 0,
        evidence_ref: "schema_fixture_proof".into(),
    };
    let response = f
        .post(&format!("/v2/endpoints/{endpoint}/verify"), &command)
        .await;
    let status = response.status().as_u16();
    assert_eq!(status, 200);
    let value: serde_json::Value = response.json().await.unwrap();
    capture_sample(
        "verify",
        "/v2/endpoints/{id}/verify",
        "VerifyEndpointRequest",
        &command,
        status,
        &value,
    );
    let command = ProvisioningCommand::RevokeEndpoint {
        operation_id: "schema_revoke".into(),
        endpoint_id: endpoint.clone(),
        expected_revision: 1,
    };
    let response = f
        .post(&format!("/v2/endpoints/{endpoint}/revoke"), &command)
        .await;
    let status = response.status().as_u16();
    assert_eq!(status, 200);
    let value: serde_json::Value = response.json().await.unwrap();
    capture_sample(
        "revoke_endpoint",
        "/v2/endpoints/{id}/revoke",
        "RevokeEndpointRequest",
        &command,
        status,
        &value,
    );
    // Current identity exists, but an unknown Conversation is not provisionable.
    let command = ProvisioningCommand::Participant {
        operation_id: "schema_member".into(),
        conversation_id: "conv_unknown".into(),
        identity_id: identity.into(),
        role: "agent".into(),
    };
    let response = f
        .post("/v2/conversations/conv_unknown/participants", &command)
        .await;
    let status = response.status().as_u16();
    assert_eq!(status, 403);
    let value: serde_json::Value = response.json().await.unwrap();
    capture_sample(
        "participant_denied",
        "/v2/conversations/{id}/participants",
        "ParticipantRequest",
        &command,
        status,
        &value,
    );
    let command = ProvisioningCommand::RevokeIdentity {
        operation_id: "schema_revoke_identity".into(),
        identity_id: identity.into(),
        expected_revision: 0,
    };
    let response = f
        .post(&format!("/v2/identities/{identity}/revoke"), &command)
        .await;
    let status = response.status().as_u16();
    assert_eq!(status, 200);
    let value: serde_json::Value = response.json().await.unwrap();
    capture_sample(
        "revoke_identity",
        "/v2/identities/{id}/revoke",
        "RevokeIdentityRequest",
        &command,
        status,
        &value,
    );
    f.server.shutdown().await.unwrap();
}
