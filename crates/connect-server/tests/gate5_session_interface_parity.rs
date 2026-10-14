//! Audio+video Session control parity; disposable state, no media/provider operations.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{Error, Operation, Outcome, Response};
use connect_server::cli::init;
use connect_store::Scope;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncWriteExt;

async fn run(mut command: tokio::process::Command, input: &[u8]) -> std::process::Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).await.unwrap();
    tokio::time::timeout(Duration::from_secs(45), child.wait_with_output())
        .await
        .unwrap()
        .unwrap()
}
fn config(path: &Path, client: &ClientConfig) {
    init::private_write(path, toml::to_string_pretty(client).unwrap().as_bytes()).unwrap();
}
async fn cli(path: &Path, operation: &Operation) -> Result<Outcome, Error> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    command
        .args(["request", "--config"])
        .arg(path)
        .arg("--envelope");
    let result = run(command, &serde_json::to_vec(operation).unwrap()).await;
    let diagnostic = String::from_utf8_lossy(&result.stderr);
    assert!(!diagnostic.contains(&ClientConfig::load(path).unwrap().credential));
    let response: Response = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result.status.success(), response.result.is_ok());
    response.result
}
async fn http(
    client: &reqwest::Client,
    f: &agent::Fixture,
    token: &str,
    operation: &Operation,
) -> Result<Outcome, Error> {
    client
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(token)
        .json(operation)
        .send()
        .await
        .unwrap()
        .json::<Response>()
        .await
        .unwrap()
        .result
}
async fn native(client: &mut ConnectClient, operation: &Operation) -> Result<Outcome, Error> {
    match client.request(operation.clone()).await {
        Ok(outcome) => Ok(outcome),
        Err(ClientError::Application(error)) => Err(error),
        Err(error) => panic!("native transport failed: {error}"),
    }
}
async fn parity(
    f: &agent::Fixture,
    client: &mut ConnectClient,
    mcp: &mcp::Client,
    http_client: &reqwest::Client,
    path: &Path,
    token: &str,
    operation: &Operation,
) -> Result<Outcome, Error> {
    let expected = native(client, operation).await;
    assert_eq!(http(http_client, f, token, operation).await, expected);
    assert_eq!(cli(path, operation).await, expected);
    let response = mcp::operation(mcp, operation).await;
    let actual = if response.is_error == Some(true) {
        Err(mcp::error(response))
    } else {
        Ok(mcp::outcome(response))
    };
    assert_eq!(actual, expected);
    expected
}
async fn ordinary() -> agent::Fixture {
    let mut f = agent::Fixture::start().await;
    f.agent = f.issue_as_operator(
        &f.agent,
        Scope::communications(vec![f.peer.identity_id.clone()]),
    );
    f.peer = f.issue_as_operator(
        &f.peer,
        Scope::communications(vec![f.agent.identity_id.clone()]),
    );
    f
}

fn session(outcome: &Outcome) -> &connect_protocol::Session {
    match outcome {
        Outcome::Session(value) => value,
        _ => panic!("Session response"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audio_video_session_receipts_match_eight_public_entry_points() {
    let mut f = ordinary().await;
    let temp = tempfile::tempdir().unwrap();
    let owner_path = temp.path().join("owner.toml");
    let peer_path = temp.path().join("peer.toml");
    config(&owner_path, &f.agent);
    config(&peer_path, &f.peer);
    let http_client = tokio::task::spawn_blocking(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let owner_mcp = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let peer_mcp = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let mut owner = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let cid = owner
        .create("av-parity-conversation", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let other_cid = owner
        .create("av-parity-other", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let mut records = Vec::new();
    // Each row first executes through Rust, then replays through HTTP, CLI and MCP.
    // The exact same rows subsequently replay through both transports of both SDKs.
    for ending in ["end", "reject", "cancel"] {
        let invite = Operation::SessionInvite {
            request_id: format!("av-{ending}-invite"),
            conversation_id: cid.clone(),
            participant_id: f.peer.identity_id.clone(),
            media: vec!["video".into(), "audio".into()],
        };
        let outcome = parity(
            &f,
            &mut owner,
            &owner_mcp,
            &http_client,
            &owner_path,
            &f.agent.credential,
            &invite,
        )
        .await
        .unwrap();
        let invited = session(&outcome).clone();
        assert_eq!(invited.media, ["audio", "video"]);
        assert_eq!(invited.state, "invited");
        assert_eq!(invited.conversation_id, cid);
        records.push(json!({"actor":"owner","operation":invite,"outcome":outcome}));
        let mut controls = vec![
            (
                false,
                Operation::SessionAccept {
                    request_id: format!("av-{ending}-wrong-actor"),
                    conversation_id: cid.clone(),
                    session_id: invited.id.clone(),
                },
                Some("forbidden"),
            ),
            (
                true,
                Operation::SessionGet {
                    conversation_id: other_cid.clone(),
                    session_id: invited.id.clone(),
                },
                Some("forbidden"),
            ),
            (
                false,
                Operation::SessionInvite {
                    request_id: format!("av-{ending}-invite"),
                    conversation_id: cid.clone(),
                    participant_id: f.peer.identity_id.clone(),
                    media: vec!["audio".into()],
                },
                Some("conflict"),
            ),
        ];
        match ending {
            "end" => {
                controls.push((
                    true,
                    Operation::SessionAccept {
                        request_id: "av-accept".into(),
                        conversation_id: cid.clone(),
                        session_id: invited.id.clone(),
                    },
                    None,
                ));
                controls.push((
                    false,
                    Operation::SessionEnd {
                        request_id: "av-end".into(),
                        conversation_id: cid.clone(),
                        session_id: invited.id.clone(),
                        reason: Some("finished video discussion".into()),
                    },
                    None,
                ));
            }
            "reject" => controls.push((
                true,
                Operation::SessionReject {
                    request_id: "av-reject".into(),
                    conversation_id: cid.clone(),
                    session_id: invited.id.clone(),
                    reason: Some("unavailable".into()),
                },
                None,
            )),
            _ => controls.push((
                false,
                Operation::SessionCancel {
                    request_id: "av-cancel".into(),
                    conversation_id: cid.clone(),
                    session_id: invited.id.clone(),
                },
                None,
            )),
        }
        controls.push((
            false,
            Operation::SessionGet {
                conversation_id: cid.clone(),
                session_id: invited.id.clone(),
            },
            None,
        ));
        for (by_peer, operation, error) in controls {
            let (client, mcp, path, token, actor) = if by_peer {
                (&mut peer, &peer_mcp, &peer_path, &f.peer.credential, "peer")
            } else {
                (
                    &mut owner,
                    &owner_mcp,
                    &owner_path,
                    &f.agent.credential,
                    "owner",
                )
            };
            let result = parity(&f, client, mcp, &http_client, path, token, &operation).await;
            if let Some(code) = error {
                assert_eq!(result.unwrap_err().code, code);
                records.push(json!({"actor":actor,"operation":operation,"error":code}));
            } else {
                let outcome = result.unwrap();
                let current = session(&outcome);
                assert_eq!(current.media, ["audio", "video"]);
                let expected_state = match &operation {
                    Operation::SessionAccept { .. } => "accepted",
                    Operation::SessionEnd { .. } => "ended",
                    Operation::SessionReject { .. } => "rejected",
                    Operation::SessionCancel { .. } => "cancelled",
                    Operation::SessionGet { .. } => match ending {
                        "end" => "ended",
                        "reject" => "rejected",
                        _ => "cancelled",
                    },
                    _ => panic!("unexpected successful Session control"),
                };
                assert_eq!(current.state, expected_state);
                match &operation {
                    Operation::SessionEnd { reason, .. }
                    | Operation::SessionReject { reason, .. } => {
                        assert_eq!(&current.ended_reason, reason)
                    }
                    _ => {}
                }
                records.push(json!({"actor":actor,"operation":operation,"outcome":outcome}));
            }
        }
    }
    for language in ["python", "typescript"] {
        let generated =
            sdk_records(language, &f, &owner_path, &peer_path, &records, &cid, false).await;
        assert_eq!(
            generated.len(),
            8,
            "Both SDK transports must originate a complete AV lifecycle"
        );
        // SDK-origin writes are checked against all other public surfaces too.
        for record in &generated {
            let operation: Operation = serde_json::from_value(record["operation"].clone()).unwrap();
            let expected: Outcome = serde_json::from_value(record["outcome"].clone()).unwrap();
            let (client, mcp, path, token) = if record["actor"] == "peer" {
                (&mut peer, &peer_mcp, &peer_path, &f.peer.credential)
            } else {
                (&mut owner, &owner_mcp, &owner_path, &f.agent.credential)
            };
            assert_eq!(
                parity(&f, client, mcp, &http_client, path, token, &operation)
                    .await
                    .unwrap(),
                expected
            );
        }
        records.extend(generated);
    }
    assert_eq!(records.len(), 35);
    owner.close().await;
    peer.close().await;
    owner_mcp.cancel().await.unwrap();
    peer_mcp.cancel().await.unwrap();
    // A formerly successful receipt cannot bypass a replacement credential's
    // current recipient boundary after the original credentials are revoked.
    f.revoke(&f.agent.credential_id);
    f.revoke(&f.peer.credential_id);
    f.agent = f.issue_as_operator(&f.agent, Scope::communications(vec![]));
    f.peer = f.issue_as_operator(&f.peer, Scope::communications(vec![]));
    let owner_path = temp.path().join("replacement-owner.toml");
    let peer_path = temp.path().join("replacement-peer.toml");
    config(&owner_path, &f.agent);
    config(&peer_path, &f.peer);
    let mut owner = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let owner_mcp = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let peer_mcp = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let denied = records.iter().filter(|record|record.get("outcome").is_some()).map(|record| json!({"actor":record["actor"], "operation":record["operation"], "error":"forbidden"})).collect::<Vec<_>>();
    assert_eq!(
        denied.len(),
        26,
        "Every historical successful row must be refused"
    );
    for record in &denied {
        let operation = serde_json::from_value(record["operation"].clone()).unwrap();
        let (client, mcp, path, token) = if record["actor"] == "peer" {
            (&mut peer, &peer_mcp, &peer_path, &f.peer.credential)
        } else {
            (&mut owner, &owner_mcp, &owner_path, &f.agent.credential)
        };
        assert_eq!(
            parity(&f, client, mcp, &http_client, path, token, &operation)
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    for language in ["python", "typescript"] {
        assert!(
            sdk_records(language, &f, &owner_path, &peer_path, &denied, &cid, true)
                .await
                .is_empty()
        );
    }
    owner.close().await;
    peer.close().await;
    owner_mcp.cancel().await.unwrap();
    peer_mcp.cancel().await.unwrap();
    f.close().await;
}

async fn sdk_records(
    language: &str,
    f: &agent::Fixture,
    owner_path: &Path,
    peer_path: &Path,
    records: &[serde_json::Value],
    cid: &str,
    replay_only: bool,
) -> Vec<serde_json::Value> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (executable, script) = if language == "python" {
        ("python3", "session_probe.py")
    } else {
        ("node", "session_probe.mjs")
    };
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(root.join("sdk/tests").join(script))
        .arg(owner_path)
        .arg(peer_path)
        .arg(format!("http://{}", f.server.http_address.unwrap()))
        .arg(env!("CARGO_BIN_EXE_connect"))
        .env("RUST_LOG", "debug");
    if language == "python" {
        command.env("PYTHONPATH", root.join("sdk/python"));
    }
    let input = json!({"owner":f.agent.credential,"peer":f.peer.credential,"records":records,"conversation_id":cid,"participant_id":f.peer.identity_id,"replay_only":replay_only});
    let output = run(command, &serde_json::to_vec(&input).unwrap()).await;
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(&f.agent.credential));
        assert!(!text.contains(&f.peer.credential));
    }
    assert!(
        output.status.success(),
        "{language}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
