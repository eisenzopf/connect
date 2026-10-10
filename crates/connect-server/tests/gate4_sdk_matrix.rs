//! Real native, language SDK, HTTP and JSON CLI task behavior against one server.
#![allow(clippy::unwrap_used)]
#[path = "support/task_matrix.rs"]
mod task_matrix;
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{tasks::TaskRequest, Operation, Outcome, Response, PROFILE};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use task_matrix::Bindings;
use tokio::io::AsyncWriteExt;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_clients_share_task_lifecycle_replies_recall_replay_and_denial() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start_local(
        ServerConfig::load(&dir.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    let mut owner = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
    let mut peer = ClientConfig::load(&dir.join("agent.toml")).unwrap();
    let operations = [
        "health",
        "create",
        "resolve",
        "send",
        "history",
        "task_create",
        "task_get",
        "task_cancel",
        "task_transition",
        "task_replies",
        "task_assign_reply",
        "task_inbox",
        "task_consume_reply",
        "task_invoke",
        "task_recall",
        "task_wait",
        "task_effect_get",
        "task_claim",
        "task_claim_get",
        "task_release_claim",
    ];
    let credential = server
        .store
        .issue(
            &owner.identity_id,
            Scope {
                operations: operations.iter().map(|name| (*name).into()).collect(),
                recipients: vec![peer.identity_id.clone()],
            },
            chrono::Utc::now().timestamp() + 3600,
        )
        .unwrap();
    owner.endpoint = server.address;
    owner.credential_id = credential.id;
    owner.credential = credential.token;
    peer.endpoint = server.address;
    let owner_path = dir.join("owner.toml");
    let peer_path = dir.join("peer.toml");
    for (path, config) in [(&owner_path, &owner), (&peer_path, &peer)] {
        init::private_write(path, toml::to_string_pretty(config).unwrap().as_bytes()).unwrap();
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let url = format!("http://{}", server.http_address.unwrap());
    let http = tokio::task::spawn_blocking(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let context = MatrixContext {
        owner: &owner,
        peer: &peer,
        owner_path: &owner_path,
        peer_path: &peer_path,
        url: &url,
        http: &http,
        root: &root,
    };
    for lane in [
        "rust",
        "http",
        "cli",
        "python-native",
        "python-http",
        "typescript-native",
        "typescript-http",
    ] {
        let mut setup = ConnectClient::open(owner.clone()).await.unwrap();
        let cid = setup
            .create(
                &format!("{lane}-conversation"),
                vec![peer.identity_id.clone()],
            )
            .await
            .unwrap()
            .id;
        setup.close().await;
        let expires = chrono::Utc::now().timestamp() + 600;
        let bindings: Bindings = [
            ("cid".into(), json!(cid)),
            ("prefix".into(), json!(lane)),
            ("peer_id".into(), json!(peer.identity_id)),
            ("expires".into(), json!(expires)),
            ("expires_changed".into(), json!(expires + 1)),
        ]
        .into();
        if lane.starts_with("python-") || lane.starts_with("typescript-") {
            language(&context, lane, bindings).await;
        } else {
            run(&context, lane, bindings).await;
        }
    }
    // Programmatic errors remain machine-readable without echoing malformed input.
    for bytes in [b"{ invalid secret-input".as_slice(), &vec![b'x'; 65_537]] {
        let output = cli(&owner_path, bytes, false).await;
        assert!(!output.status.success());
        let response: Response = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response.result.unwrap_err().code, "invalid-request");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-input"));
    }
    let output = cli(
        &owner_path,
        &serde_json::to_vec(&Operation::Task {
            request: Box::new(TaskRequest::new(
                connect_protocol::tasks::TaskCommand::Get {
                    task_id: "not-dispatched".into(),
                },
            )),
        })
        .unwrap(),
        false,
    )
    .await;
    assert!(!output.status.success());
    let response: Response = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response.result.unwrap_err().code, "unsupported-profile");
    let mut legacy = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .arg("request")
        .arg("--config")
        .arg(&owner_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    legacy
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"op":"health"}"#)
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(20), legacy.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "legacy CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(matches!(
        serde_json::from_slice::<Outcome>(&output.stdout).unwrap(),
        Outcome::Health { .. }
    ));
    server.shutdown().await.unwrap();
}

struct MatrixContext<'a> {
    owner: &'a ClientConfig,
    peer: &'a ClientConfig,
    owner_path: &'a Path,
    peer_path: &'a Path,
    url: &'a str,
    http: &'a reqwest::Client,
    root: &'a Path,
}

async fn run(context: &MatrixContext<'_>, lane: &str, mut bindings: Bindings) {
    let MatrixContext {
        owner,
        peer,
        owner_path,
        peer_path,
        url,
        http,
        ..
    } = *context;
    let mut clients = [
        Some(ConnectClient::open(owner.clone()).await.unwrap()),
        Some(ConnectClient::open(peer.clone()).await.unwrap()),
    ];
    for (index, address) in ["connect:agent@local", "connect:reviews@local"]
        .iter()
        .enumerate()
    {
        clients[index]
            .as_mut()
            .unwrap()
            .negotiate_tasks(address, Some(bindings["cid"].as_str().unwrap()))
            .await
            .unwrap();
    }
    for template in task_matrix::matrix() {
        let step = task_matrix::expand(&template, &bindings);
        let actor = if step["actor"] == "peer" { 1 } else { 0 };
        let config = if actor == 0 { owner } else { peer };
        if step["reconnect"] == true {
            clients[actor].take().unwrap().close().await;
            clients[actor] = Some(ConnectClient::open(config.clone()).await.unwrap());
            clients[actor]
                .as_mut()
                .unwrap()
                .negotiate_tasks(
                    if actor == 0 {
                        "connect:agent@local"
                    } else {
                        "connect:reviews@local"
                    },
                    Some(bindings["cid"].as_str().unwrap()),
                )
                .await
                .unwrap();
        }
        let operation: Operation = if let Some(command) = step.get("command") {
            Operation::Task {
                request: Box::new(TaskRequest::new(
                    serde_json::from_value(command.clone()).unwrap(),
                )),
            }
        } else {
            serde_json::from_value(step["operation"].clone()).unwrap()
        };
        let result = match lane {
            "rust" => if let Operation::Task { request } = operation {
                clients[actor]
                    .as_mut()
                    .unwrap()
                    .task(request.command)
                    .await
                    .map(|outcome| Outcome::Task(Box::new(outcome)))
            } else {
                clients[actor].as_mut().unwrap().request(operation).await
            }
            .map_err(|error| match error {
                ClientError::Application(error) => error,
                _ => panic!("native exchange failed: {error}"),
            }),
            "http" => {
                let response = http
                    .post(format!("{url}/v1/operations"))
                    .bearer_auth(&config.credential)
                    .json(&operation)
                    .send()
                    .await
                    .unwrap();
                let response: Response = response.json().await.unwrap();
                assert_eq!(response.profile, PROFILE);
                response.result
            }
            "cli" => {
                let output = cli(
                    if actor == 0 { owner_path } else { peer_path },
                    &serde_json::to_vec(&operation).unwrap(),
                    true,
                )
                .await;
                let response: Response = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(response.result.is_ok(), output.status.success());
                assert_eq!(response.profile, PROFILE);
                response.result
            }
            _ => unreachable!(),
        };
        match result {
            Ok(outcome) => {
                assert!(
                    step.get("error").is_none(),
                    "{lane}: {} was accepted",
                    step["id"]
                );
                let value = match outcome {
                    Outcome::Task(outcome) => serde_json::to_value(*outcome).unwrap(),
                    outcome => serde_json::to_value(outcome).unwrap(),
                };
                task_matrix::verify(&value, &step, &mut bindings);
            }
            Err(error) => assert_eq!(
                error.code,
                step["error"].as_str().unwrap_or("success expected"),
                "{lane}: {}",
                step["id"]
            ),
        }
    }
    for client in clients.into_iter().flatten() {
        client.close().await;
    }
}

async fn cli(path: &Path, input: &[u8], discover: bool) -> std::process::Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    command
        .arg("request")
        .arg("--config")
        .arg(path)
        .arg("--envelope");
    if discover {
        command.arg("--task-address").arg("connect:reviews@local");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
}

async fn language(context: &MatrixContext<'_>, lane: &str, mut bindings: Bindings) {
    let MatrixContext {
        root,
        owner_path,
        peer_path,
        url,
        owner,
        peer,
        ..
    } = *context;
    let python = lane.starts_with("python-");
    let mut command = tokio::process::Command::new(if python { "python3" } else { "node" });
    command
        .arg(root.join(if python {
            "sdk/tests/task_matrix_probe.py"
        } else {
            "sdk/tests/task_matrix_probe.mjs"
        }))
        .arg(owner_path)
        .arg(peer_path)
        .arg(url)
        .arg(env!("CARGO_BIN_EXE_connect"))
        .arg(lane.split('-').nth(1).unwrap());
    if python {
        command.env("PYTHONPATH", root.join("sdk/python"));
    }
    bindings.insert(
        "credentials".into(),
        json!({"owner":owner.credential,"peer":peer.credential}),
    );
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&bindings).unwrap())
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "{lane} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
