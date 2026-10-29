//! Current installed CLI and explicit application SDK against the canonical runtime.
#![recursion_limit = "256"]
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::{CanonicalClient, CanonicalClientConfig, Command, CommandScope, Payload},
    conversations::CreateRequest,
    MessageDraft,
};
use connect_protocol::{Operation, Outcome, PROFILE};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::canonical_init,
};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
fn port() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}
fn native_config(state: &Path, address: std::net::SocketAddr) -> CanonicalClientConfig {
    let mut c = CanonicalClientConfig::load(&state.join("reviews.toml")).unwrap();
    c.endpoint = address;
    c
}
struct Fixture {
    _temp: tempfile::TempDir,
    state: PathBuf,
    server: RunningCanonicalServer,
    application: ApplicationClientConfig,
}
impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        canonical_init::initialize_with_http(
            &state,
            true,
            "127.0.0.1:0".parse().unwrap(),
            Some(port()),
        )
        .unwrap();
        let application =
            ApplicationClientConfig::load(&state.join("reviews.application.toml")).unwrap();
        let server = RunningCanonicalServer::start(
            CanonicalServerConfig::load(&state.join("server.toml")).unwrap(),
        )
        .await
        .unwrap();
        Self {
            _temp: temp,
            state,
            server,
            application,
        }
    }
    async fn room(&self, client: &mut CanonicalClient) -> String {
        let other = CanonicalClientConfig::load(&self.state.join("agent.toml")).unwrap();
        let mut input = CreateRequest::persistent(vec![other.identity_id]);
        input.tenant_id = Some("tnt_default".into());
        client
            .create_conversation("application-room", input)
            .await
            .unwrap()
            .id
    }
}
#[tokio::test]
async fn application_and_signed_clients_share_current_authority_health_and_mutation_receipts() {
    let f = Fixture::new().await;
    let mut app = ApplicationClient::open(f.application.clone())
        .await
        .unwrap();
    let peer = app.resolve("connect:agent@local").await.unwrap();
    assert_eq!(peer.alias, "agent");
    let Outcome::Health {
        schema,
        profile,
        installation_id,
        readiness,
    } = app.health().await.unwrap()
    else {
        panic!("health")
    };
    assert_eq!(schema, connect_store::SCHEMA_VERSION);
    assert_eq!(profile, PROFILE);
    assert_eq!(installation_id, f.server.store.installation_id().unwrap());
    assert!(readiness.is_some());
    let mut native = CanonicalClient::open(native_config(&f.state, f.server.address))
        .await
        .unwrap();
    let cid = f.room(&mut native).await;
    native
        .send_message(
            "application-source",
            &cid,
            "application_message",
            MessageDraft::text_all("shared source"),
        )
        .await
        .unwrap();
    let operation = || Operation::ReceiptPreferencesSet {
        request_id: "application-preferences".into(),
        conversation_id: cid.clone(),
        delivery: true,
        read: true,
    };
    let first = app.execute(operation().try_into().unwrap()).await.unwrap();
    let replay = app.execute(operation().try_into().unwrap()).await.unwrap();
    assert_eq!(first, replay);
    let record = native
        .operation_get(
            "application-preferences",
            CommandScope {
                conversation_id: Some(cid.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(record.state).unwrap(),
        json!("completed")
    );
    assert_eq!(record.result, Some(serde_json::to_value(first).unwrap()));
    let changed = Operation::ReceiptPreferencesSet {
        request_id: "application-preferences".into(),
        conversation_id: cid.clone(),
        delivery: false,
        read: false,
    };
    assert!(
        matches!(app.execute(changed.try_into().unwrap()).await,Err(connect_client::ClientError::Application(e)) if e.code=="conflict")
    );
    let Outcome::MembershipSnapshot(members) = app
        .execute(
            Operation::MembershipSnapshot {
                conversation_id: cid,
            }
            .try_into()
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("members")
    };
    assert_eq!(members.participants.len(), 2);
    app.health().await.unwrap();
    native.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_application_write_replays_one_original_id_and_current_revocation_is_enforced() {
    let f = Fixture::new().await;
    let mut native = CanonicalClient::open(native_config(&f.state, f.server.address))
        .await
        .unwrap();
    let cid = f.room(&mut native).await;
    let mut app = ApplicationClient::open(f.application.clone())
        .await
        .unwrap();
    let operation = || Operation::ReceiptPreferencesSet {
        request_id: "cancelled-application-preferences".into(),
        conversation_id: cid.clone(),
        delivery: true,
        read: true,
    };
    let lock = rusqlite::Connection::open(f.server.store.path()).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let cancel = token.clone();
    let (result, ()) = tokio::join!(
        app.execute_with_cancel(operation().try_into().unwrap(), token),
        async {
            tokio::time::sleep(Duration::from_millis(40)).await;
            cancel.cancel();
        }
    );
    assert!(matches!(
        result,
        Err(connect_client::ClientError::Cancelled)
    ));
    lock.execute_batch("ROLLBACK").unwrap();
    let mut fresh = ApplicationClient::open(f.application.clone())
        .await
        .unwrap();
    let first = fresh
        .execute(operation().try_into().unwrap())
        .await
        .unwrap();
    assert_eq!(
        fresh
            .execute(operation().try_into().unwrap())
            .await
            .unwrap(),
        first
    );
    let record = native
        .operation_get(
            "cancelled-application-preferences",
            CommandScope {
                conversation_id: Some(cid),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(record.result, Some(serde_json::to_value(first).unwrap()));
    let operator =
        ApplicationClientConfig::load(&f.state.join("operator.application.toml")).unwrap();
    let actor = f.server.store.authenticate(&operator.credential).unwrap();
    let target = f
        .server
        .store
        .authenticate(&f.application.credential)
        .unwrap();
    f.server
        .store
        .revoke(&actor, &target.credential_id)
        .unwrap();
    assert!(
        matches!(fresh.health().await,Err(connect_client::ClientError::Application(e)) if e.code=="unauthorized")
    );
    native.close();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn application_task_discovery_uses_only_explicit_current_credential() {
    use connect_client::tasks::{TaskCommand, TaskRequest};
    let f = Fixture::new().await;
    let cfg = ApplicationClientConfig::load(&f.state.join("operator.application.toml")).unwrap();
    let mut app = ApplicationClient::open(cfg.clone()).await.unwrap();
    let request = || {
        Operation::Task {
            request: Box::new(TaskRequest::new(TaskCommand::Get {
                task_id: "task_absent".into(),
            })),
        }
        .try_into()
        .unwrap()
    };
    assert!(
        matches!(app.execute(request()).await,Err(connect_client::ClientError::Application(e)) if e.code=="unsupported-profile")
    );
    app.negotiate_tasks("connect:agent@local", None)
        .await
        .unwrap();
    // Exercise a genuine application-specific mutation and its original-ID
    // replay, rather than proving only discovery or a refusal path.
    let mut native_config = CanonicalClientConfig::load(&f.state.join("operator.toml")).unwrap();
    native_config.endpoint = f.server.address;
    let mut native = CanonicalClient::open(native_config).await.unwrap();
    let cid = f.room(&mut native).await;
    let agent = CanonicalClientConfig::load(&f.state.join("agent.toml")).unwrap();
    let create = Operation::Task {
        request: Box::new(TaskRequest::new(TaskCommand::Create {
            request_id: "application-owned-task".into(),
            definition: connect_client::tasks::TaskDefinition {
                conversation_id: cid,
                scope: connect_client::tasks::TaskScope {
                    operations: vec![connect_client::tasks::TaskPermission::History],
                    recipients: vec![agent.identity_id],
                    sms_enrollments: vec![],
                },
                expires_at: chrono::Utc::now().timestamp() + 60,
                initiating_event_id: None,
            },
        })),
    };
    let first = app
        .execute(create.clone().try_into().unwrap())
        .await
        .unwrap();
    assert_eq!(
        app.execute(create.try_into().unwrap()).await.unwrap(),
        first
    );
    let Outcome::Task(value) = first else {
        panic!("task")
    };
    let connect_client::tasks::TaskOutcome::Record(task) = *value else {
        panic!("record")
    };
    let fetched = app
        .execute(
            Operation::Task {
                request: Box::new(TaskRequest::new(TaskCommand::Get {
                    task_id: task.id.clone(),
                })),
            }
            .try_into()
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        matches!(fetched,Outcome::Task(value) if matches!(&*value,connect_client::tasks::TaskOutcome::Record(record) if record.id==task.id))
    );
    native.shutdown().await.unwrap();
    let protected = connect_client::tasks::ProtectedTaskGrant::new(
        connect_client::tasks::InvocationGrant {
            id: "unissued_grant".into(),
            task_id: "task_absent".into(),
            binding: connect_client::tasks::InvocationBinding {
                kind: connect_client::tasks::InvocationKind::Native,
                adapter: "native".into(),
                external_id: "unissued_origin".into(),
            },
            operations: vec![],
            expires_at: cfg.expires_at.timestamp(),
            revoked: false,
        },
        "unissued-grant-only-canary".into(),
    );
    let mut narrowed = ApplicationClient::open(cfg.with_task_grant(&protected).unwrap())
        .await
        .unwrap();
    assert!(matches!(narrowed.health().await,
        Err(connect_client::ClientError::Application(e)) if e.code == "unauthorized"));

    assert!(
        matches!(app.execute(request()).await,Err(connect_client::ClientError::Application(e)) if e.code=="forbidden" || e.code=="not-found")
    );
    f.server.shutdown().await.unwrap();
}
async fn run(args: &[&str], input: Option<Value>) -> std::process::Output {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(value) = input {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
    } else {
        drop(child.stdin.take());
    }
    tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
}
fn payload(kind: &str, value: Value) -> Payload {
    serde_json::from_value(json!({"type":kind,"payload":value})).unwrap()
}
#[tokio::test]
async fn installed_request_health_resolve_use_explicit_current_transports_without_config_fallback()
{
    let temp = tempfile::tempdir_in(".local/gate-6").unwrap();
    let state = temp.path().join("fresh");
    let http = port();
    let out = run(
        &[
            "init",
            "--directory",
            state.to_str().unwrap(),
            "--example",
            "review-loop",
            "--bind",
            "127.0.0.1:0",
            "--http-bind",
            &http.to_string(),
        ],
        None,
    )
    .await;
    assert!(out.status.success());
    let mut cfg = CanonicalServerConfig::load(&state.join("server.toml")).unwrap();
    cfg.limits.connections = 2;
    std::fs::write(state.join("server.toml"), toml::to_string(&cfg).unwrap()).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(["serve", "--config"])
        .arg(state.join("server.toml"))
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
        .await
        .unwrap()
        .unwrap();
    let ready = match ready {
        Some(ready) => ready,
        None => {
            use tokio::io::AsyncReadExt;
            let status = child.wait().await.unwrap();
            let mut diagnostics = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut diagnostics)
                .await
                .unwrap();
            panic!("serve exited before readiness: {status}: {diagnostics}");
        }
    };
    assert!(ready.contains("profile=connect/2"));
    let address = ready
        .split_whitespace()
        .find_map(|s| s.strip_prefix("address="))
        .unwrap()
        .parse()
        .unwrap();
    let mut app_native = CanonicalClientConfig::load(&state.join("reviews.toml")).unwrap();
    app_native.endpoint = address;
    std::fs::write(
        state.join("reviews.toml"),
        toml::to_string(&app_native).unwrap(),
    )
    .unwrap();
    let native = state.join("reviews.toml");
    let application = state.join("reviews.application.toml");
    let peer = CanonicalClientConfig::load(&state.join("agent.toml")).unwrap();
    let mut create = CreateRequest::persistent(vec![peer.identity_id]);
    create.tenant_id = Some("tnt_default".into());
    let command = Command {
        command: payload("conversation.create", serde_json::to_value(create).unwrap()),
        scope: CommandScope {
            operation_id: Some("installed-command-create".into()),
            ..Default::default()
        },
    };
    let input = serde_json::to_value(command).unwrap();
    let mut ids = Vec::new();
    let mut conversations = Vec::new();
    for _ in 0..6 {
        let out = run(
            &[
                "request",
                "--config",
                native.to_str().unwrap(),
                "--envelope",
            ],
            Some(input.clone()),
        )
        .await;
        assert!(out.status.success(), "command failed");
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        let data = &value["result"]["Ok"]["data"];
        ids.push(data["request_id"].clone());
        conversations.push(
            data["frames"].as_array().unwrap().last().unwrap()["payload"]["details"]
                ["conversation"]["id"]
                .clone(),
        );
    }
    assert!(ids.windows(2).all(|x| x[0] != x[1]));
    assert!(conversations.iter().all(|x| x == &conversations[0]));
    let out = run(
        &[
            "resolve",
            "--config",
            application.to_str().unwrap(),
            "connect:agent@local",
        ],
        None,
    )
    .await;
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["alias"],
        "agent"
    );
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(["health", "--config"])
        .arg(&application)
        .env(
            "HTTP_PROXY",
            format!("http://{}", proxy.local_addr().unwrap()),
        )
        .env(
            "ALL_PROXY",
            format!("http://{}", proxy.local_addr().unwrap()),
        )
        .env("NO_PROXY", "")
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["transport"], "http");
    assert_eq!(value["result"]["kind"], "health");
    assert!(value["result"]["data"]["readiness"].is_object());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), proxy.accept())
            .await
            .is_err()
    );
    let out = run(&["health", "--config", native.to_str().unwrap()], None).await;
    assert!(!out.status.success());
    let out = run(
        &[
            "request",
            "--config",
            application.to_str().unwrap(),
            "--envelope",
        ],
        Some(input),
    )
    .await;
    assert!(!out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"]["Err"]["code"],
        "configuration"
    );
    let rejected = serde_json::to_value(Operation::Create {
        request_id: "no-application-native".into(),
        request: CreateRequest::persistent(vec![]),
    })
    .unwrap();
    let out = run(
        &[
            "request",
            "--application",
            "--config",
            application.to_str().unwrap(),
            "--envelope",
        ],
        Some(rejected),
    )
    .await;
    assert!(!out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"]["Err"]["code"],
        "canonical-command-required"
    );
    let out = run(
        &[
            "request",
            "--application",
            "--config",
            application.to_str().unwrap(),
            "--envelope",
        ],
        Some(json!({"op":"health"})),
    )
    .await;
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["result"]["Ok"]["kind"],
        "health"
    );
    rustix::process::kill_process(
        rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap(),
        rustix::process::Signal::TERM,
    )
    .unwrap();
    assert!(tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
}
#[test]
fn application_bootstrap_rejects_unsaved_endpoint_addresses_before_creating_state() {
    let temp = tempfile::tempdir().unwrap();
    let address = "127.0.0.1:0".parse().unwrap();
    for http in ["0.0.0.0:4434", "127.0.0.1:0"] {
        let state = temp.path().join("rejected");
        assert!(canonical_init::initialize_with_http(
            &state,
            true,
            address,
            Some(http.parse().unwrap())
        )
        .is_err());
        assert!(!state.exists());
    }
}

fn identity_command(id: &str, alias: &str) -> connect_protocol::provisioning::ProvisioningCommand {
    connect_protocol::provisioning::ProvisioningCommand::Identity {
        operation_id: id.into(),
        alias: alias.into(),
        kind: connect_protocol::provisioning::IdentityKind::Human,
        display_name: None,
    }
}
async fn administrator(f: &Fixture) -> ApplicationClient {
    ApplicationClient::open(
        ApplicationClientConfig::load(&f.state.join("operator.application.toml")).unwrap(),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn provisioning_sdk_executes_all_routes_and_projects_current_state_on_replay() {
    use base64::Engine;
    use connect_protocol::provisioning::*;
    let f = Fixture::new().await;
    let mut admin = administrator(&f).await;
    let create = identity_command("sdk-provision-identity", "sdk-new");
    let first = admin.provision(create.clone()).await.unwrap();
    assert_eq!(admin.provision(create.clone()).await.unwrap(), first);
    let ProvisioningOutcome::Identity {
        identity_id,
        revision,
        ..
    } = first
    else {
        panic!("identity")
    };
    assert!(
        matches!(admin.provision(identity_command("sdk-provision-identity","changed")).await,Err(connect_client::ClientError::Application(e)) if e.code=="conflict")
    );
    let mut native = CanonicalClient::open(native_config(&f.state, f.server.address))
        .await
        .unwrap();
    let cid = f.room(&mut native).await;
    let member = ProvisioningCommand::Participant {
        operation_id: "sdk-provision-participant".into(),
        conversation_id: cid.clone(),
        identity_id: identity_id.clone(),
        role: "member".into(),
    };
    let enrolled = admin.provision(member.clone()).await.unwrap();
    assert_eq!(admin.provision(member).await.unwrap(), enrolled);
    let endpoint =
        |identity_id: String, device_id: &str, operation_id: &str| ProvisioningCommand::Endpoint {
            operation_id: operation_id.into(),
            identity_id,
            device: DeviceSpec {
                id: device_id.into(),
                kind: "desktop".into(),
                platform: "macos".into(),
            },
            transport: EndpointTransport::Quic,
            address: format!(
                "quic://127.0.0.1:{}/",
                if device_id == "sdk_new_device" {
                    7001
                } else {
                    7002
                }
            ),
            capabilities: vec!["messaging".into()],
            secret_ref: None,
        };
    let ep = endpoint(
        identity_id.clone(),
        "sdk_new_device",
        "sdk-provision-endpoint",
    );
    let ProvisioningOutcome::Endpoint {
        endpoint_id,
        revision: ep_revision,
        ..
    } = admin.provision(ep.clone()).await.unwrap()
    else {
        panic!("endpoint")
    };
    let verify = ProvisioningCommand::VerifyEndpoint {
        operation_id: "sdk-verify-endpoint".into(),
        endpoint_id: endpoint_id.clone(),
        expected_revision: ep_revision,
        evidence_ref: "local-proof".into(),
    };
    let ProvisioningOutcome::Endpoint {
        revision: verified_revision,
        verification_state,
        ..
    } = admin.provision(verify.clone()).await.unwrap()
    else {
        panic!("verify")
    };
    assert_eq!(verification_state, "verified");
    admin
        .provision(ProvisioningCommand::RevokeEndpoint {
            operation_id: "sdk-revoke-endpoint".into(),
            endpoint_id,
            expected_revision: verified_revision,
        })
        .await
        .unwrap();
    for replay in [ep, verify] {
        let ProvisioningOutcome::Endpoint {
            verification_state, ..
        } = admin.provision(replay).await.unwrap()
        else {
            panic!("replay")
        };
        assert_eq!(verification_state, "revoked");
    }
    let agent = CanonicalClientConfig::load(&f.state.join("agent.toml")).unwrap();
    admin
        .provision(endpoint(
            agent.identity_id.clone(),
            "sdk_key_device",
            "sdk-key-endpoint",
        ))
        .await
        .unwrap();
    let actor = f.server.store.authenticate(&agent.credential).unwrap();
    let key = ProvisioningCommand::SigningKey {
        operation_id: "sdk-enroll-key".into(),
        identity_id: agent.identity_id,
        credential_id: actor.credential_id,
        device_id: "sdk_key_device".into(),
        key_id: "sdk_key".into(),
        algorithm: SigningAlgorithm::EdDSA,
        public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            ed25519_dalek::SigningKey::from_bytes(&[19; 32])
                .verifying_key()
                .as_bytes(),
        ),
        not_before: chrono::Utc::now().timestamp() - 1,
        expires_at: chrono::Utc::now().timestamp() + 3600,
        expected_revision: None,
    };
    let ProvisioningOutcome::SigningKey {
        revision: key_revision,
        ..
    } = admin.provision(key.clone()).await.unwrap()
    else {
        panic!("key")
    };
    admin
        .provision(ProvisioningCommand::RevokeSigningKey {
            operation_id: "sdk-revoke-key".into(),
            key_id: "sdk_key".into(),
            expected_revision: key_revision,
        })
        .await
        .unwrap();
    assert!(matches!(
        admin.provision(key).await.unwrap(),
        ProvisioningOutcome::SigningKey { revoked: true, .. }
    ));
    // Fresh devices start at revision zero; retrieve the persisted public projection.
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    let device_revision = c
        .query_row(
            "SELECT revision FROM devices WHERE id='sdk_key_device'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    admin
        .provision(ProvisioningCommand::RevokeDevice {
            operation_id: "sdk-revoke-device".into(),
            device_id: "sdk_key_device".into(),
            expected_revision: device_revision,
        })
        .await
        .unwrap();
    admin
        .provision(ProvisioningCommand::RevokeIdentity {
            operation_id: "sdk-revoke-identity".into(),
            identity_id,
            expected_revision: revision,
        })
        .await
        .unwrap();
    assert!(matches!(
        admin.provision(create).await.unwrap(),
        ProvisioningOutcome::Identity { revoked: true, .. }
    ));
    native.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_provisioning_reuses_saved_intent_and_replay_requires_current_admin() {
    let f = Fixture::new().await;
    let mut public = ApplicationClient::open(f.application.clone())
        .await
        .unwrap();
    let command = identity_command("sdk-cancel-provisioning", "cancelled-provisioning");
    assert!(
        matches!(public.provision(command.clone()).await,Err(connect_client::ClientError::Application(e)) if e.code=="forbidden")
    );
    let mut admin = administrator(&f).await;
    let lock = rusqlite::Connection::open(f.server.store.path()).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let token = tokio_util::sync::CancellationToken::new();
    let cancel = token.clone();
    let (result, ()) = tokio::join!(admin.provision_with_cancel(command.clone(), token), async {
        tokio::time::sleep(Duration::from_millis(40)).await;
        cancel.cancel();
    });
    assert!(matches!(
        result,
        Err(connect_client::ClientError::Cancelled)
    ));
    lock.execute_batch("ROLLBACK").unwrap();
    let mut fresh = administrator(&f).await;
    let first = fresh.provision(command.clone()).await.unwrap();
    assert_eq!(fresh.provision(command.clone()).await.unwrap(), first);
    assert_eq!(
        lock.query_row(
            "SELECT COUNT(*) FROM operations WHERE operation_id='sdk-cancel-provisioning'",
            [],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    let cfg = ApplicationClientConfig::load(&f.state.join("operator.application.toml")).unwrap();
    let actor = f.server.store.authenticate(&cfg.credential).unwrap();
    f.server.store.revoke(&actor, &actor.credential_id).unwrap();
    assert!(
        matches!(fresh.provision(command).await,Err(connect_client::ClientError::Application(e)) if e.code=="unauthorized")
    );
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn installed_provision_command_uses_application_authority_and_never_echoes_invalid_input() {
    use connect_protocol::provisioning::*;
    let f = Fixture::new().await;
    let operator = f.state.join("operator.application.toml");
    let command = serde_json::to_value(identity_command(
        "installed-provision",
        "installed-provisioned",
    ))
    .unwrap();
    let mut replies = Vec::new();
    for _ in 0..2 {
        let out = run(
            &["provision", "--config", operator.to_str().unwrap()],
            Some(command.clone()),
        )
        .await;
        assert!(out.status.success());
        let response: ProvisioningResponse = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(response.profile, PROVISIONING_PROFILE);
        replies.push(response.result.unwrap());
    }
    assert_eq!(replies[0], replies[1]);
    for (path, input, code) in [
        (
            f.state.join("operator.toml"),
            command.clone(),
            "configuration",
        ),
        (
            f.state.join("reviews.application.toml"),
            command.clone(),
            "forbidden",
        ),
        (
            operator.clone(),
            json!({"type":"identity","alias":"private-canary","kind":"human"}),
            "invalid-request",
        ),
        (
            operator.clone(),
            json!({"type":"revoke_device","operation_id":"invalid-path","device_id":"../private-canary","expected_revision":0}),
            "invalid-request",
        ),
        (
            operator,
            json!({"private-canary":"x".repeat(65_536)}),
            "invalid-request",
        ),
    ] {
        let out = run(
            &["provision", "--config", path.to_str().unwrap()],
            Some(input),
        )
        .await;
        assert!(!out.status.success());
        let response: ProvisioningResponse = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(response.result.unwrap_err().code, code);
        assert!(!String::from_utf8_lossy(&out.stdout).contains("private-canary"));
        assert!(!String::from_utf8_lossy(&out.stderr).contains("private-canary"));
    }
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn credential_revocation_sdk_retires_signed_peer_and_native_recovery_shares_the_receipt() {
    use base64::Engine;
    use connect_protocol::provisioning::*;
    let f = Fixture::new().await;
    let mut cfg = CanonicalClientConfig::load(&f.state.join("agent.toml")).unwrap();
    cfg.endpoint = f.server.address;
    let target = f.server.store.authenticate(&cfg.credential).unwrap();
    // Keep the same key valid under another explicitly enrolled credential.
    // Revoked authority must not borrow that credential through key resolution.
    let replacement = f
        .server
        .store
        .issue(&target.identity_id, target.scope.clone(), target.expires_at)
        .unwrap();
    let facts = f
        .server
        .store
        .resolve_identity_signing_key(
            &cfg.identity_id,
            &f.server.store.installation_id().unwrap(),
            &cfg.signing_key_id,
            &cfg.signing_algorithm,
        )
        .unwrap();
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    let revision = c
        .query_row(
            "SELECT revision FROM peer_signing_keys WHERE key_id=?1",
            [&cfg.signing_key_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut enrollment = administrator(&f).await;
    enrollment
        .provision(ProvisioningCommand::SigningKey {
            operation_id: "sdk-bind-replacement-credential".into(),
            identity_id: cfg.identity_id.clone(),
            credential_id: replacement.id,
            device_id: cfg.device_id.clone(),
            key_id: cfg.signing_key_id.clone(),
            algorithm: facts.algorithm,
            public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(facts.public_key),
            not_before: facts.not_before,
            expires_at: facts.expires_at,
            expected_revision: Some(revision),
        })
        .await
        .unwrap();
    let mut replacement_config = cfg.clone();
    replacement_config.credential = replacement.token;
    let mut peer = CanonicalClient::open(cfg.clone()).await.unwrap();
    let mut root_cfg = CanonicalClientConfig::load(&f.state.join("operator.toml")).unwrap();
    root_cfg.endpoint = f.server.address;
    let mut root = CanonicalClient::open(root_cfg).await.unwrap();
    let mut admin = administrator(&f).await;
    let command = ProvisioningCommand::RevokeCredential {
        operation_id: "sdk-revoke-peer-credential".into(),
        credential_id: target.credential_id,
    };
    let result = admin.provision(command.clone()).await.unwrap();
    assert_eq!(admin.provision(command).await.unwrap(), result);
    let terminal = tokio::time::timeout(Duration::from_secs(5), peer.receive())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        terminal.kind(),
        "auth.bye" | "identity.assurance-changed"
    ));
    assert!(peer.receive().await.is_err());
    assert!(CanonicalClient::open(cfg).await.is_err());
    let mut healthy = CanonicalClient::open(replacement_config).await.unwrap();
    healthy.keepalive().await.unwrap();
    healthy.shutdown().await.unwrap();
    let record = root
        .operation_get("sdk-revoke-peer-credential", Default::default())
        .await
        .unwrap();
    assert_eq!(record.operation_id.as_str(), "sdk-revoke-peer-credential");
    assert_eq!(
        serde_json::to_value(record.state).unwrap(),
        json!("completed")
    );
    assert_eq!(record.result, Some(serde_json::to_value(result).unwrap()));
    let mut application = ApplicationClient::open(
        ApplicationClientConfig::load(&f.state.join("agent.application.toml")).unwrap(),
    )
    .await
    .unwrap();
    assert!(
        matches!(application.health().await,Err(connect_client::ClientError::Application(e)) if e.code=="unauthorized")
    );
    admin.health().await.unwrap();
    root.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
}
#[tokio::test]
async fn installed_revoke_requires_saved_operation_id_and_current_application_config() {
    use connect_protocol::provisioning::*;
    let f = Fixture::new().await;
    let cfg = ApplicationClientConfig::load(&f.state.join("agent.application.toml")).unwrap();
    let target = f.server.store.authenticate(&cfg.credential).unwrap();
    let operator = f.state.join("operator.application.toml");
    let operation_id = "installed-credential-revocation";
    let mut replies = Vec::new();
    for _ in 0..2 {
        let out = run(
            &[
                "revoke",
                "--config",
                operator.to_str().unwrap(),
                "--operation-id",
                operation_id,
                &target.credential_id,
            ],
            None,
        )
        .await;
        assert!(out.status.success());
        let value: ProvisioningResponse = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value.profile, PROVISIONING_PROFILE);
        replies.push(value.result.unwrap());
        assert!(!String::from_utf8_lossy(&out.stdout).contains(&cfg.credential));
    }
    assert_eq!(replies[0], replies[1]);
    let out = run(
        &["provision", "--config", operator.to_str().unwrap()],
        Some(
            serde_json::to_value(ProvisioningCommand::RevokeCredential {
                operation_id: operation_id.into(),
                credential_id: target.credential_id.clone(),
            })
            .unwrap(),
        ),
    )
    .await;
    assert!(out.status.success());
    let reviews = f
        .server
        .store
        .authenticate(&f.application.credential)
        .unwrap();
    let out = run(
        &[
            "revoke",
            "--config",
            operator.to_str().unwrap(),
            "--operation-id",
            operation_id,
            &reviews.credential_id,
        ],
        None,
    )
    .await;
    assert!(!out.status.success());
    let value: ProvisioningResponse = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value.result.unwrap_err().code, "conflict");
    for (path, code) in [
        (f.state.join("operator.toml"), "configuration"),
        (f.state.join("reviews.application.toml"), "forbidden"),
    ] {
        let out = run(
            &[
                "revoke",
                "--config",
                path.to_str().unwrap(),
                "--operation-id",
                "refused-revoke",
                &reviews.credential_id,
            ],
            None,
        )
        .await;
        assert!(!out.status.success());
        let value: ProvisioningResponse = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value.result.unwrap_err().code, code);
    }
    let out = run(
        &[
            "revoke",
            "--config",
            operator.to_str().unwrap(),
            &reviews.credential_id,
        ],
        None,
    )
    .await;
    assert!(!out.status.success());
    let out = run(
        &[
            "revoke",
            "--config",
            operator.to_str().unwrap(),
            "--server-config",
            "private-no-longer-used",
            "--operation-id",
            "refused-revoke",
            &reviews.credential_id,
        ],
        None,
    )
    .await;
    assert!(!out.status.success());
    f.server
        .store
        .authenticate(&f.application.credential)
        .unwrap();
    f.server.shutdown().await.unwrap();
}
