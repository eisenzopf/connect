#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_client::{ClientConfig, ConnectClient};
use connect_connectors::{Telnyx, Vapi};
use connect_protocol::{Operation, Response};
use connect_server::{cli::init, config::ServerConfig, providers::Worker, runtime::RunningServer};
use connect_store::Route;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

async fn callback(
    url: &str,
    key: &SigningKey,
    value: &Value,
    timestamp: i64,
    tampered: bool,
) -> reqwest::Response {
    let body = serde_json::to_vec(value).unwrap();
    let ts = timestamp.to_string();
    let signed = [ts.as_bytes(), b"|", &body].concat();
    let sig = key.sign(&signed);
    let body = if tampered { b"{}".to_vec() } else { body };
    reqwest::Client::new()
        .post(url)
        .header("telnyx-signature-ed25519", STANDARD.encode(sig.to_bytes()))
        .header("telnyx-timestamp", ts)
        .body(body)
        .send()
        .await
        .unwrap()
}
#[tokio::test(flavor = "multi_thread")]
async fn signed_sms_vapi_reply_delivery_and_http_quic_replay_share_one_journal() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let key = SigningKey::from_bytes(&[7; 32]);
    std::env::set_var(
        "TELNYX_PUBLIC_KEY",
        STANDARD.encode(key.verifying_key().to_bytes()),
    );
    let server = RunningServer::start_interfaces(
        ServerConfig::load(&dir.join("server.toml")).unwrap(),
        Some("127.0.0.1:0".parse().unwrap()),
        Some("127.0.0.1:0".parse().unwrap()),
        false,
    )
    .await
    .unwrap();
    std::env::remove_var("TELNYX_PUBLIC_KEY");
    let mut app_config = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
    app_config.endpoint = server.address;
    let bot_config = ClientConfig::load(&dir.join("agent.toml")).unwrap();
    let operator = ClientConfig::load(&dir.join("operator.toml")).unwrap();
    let authority = server.store.authenticate(&operator.credential).unwrap();
    let mut app = ConnectClient::open(app_config.clone()).await.unwrap();
    let conv = app
        .create("conversation", vec![bot_config.identity_id.clone()])
        .await
        .unwrap();
    let capabilities = app
        .capabilities("connect:agent@local", Some(&conv.id))
        .await
        .unwrap();
    assert!(capabilities
        .routes
        .iter()
        .any(|r| r.media.contains(&"files".into())));
    assert!(app
        .context_deliveries(&conv.id, None, 10)
        .await
        .unwrap()
        .deliveries
        .is_empty());
    let route = Route {
        id: "vapi".into(),
        identity_id: bot_config.identity_id.clone(),
        credential_id: bot_config.credential_id,
        conversation_id: conv.id.clone(),
        connector: "vapi".into(),
        target: "12345678-1234-1234-1234-123456789012".into(),
        source: None,
        enabled: true,
    };
    server
        .store
        .configure_route(&authority.credential_id, &route)
        .unwrap();
    server
        .store
        .configure_route(
            &authority.credential_id,
            &Route {
                id: "sms".into(),
                identity_id: app_config.identity_id.clone(),
                credential_id: app_config.credential_id.clone(),
                conversation_id: conv.id.clone(),
                connector: "telnyx".into(),
                target: "+15555550101".into(),
                source: Some("+15555550102".into()),
                enabled: true,
            },
        )
        .unwrap();
    server
        .store
        .refresh_sms_grants(
            "fixture",
            &connect_store::SmsGrantSnapshot {
                revision: 1,
                grants: vec![connect_store::SmsGrant {
                    id: "reviewed".into(),
                    phone: "+15555550101".into(),
                    sender: "+15555550102".into(),
                    conversation_id: conv.id.clone(),
                    purpose: "Requested offline fixture".into(),
                    consent_id: "form".into(),
                    disclosure_version: "v1".into(),
                    disclosure_text: "Explicit fixture consent".into(),
                    submitted_at: 1700000000,
                    approved_at: 1700000001,
                    expires_at: 4000000000,
                }],
            },
        )
        .unwrap();
    let mock = MockServer::start().await;
    mount_assistant(&mock).await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .and(body_partial_json(json!({"input":"help with SMS question"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id":"chat-1","output":[{"role":"assistant","content":"SMS answer"}]}),
        ))
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/messages"))
        .and(body_partial_json(json!({"text":"SMS answer"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{"id":"sms-1"}})))
        .expect(1)
        .mount(&mock)
        .await;
    let worker = Worker::with_drivers(
        server.service.clone(),
        vec![
            Arc::new(Vapi::new("fake", &mock.uri()).unwrap()),
            Arc::new(Telnyx::new("fake", &mock.uri()).unwrap()),
        ],
    )
    .unwrap();
    let base = format!("http://{}", server.http_address.unwrap());
    let webhook_base = format!("http://{}", server.webhook_address.unwrap());
    let webhook = format!("{webhook_base}/webhooks/telnyx");
    assert_eq!(
        reqwest::Client::new()
            .post(format!("{webhook_base}/v1/operations"))
            .json(&Operation::Health {})
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let timestamp = chrono::Utc::now().timestamp();
    for (index, text) in ["STOP", "stop all", "HELP", "START", "custom help"]
        .into_iter()
        .enumerate()
    {
        let mut ignored = json!({"data":{"id":format!("ignored-{index}"),"event_type":"message.received","payload":{"from":{"phone_number":"+15555550101"},"to":[{"phone_number":"+15555550102"}],"text":text}}});
        ignored["data"]["occurred_at"] = json!(chrono::DateTime::from_timestamp(
            timestamp - 10 + index as i64,
            0
        )
        .unwrap()
        .to_rfc3339());
        if index == 4 {
            ignored["data"]["payload"]["autoresponse_type"] = json!("HELP");
        }
        assert_eq!(
            callback(&webhook, &key, &ignored, timestamp, false)
                .await
                .status(),
            204
        );
    }
    assert!(app
        .history(&conv.id, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    let event = json!({"data":{"id":"incoming-1","event_type":"message.received","payload":{"id":"sms-incoming","from":{"phone_number":"+15555550101"},"to":[{"phone_number":"+15555550102"}],"text":"help with SMS question"}}});
    assert_eq!(
        callback(&webhook, &key, &event, timestamp, true)
            .await
            .status(),
        401
    );
    assert_eq!(
        callback(&webhook, &key, &event, timestamp - 600, false)
            .await
            .status(),
        401
    );
    assert_eq!(
        callback(&webhook, &key, &event, timestamp, false)
            .await
            .status(),
        204
    );
    assert_eq!(
        callback(&webhook, &key, &event, timestamp, false)
            .await
            .status(),
        204
    );
    let mut retried = event.clone();
    retried["meta"] = json!({"attempt":2,"delivered_to":"https://fixture.invalid/webhooks/telnyx"});
    assert_eq!(
        callback(&webhook, &key, &retried, timestamp, false)
            .await
            .status(),
        204
    );
    retried["data"]["payload"]["text"] = json!("changed event body");
    assert_eq!(
        callback(&webhook, &key, &retried, timestamp, false)
            .await
            .status(),
        400
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if app
                .provider_status(&conv.id)
                .await
                .unwrap()
                .iter()
                .any(|j| j.state == "accepted")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let messages = app.history(&conv.id, None, 100).await.unwrap().messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].body, "help with SMS question");
    assert_eq!(messages[1].sender, bot_config.identity_id);
    let finalized = json!({"data":{"id":"delivery","event_type":"message.finalized","payload":{"id":"sms-1","to":[{"status":"delivered"}]}}});
    assert_eq!(
        callback(&webhook, &key, &finalized, timestamp, false)
            .await
            .status(),
        204
    );
    assert!(app
        .provider_status(&conv.id)
        .await
        .unwrap()
        .iter()
        .any(|j| j.state == "delivered"));
    let http = reqwest::Client::new();
    let op = Operation::Create {
        request_id: "http-create".into(),
        participants: vec![],
    };
    let response: Response = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(&app_config.credential)
        .json(&op)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        response.result.unwrap(),
        app.request(op.clone()).await.unwrap()
    );
    assert_eq!(
        http.post(format!("{base}/v1/operations"))
            .json(&op)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.post(format!("{base}/v1/operations"))
            .bearer_auth(&app_config.credential)
            .body("x".repeat(70_000))
            .header("content-type", "application/json")
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    worker.shutdown().await.unwrap();
    app.close().await;
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn owned_worker_shutdown_records_uncertainty_and_restart_does_not_resubmit() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start(ServerConfig::load(&dir.join("server.toml")).unwrap())
        .await
        .unwrap();
    let mut config = ClientConfig::load(&dir.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let bot = ClientConfig::load(&dir.join("agent.toml")).unwrap();
    let op = ClientConfig::load(&dir.join("operator.toml")).unwrap();
    let actor = server.store.authenticate(&op.credential).unwrap();
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let conv = client
        .create("create", vec![bot.identity_id.clone()])
        .await
        .unwrap();
    // Provider fixture setup is independent of the native connection. Keep it
    // closed until the worker is ready so setup cannot exhaust its idle budget.
    client.close().await;
    server
        .store
        .configure_route(
            &actor.credential_id,
            &Route {
                id: "vapi".into(),
                identity_id: bot.identity_id,
                credential_id: bot.credential_id,
                conversation_id: conv.id.clone(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
            },
        )
        .unwrap();
    let mock = MockServer::start().await;
    mount_assistant(&mock).await;
    Mock::given(method("POST"))
        .and(path("/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(60))
                .set_body_json(json!({"id":"late","output":[{"content":"late"}]})),
        )
        .expect(1)
        .mount(&mock)
        .await;
    let driver = Arc::new(Vapi::new("fake", &mock.uri()).unwrap());
    let worker = Worker::with_drivers(server.service.clone(), vec![driver.clone()]).unwrap();
    client = ConnectClient::open(config.clone()).await.unwrap();
    client.send("send", &conv.id, "question").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !mock
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|request| request.method == "POST")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    // Draining the deliberately stalled provider can outlast QUIC's idle
    // timeout. Reconnect for inspection instead of reusing an idle transport.
    client.close().await;
    worker.shutdown().await.unwrap();
    client = ConnectClient::open(config).await.unwrap();
    assert_eq!(
        client.provider_status(&conv.id).await.unwrap()[0].state,
        "uncertain"
    );
    let worker = Worker::with_drivers(server.service.clone(), vec![driver]).unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        mock.received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    worker.shutdown().await.unwrap();
    client.close().await;
    server.shutdown().await.unwrap();
}

async fn mount_assistant(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/assistant/12345678-1234-1234-1234-123456789012"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id":"12345678-1234-1234-1234-123456789012",
            "model":{"messages":[{"role":"system","content":"History: {{connect_context}}"}]}
        })))
        .mount(server)
        .await;
}
