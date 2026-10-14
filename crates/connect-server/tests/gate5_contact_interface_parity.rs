//! Source-only public contact parity; disposable state, no provider operations.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{contacts::OwnershipState, Error, Operation, Outcome, Response};
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
    if let Operation::ContactPairingComplete { challenge, .. } = operation {
        assert!(
            !diagnostic.contains(challenge),
            "CLI leaked challenge in diagnostics"
        );
    }
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
fn pairing(outcome: Outcome) -> connect_protocol::contacts::Pairing {
    match outcome {
        Outcome::ContactPairing(value) => value,
        _ => panic!("pairing response"),
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_http_json_cli_mcp_share_receipts_current_authority_and_no_sms_consent() {
    let f = ordinary().await;
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
    let names = owner_mcp
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>();
    for name in ["start", "complete", "revoke"] {
        assert!(names.contains(&format!("connect_contact_pairing_{name}")));
    }
    assert!(names.contains(&"connect_contact_state".into()));
    assert!(
        !names.contains(&"connect_task_create".into()),
        "pairing needs no optional task authority"
    );
    mcp::invalid(&owner_mcp, "connect_contact_pairing_complete", json!({"request_id":"schema", "pairing_id":"unknown", "challenge":"private-schema-challenge", "identity_id":f.peer.identity_id})).await;
    mcp::invalid(
        &owner_mcp,
        "connect_contact_state",
        json!({"peer_identity_id":f.peer.identity_id, "task_id":"unnecessary"}),
    )
    .await;
    let mut owner = f.agent_client().await;
    let mut peer = f.peer_client().await;
    for operation in [
        Operation::ContactState {
            peer_identity_id: f.foreign.identity_id.clone(),
        },
        Operation::ContactPairingStart {
            request_id: "contact-forbidden-peer".into(),
            peer_identity_id: f.foreign.identity_id.clone(),
            expires_at: chrono::Utc::now().timestamp() + 300,
        },
    ] {
        assert_eq!(
            parity(
                &f,
                &mut owner,
                &owner_mcp,
                &http_client,
                &owner_path,
                &f.agent.credential,
                &operation
            )
            .await
            .unwrap_err()
            .code,
            "forbidden"
        );
    }
    let start = Operation::ContactPairingStart {
        request_id: "contact-parity-start".into(),
        peer_identity_id: f.peer.identity_id.clone(),
        expires_at: chrono::Utc::now().timestamp() + 300,
    };
    let pending = pairing(
        parity(
            &f,
            &mut owner,
            &owner_mcp,
            &http_client,
            &owner_path,
            &f.agent.credential,
            &start,
        )
        .await
        .unwrap(),
    );
    assert_eq!(pending.state, OwnershipState::Pending);
    let challenge = pending.challenge.clone().unwrap();
    assert!(
        !format!("{start:?} {pending:?}").contains(&challenge),
        "Debug must redact challenge"
    );
    let mut changed = start.clone();
    if let Operation::ContactPairingStart { expires_at, .. } = &mut changed {
        *expires_at += 1;
    }
    assert_eq!(
        parity(
            &f,
            &mut owner,
            &owner_mcp,
            &http_client,
            &owner_path,
            &f.agent.credential,
            &changed
        )
        .await
        .unwrap_err()
        .code,
        "conflict"
    );
    let wrong = Operation::ContactPairingComplete {
        request_id: "contact-parity-wrong".into(),
        pairing_id: pending.id.clone(),
        challenge: "wrong-private-challenge".into(),
    };
    let attempted = pairing(
        parity(
            &f,
            &mut peer,
            &peer_mcp,
            &http_client,
            &peer_path,
            &f.peer.credential,
            &wrong,
        )
        .await
        .unwrap(),
    );
    assert_eq!(attempted.state, OwnershipState::Pending);
    assert!(attempted.challenge.is_none());
    let complete = Operation::ContactPairingComplete {
        request_id: "contact-parity-complete".into(),
        pairing_id: pending.id.clone(),
        challenge,
    };
    if let Operation::ContactPairingComplete { challenge, .. } = &complete {
        assert!(!format!("{complete:?}").contains(challenge));
    }
    assert_eq!(
        parity(
            &f,
            &mut owner,
            &owner_mcp,
            &http_client,
            &owner_path,
            &f.agent.credential,
            &complete
        )
        .await
        .unwrap_err()
        .code,
        "forbidden"
    );
    let verified = pairing(
        parity(
            &f,
            &mut peer,
            &peer_mcp,
            &http_client,
            &peer_path,
            &f.peer.credential,
            &complete,
        )
        .await
        .unwrap(),
    );
    assert_eq!(verified.state, OwnershipState::VerifiedNative);
    assert!(verified.challenge.is_none());
    let contact = Operation::ContactState {
        peer_identity_id: f.peer.identity_id.clone(),
    };
    assert!(
        matches!(parity(&f, &mut owner, &owner_mcp, &http_client, &owner_path, &f.agent.credential, &contact).await.unwrap(), Outcome::Contact(value) if value.ownership == OwnershipState::VerifiedNative && !value.sms_consent && value.verified_at.is_some())
    );
    let mut used = complete.clone();
    if let Operation::ContactPairingComplete { request_id, .. } = &mut used {
        *request_id = "contact-parity-reuse".into();
    }
    assert_eq!(
        parity(
            &f,
            &mut peer,
            &peer_mcp,
            &http_client,
            &peer_path,
            &f.peer.credential,
            &used
        )
        .await
        .unwrap_err()
        .code,
        "pairing-used"
    );
    let revoke = Operation::ContactPairingRevoke {
        request_id: "contact-parity-revoke".into(),
        pairing_id: pending.id,
    };
    assert_eq!(
        pairing(
            parity(
                &f,
                &mut owner,
                &owner_mcp,
                &http_client,
                &owner_path,
                &f.agent.credential,
                &revoke
            )
            .await
            .unwrap()
        )
        .state,
        OwnershipState::Revoked
    );
    assert!(parity(
        &f,
        &mut peer,
        &peer_mcp,
        &http_client,
        &peer_path,
        &f.peer.credential,
        &complete
    )
    .await
    .is_err());
    assert!(
        matches!(parity(&f, &mut owner, &owner_mcp, &http_client, &owner_path, &f.agent.credential, &contact).await.unwrap(), Outcome::Contact(value) if value.ownership == OwnershipState::Revoked && !value.sms_consent)
    );
    owner.close().await;
    peer.close().await;
    owner_mcp.cancel().await.unwrap();
    peer_mcp.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_python_typescript_native_helper_and_http_contact_methods() {
    let f = ordinary().await;
    let temp = tempfile::tempdir().unwrap();
    let owner_path = temp.path().join("owner.toml");
    let peer_path = temp.path().join("peer.toml");
    config(&owner_path, &f.agent);
    config(&peer_path, &f.peer);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let url = format!("http://{}", f.server.http_address.unwrap());
    for (language, executable, script) in [
        ("python", "python3", "contact_probe.py"),
        ("typescript", "node", "contact_probe.mjs"),
    ] {
        let mut command = tokio::process::Command::new(executable);
        command
            .arg(root.join("sdk/tests").join(script))
            .arg(&owner_path)
            .arg(&peer_path)
            .arg(&url)
            .arg(env!("CARGO_BIN_EXE_connect"));
        command.env("RUST_LOG", "debug");
        if language == "python" {
            command.env("PYTHONPATH", root.join("sdk/python"));
        }
        let result = run(
            command,
            &serde_json::to_vec(&json!({"owner":f.agent.credential,"peer":f.peer.credential}))
                .unwrap(),
        )
        .await;
        assert!(
            result.status.success(),
            "{language}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        for bytes in [&result.stdout, &result.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(&f.agent.credential));
            assert!(!text.contains(&f.peer.credential));
            assert!(!text.contains("private-challenge"));
        }
    }
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn credential_revocation_rechecks_pairing_replay_and_observation() {
    let f = ordinary().await;
    let mut owner = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let pending = owner
        .contact_pairing_start(
            "credential-start",
            &f.peer.identity_id,
            chrono::Utc::now().timestamp() + 300,
        )
        .await
        .unwrap();
    peer.contact_pairing_complete(
        "credential-complete",
        &pending.id,
        pending.challenge.as_ref().unwrap(),
    )
    .await
    .unwrap();
    f.revoke(&f.peer.credential_id);
    assert_eq!(
        owner
            .contact_state(&f.peer.identity_id)
            .await
            .unwrap()
            .ownership,
        OwnershipState::Revoked
    );
    owner.close().await;
    peer.close().await;
    let mut replacement = ConnectClient::open(f.issue_as_operator(
        &f.peer,
        Scope::communications(vec![f.agent.identity_id.clone()]),
    ))
    .await
    .unwrap();
    assert!(
        replacement
            .contact_pairing_complete(
                "credential-complete",
                &pending.id,
                pending.challenge.as_ref().unwrap()
            )
            .await
            .is_err(),
        "replacement credential cannot replay possession proof"
    );
    replacement.close().await;
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn original_owner_revocation_denies_completion_receipt_on_all_control_interfaces() {
    let f = ordinary().await;
    let temp = tempfile::tempdir().unwrap();
    let peer_path = temp.path().join("peer.toml");
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
    let peer_mcp = mcp::client(f.server.mcp_address.unwrap(), &f.peer.credential).await;
    let mut owner = f.agent_client().await;
    let mut peer = f.peer_client().await;
    let pending = owner
        .contact_pairing_start(
            "owner-revoke-start",
            &f.peer.identity_id,
            chrono::Utc::now().timestamp() + 300,
        )
        .await
        .unwrap();
    let complete = Operation::ContactPairingComplete {
        request_id: "owner-revoke-complete".into(),
        pairing_id: pending.id,
        challenge: pending.challenge.unwrap(),
    };
    parity(
        &f,
        &mut peer,
        &peer_mcp,
        &http_client,
        &peer_path,
        &f.peer.credential,
        &complete,
    )
    .await
    .unwrap();
    f.revoke(&f.agent.credential_id);
    assert!(parity(
        &f,
        &mut peer,
        &peer_mcp,
        &http_client,
        &peer_path,
        &f.peer.credential,
        &complete
    )
    .await
    .is_err());
    owner.close().await;
    peer.close().await;
    peer_mcp.cancel().await.unwrap();
    f.close().await;
}
