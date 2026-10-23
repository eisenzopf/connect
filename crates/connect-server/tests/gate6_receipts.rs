//! All current facades retain the same self-reported receipt and current preferences.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{receipts::*, ConnectClient};
use connect_protocol::{messages::*, Operation, Outcome, Response};
use connect_store::Scope;
use serde_json::json;
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recipient_receipts_and_opt_in_read_reports_share_native_http_mcp_and_sdk_policy() {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, Scope::collaboration(vec!["*".into()]));
    let peer = f.issue_as_operator(&f.peer, Scope::collaboration(vec!["*".into()]));
    let foreign = f.issue_as_operator(&f.foreign, Scope::collaboration(vec!["*".into()]));
    let mut owner = ConnectClient::open(config.clone()).await.unwrap();
    let mut recipient = ConnectClient::open(peer.clone()).await.unwrap();
    let cid = owner
        .create(
            "receipts-room",
            vec![peer.identity_id.clone(), foreign.identity_id.clone()],
        )
        .await
        .unwrap()
        .id;
    let participant = owner
        .capabilities("connect:agent@local", Some(&cid))
        .await
        .unwrap()
        .participant_id
        .unwrap();
    let m = owner
        .send_message(
            "private-question",
            &cid,
            MessageDraft::text(
                MessageAudience::Participants(vec![participant]),
                "private question",
            ),
        )
        .await
        .unwrap()
        .message_id;
    assert!(!recipient.receipt_preferences(&cid).await.unwrap().read);
    assert_eq!(
        owner
            .message_receipts(&cid, &m, None, 100)
            .await
            .unwrap()
            .receipts[0]
            .state,
        DeliveryState::Queued
    );
    let base = format!("http://{}", f.server.http_address.unwrap());
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let op = Operation::MessageReceipt {
        request_id: "same-delivered".into(),
        conversation_id: cid.clone(),
        message_id: m.clone(),
        state: ClientReceiptState::Delivered,
    };
    let raw: serde_json::Value = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&peer.credential)
        .json(&op)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let result: Response = serde_json::from_value(raw.clone()).unwrap();
    let expected = Outcome::ReceiptAcknowledged(
        recipient
            .message_receipt("same-delivered", &cid, &m, ClientReceiptState::Delivered)
            .await
            .unwrap(),
    );
    assert_eq!(result.result.unwrap(), expected);
    let client = mcp::client(f.server.mcp_address.unwrap(), &peer.credential).await;
    assert_eq!(mcp::outcome(mcp::operation(&client, &op).await), expected);
    assert!(matches!(
        mcp::outcome(
            mcp::operation(
                &client,
                &Operation::ReceiptPreferencesSet {
                    request_id: "read-on".into(),
                    conversation_id: cid.clone(),
                    delivery: true,
                    read: true
                }
            )
            .await
        ),
        Outcome::ReceiptPreferences(ReceiptPreferences { read: true, .. })
    ));
    let read = Operation::MessageReceipt {
        request_id: "same-read".into(),
        conversation_id: cid.clone(),
        message_id: m.clone(),
        state: ClientReceiptState::Read,
    };
    assert!(matches!(
        mcp::outcome(mcp::operation(&client, &read).await),
        Outcome::ReceiptAcknowledged(ReceiptAcknowledgement {
            recorded: Some(DeliveryState::Read),
            ..
        })
    ));
    let query = Operation::MessageReceipts {
        conversation_id: cid.clone(),
        message_id: m.clone(),
        after: None,
        limit: 100,
    };
    assert!(
        matches!(mcp::outcome(mcp::operation(&client,&query).await),Outcome::MessageReceipts(p) if p.receipts.len()==1 && p.receipts[0].state==DeliveryState::Read)
    );
    client.cancel().await.unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let data=json!({"base":base,"token":config.credential,"peer_token":peer.credential,"foreign_token":foreign.credential,"cid":cid,"message":m}).to_string();
    tokio::task::spawn_blocking(move || {
        for (program, script) in [
            ("node", "sdk/tests/receipts_probe.mjs"),
            ("python3", "sdk/tests/receipts_probe.py"),
        ] {
            let result = std::process::Command::new(program)
                .current_dir(&root)
                .env("PYTHONPATH", root.join("sdk/python"))
                .arg(script)
                .arg(&data)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "SDK: {}",
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
    let body: serde_json::Value = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&config.credential)
        .json(&query)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let result: Response = serde_json::from_value(body.clone()).unwrap();
    assert!(
        matches!(result.result.unwrap(),Outcome::MessageReceipts(p) if p.receipts[0].state==DeliveryState::Read)
    );
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_ATTACHMENT_SAMPLES") {
        let dir = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, request, response) in [
            ("receipt-ack", serde_json::to_value(&op).unwrap(), raw),
            ("receipts", serde_json::to_value(&query).unwrap(), body),
        ] {
            std::fs::write(
                dir.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&json!({"request":request,"response":response})).unwrap(),
            )
            .unwrap();
        }
    }
    recipient
        .set_receipt_preferences("off", &cid, false, false)
        .await
        .unwrap();
    assert!(owner
        .message_receipts(&cid, &m, None, 100)
        .await
        .unwrap()
        .receipts
        .is_empty());
    assert!(owner
        .inbox(None, 100, 0)
        .await
        .unwrap()
        .events
        .iter()
        .all(|e| e.receipt.is_none()));
    assert_eq!(
        owner.history(&cid, None, 100).await.unwrap().messages.len(),
        1
    );
    recipient.close().await;
    owner.close().await;
    f.close().await;
}
