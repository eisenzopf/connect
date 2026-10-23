//! Actual native SDK, HTTP, CLI, MCP and JS/Python SDK replay one typed intent.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::ConnectClient;
use connect_protocol::{messages::*, Operation, Outcome, Response};
use connect_store::Scope;
use serde_json::json;
use std::process::{Command, Stdio};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn addressed_binary_message_has_identical_receipt_and_private_history_across_current_facades()
{
    let f = agent::Fixture::start().await;
    let mut config = f.agent.clone();
    let key = f
        .server
        .store
        .issue(
            &config.identity_id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    config.credential = key.token.clone();
    config.credential_id = key.id.clone();
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let cid = native
        .create(
            "addressed-room",
            vec![f.peer.identity_id.clone(), f.foreign.identity_id.clone()],
        )
        .await
        .unwrap()
        .id;
    let participant = native
        .capabilities("connect:agent@local", Some(&cid))
        .await
        .unwrap()
        .participant_id
        .unwrap();
    let mut message =
        MessageDraft::text(MessageAudience::Participants(vec![participant]), "AP9hAA==");
    message.body_encoding = BodyEncoding::Base64;
    message.content_type = "application/octet-stream".into();
    message.label = "private-binary".into();
    message.msg_id = Some("shared-binary-id".into());
    let request_id = "same-message";
    let receipt = native
        .send_message(request_id, &cid, message.clone())
        .await
        .unwrap();
    let operation = Operation::Send {
        request_id: request_id.into(),
        conversation_id: cid.clone(),
        message: message.clone(),
    };
    let expected = Outcome::Accepted {
        receipt: Some(receipt.receipt.clone()),
        request_id: request_id.into(),
        conversation_id: cid.clone(),
        message_id: receipt.message_id.clone(),
    };
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://{}", f.server.http_address.unwrap());
    let response = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&config.credential)
        .json(&operation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let response: Response = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(response.result.unwrap(), expected);
    let client = mcp::client(f.server.mcp_address.unwrap(), &config.credential).await;
    assert_eq!(
        mcp::outcome(mcp::operation(&client, &operation).await),
        expected
    );
    client.cancel().await.unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let data=json!({"base":base,"token":config.credential,"cid":cid,"request_id":request_id,"message":message}).to_string();
    let path = root.clone();
    tokio::task::spawn_blocking(move || {
        for (program, script) in [
            ("node", "sdk/tests/messages_probe.mjs"),
            ("python3", "sdk/tests/messages_probe.py"),
        ] {
            let result = Command::new(program)
                .current_dir(&path)
                .env("PYTHONPATH", path.join("sdk/python"))
                .arg(script)
                .arg(&data)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "SDK probe failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
                json!({"passed":true})
            );
        }
    })
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("client.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let cli = operation.clone();
    let result = tokio::task::spawn_blocking(move || {
        use std::io::Write;
        let mut child = Command::new(env!("CARGO_BIN_EXE_connect"))
            .args(["request", "--config"])
            .arg(config_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(serde_json::to_string(&cli).unwrap().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap();
    assert!(result.status.success(), "CLI request failed");
    let from_cli: Outcome = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(from_cli, expected);
    let history = native.history(&cid, None, 100).await.unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(history.messages[0].body, "AP9hAA==");
    let mut peer = f.peer_client().await;
    assert_eq!(
        peer.history(&cid, None, 100).await.unwrap().messages,
        history.messages
    );
    let foreign_key = f
        .server
        .store
        .issue(
            &f.foreign.identity_id,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let Outcome::History { messages, .. } = f
        .server
        .service
        .execute(
            foreign_key.id,
            Operation::History {
                conversation_id: cid.clone(),
                after: None,
                limit: 100,
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(messages.is_empty());
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_MESSAGE_SAMPLES") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("send.json"),
            serde_json::to_vec_pretty(&json!({"request":operation,"response":body})).unwrap(),
        )
        .unwrap();
        let history_operation = Operation::History {
            conversation_id: cid,
            after: None,
            limit: 100,
        };
        let history_body: serde_json::Value = http
            .post(format!("{base}/v1/operations"))
            .bearer_auth(&config.credential)
            .json(&history_operation)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let decoded: Response = serde_json::from_value(history_body.clone()).unwrap();
        assert!(
            matches!(decoded.result.unwrap(),Outcome::History{messages,..} if messages==history.messages)
        );
        std::fs::write(
            directory.join("history.json"),
            serde_json::to_vec_pretty(
                &json!({"request":history_operation,"response":history_body}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    peer.close().await;
    native.close().await;
    f.server.shutdown().await.unwrap();
}
