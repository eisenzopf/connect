//! Explicit read services use real HTTP separately from signed native ownership.
use super::*;
use crate::{
    application::{ApplicationClient, ApplicationClientConfig},
    sdk::{self, Client, ClientConfig, RequestOptions},
};
use connect_protocol::{
    receipts::{DeliveryState, ReceiptPage, RecipientReceipt},
    Outcome, Response, PROFILE,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(3)).unwrap()
}
fn fact(message: &str, channel: &str, index: usize) -> RecipientReceipt {
    RecipientReceipt {
        conversation_id: "conv_one".into(),
        message_id: message.into(),
        participant_id: "participant_one".into(),
        membership_epoch: 1,
        channel: channel.into(),
        delivery_id: format!("private-delivery-{index}"),
        state: if channel == "native" {
            DeliveryState::Read
        } else {
            DeliveryState::Accepted
        },
        provenance: if channel == "native" {
            "client_reported"
        } else {
            "provider_observation"
        }
        .into(),
        observed_at: 1,
    }
}
struct Http {
    config: ApplicationClientConfig,
    ready: Arc<Notify>,
    release: Arc<Notify>,
    bodies: Arc<tokio::sync::Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
async fn http() -> Http {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let ready = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let bodies = Arc::new(tokio::sync::Mutex::new(vec![]));
    let entered = ready.clone();
    let gate = release.clone();
    let seen = bodies.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut header = vec![];
            let mut b = [0; 1];
            while !header.ends_with(b"\r\n\r\n") {
                if stream.read_exact(&mut b).await.is_err() {
                    return;
                }
                header.push(b[0]);
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap();
            assert!(header.starts_with("POST /v1/operations HTTP/1.1\r\n"));
            let length = header
                .lines()
                .find_map(|s| {
                    s.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            assert!(length <= 65536);
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let command: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(command["op"], "message_receipts");
            assert_eq!(command["conversation_id"], "conv_one");
            assert_eq!(command["limit"], 100);
            seen.lock().await.push(command.clone());
            let message = command["message_id"].as_str().unwrap();
            let after = command["after"].as_str();
            if message == "held" {
                entered.notify_one();
                gate.notified().await;
            }
            let mut page = ReceiptPage {
                receipts: vec![fact(message, "native", 0)],
                next: None,
            };
            match message {
                "good" => match after {
                    None => page.next = Some("page-one".into()),
                    Some("page-one") => {
                        page.receipts = vec![fact(message, "provider", 1)];
                        page.next = Some("page-two".into());
                    }
                    Some("page-two") => page.receipts.clear(),
                    _ => panic!("bad cursor"),
                },
                "foreign-cid" => page.receipts[0].conversation_id = "foreign".into(),
                "foreign-message" => page.receipts[0].message_id = "foreign".into(),
                "zero-epoch" => page.receipts[0].membership_epoch = 0,
                "unsafe-epoch" => page.receipts[0].membership_epoch = 9_007_199_254_740_992,
                "bad-provenance" => page.receipts[0].provenance = "human_action".into(),
                "bad-time" => page.receipts[0].observed_at = -1,
                "bad-participant" => page.receipts[0].participant_id = "bad participant".into(),
                "duplicate" => page.receipts.push(page.receipts[0].clone()),
                "cycle" => {
                    page.receipts.clear();
                    page.next = Some("same-cursor".into());
                }
                "wide" => page.receipts = (0..101).map(|i| fact(message, "native", i)).collect(),
                "many" => {
                    let index = after.map(|s| s.parse::<usize>().unwrap()).unwrap_or(0);
                    page.receipts = (0..100)
                        .map(|i| fact(message, "native", index * 100 + i))
                        .collect();
                    page.next = Some((index + 1).to_string());
                }
                "refused-after-page" if after.is_none() => page.next = Some("refuse-now".into()),
                _ => (),
            }
            let refused =
                message == "refused" || message == "refused-after-page" && after.is_some();
            let result = if refused {
                Err(connect_protocol::Error::new(
                    "forbidden",
                    "private server detail",
                ))
            } else {
                Ok(Outcome::MessageReceipts(page))
            };
            let mut response = serde_json::to_value(Response {
                profile: PROFILE.into(),
                result,
            })
            .unwrap();
            if message == "wrong-profile" {
                response["profile"] = json!("foreign/1");
            }
            if message == "wrong-outcome" {
                response["result"] = json!({"Ok":{"kind":"accepted","request_id":"not-a-receipt"}});
            }
            let body = serde_json::to_vec(&response).unwrap();
            let header=format!("HTTP/1.1 {}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",if refused{"403 Forbidden"}else{"200 OK"},body.len());
            if stream.write_all(header.as_bytes()).await.is_ok() {
                let _ = stream.write_all(&body).await;
            }
            let _ = stream.shutdown().await;
        }
    });
    Http {
        config: ApplicationClientConfig {
            endpoint,
            credential: "controlled-reader".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
            timeout_ms: 3000,
        },
        ready,
        release,
        bodies,
        task,
    }
}
async fn setup(http: &Http) -> (Client, tokio::task::JoinHandle<()>) {
    let (peer, host, _) = controlled(Fault::SdkRaw).await;
    let native = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let app = ApplicationClient::open(http.config.clone()).await.unwrap();
    (native.with_application_reads(app), host)
}
async fn control(client: &Client) {
    let v = json!({"v":2,"id":"caller-attempt","ts":"2026-01-01T00:00:00Z","type":"operation.get","cid":"conv_one","payload":{"operation_id":"original-send","vendor":{"private":"payload-secret"}}});
    client
        .raw()
        .request(client.raw().validate(v).unwrap(), options())
        .await
        .unwrap();
}
async fn finish(client: &Client, host: tokio::task::JoinHandle<()>, http: Http) {
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
    http.task.abort();
    let _ = http.task.await;
}
#[tokio::test]
async fn receipt_pages_preserve_channels_epochs_provenance_and_do_not_imply_human_action() {
    let http = http().await;
    let (client, host) = setup(&http).await;
    control(&client).await;
    let room = client.conversation_by_id("conv_one".parse().unwrap());
    let receipts = room
        .receipts("good".parse().unwrap(), options())
        .await
        .unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].state, DeliveryState::Read);
    assert_eq!(receipts[0].provenance, "client_reported");
    assert_eq!(receipts[1].state, DeliveryState::Accepted);
    assert_eq!(receipts[1].channel, "provider");
    assert_eq!(receipts[0].membership_epoch, 1);
    assert!(!format!("{:?}", receipts[0]).contains("private-delivery"));
    assert_eq!(http.bodies.lock().await.len(), 3);
    finish(&client, host, http).await;
}
#[tokio::test]
async fn receipt_reads_reject_foreign_malformed_repeating_unbounded_and_refused_partial_pages() {
    let http = http().await;
    let (client, host) = setup(&http).await;
    control(&client).await;
    let room = client.conversation_by_id("conv_one".parse().unwrap());
    for message in [
        "foreign-cid",
        "foreign-message",
        "zero-epoch",
        "unsafe-epoch",
        "bad-provenance",
        "bad-time",
        "bad-participant",
        "wide",
        "wrong-profile",
        "wrong-outcome",
    ] {
        let e = room
            .receipts(message.parse().unwrap(), options())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.certainty),
            (sdk::ErrorKind::Protocol, sdk::Certainty::ReadOnly),
            "{message}"
        );
    }
    for message in ["duplicate", "cycle", "many"] {
        let e = room
            .receipts(message.parse().unwrap(), options())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.certainty),
            (sdk::ErrorKind::ResyncRequired, sdk::Certainty::ReadOnly),
            "{message}"
        );
    }
    let e = room
        .receipts("refused-after-page".parse().unwrap(), options())
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Authentication, sdk::Certainty::Refused)
    );
    assert!(!format!("{e:?}").contains("private server detail"));
    finish(&client, host, http).await;
}
#[tokio::test]
async fn cancelled_or_expired_http_reads_leave_signed_control_and_next_read_usable() {
    for timeout in [false, true] {
        let http = http().await;
        let (client, host) = setup(&http).await;
        let room = client.conversation_by_id("conv_one".parse().unwrap());
        let read = if timeout {
            RequestOptions::new(Duration::from_millis(80)).unwrap()
        } else {
            options()
        };
        let cancel = read.signal.clone();
        let pending =
            tokio::spawn(async move { room.receipts("held".parse().unwrap(), read).await });
        tokio::time::timeout(Duration::from_secs(2), http.ready.notified())
            .await
            .unwrap();
        control(&client).await;
        if !timeout {
            cancel.cancel();
        }
        let e = pending.await.unwrap().unwrap_err();
        assert_eq!(
            e.kind,
            if timeout {
                sdk::ErrorKind::Deadline
            } else {
                sdk::ErrorKind::Cancelled
            }
        );
        assert_eq!(e.certainty, sdk::Certainty::ReadOnly);
        http.release.notify_one();
        assert_eq!(
            client
                .conversation_by_id("conv_one".parse().unwrap())
                .receipts("good".parse().unwrap(), options())
                .await
                .unwrap()
                .len(),
            2
        );
        finish(&client, host, http).await;
    }
}
#[tokio::test]
async fn closing_owned_read_services_cancels_pending_work_and_retains_both_cleanup_joins() {
    let http = http().await;
    let (client, host) = setup(&http).await;
    control(&client).await;
    let room = client.conversation_by_id("conv_one".parse().unwrap());
    let pending =
        tokio::spawn(async move { room.receipts("held".parse().unwrap(), options()).await });
    tokio::time::timeout(Duration::from_secs(2), http.ready.notified())
        .await
        .unwrap();
    let close = options();
    close.signal.cancel();
    assert_eq!(
        client.close(close).await.unwrap_err().kind,
        sdk::ErrorKind::CleanupIncomplete
    );
    let e = pending.await.unwrap().unwrap_err();
    assert_eq!(e.kind, sdk::ErrorKind::Cancelled);
    finish(&client, host, http).await;
}
#[tokio::test]
async fn unconfigured_receipts_are_explicitly_unsupported_without_losing_native_control() {
    let (peer, host, _) = controlled(Fault::SdkRaw).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let e = client
        .conversation_by_id("conv_one".parse().unwrap())
        .receipts("message".parse().unwrap(), options())
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Unsupported, sdk::Certainty::ReadOnly)
    );
    control(&client).await;
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), host)
        .await
        .unwrap()
        .unwrap();
}
fn config() -> ClientConfig {
    ClientConfig {
        tenant_id: "tenant".parse().unwrap(),
        application: None,
        control: CanonicalClientConfig {
            network_profile: NativeNetworkProfile::Local,
            endpoint: "127.0.0.1:9".parse().unwrap(),
            server_name: "localhost".into(),
            trust_certificate: "tls.pem".into(),
            credential: "private-config-secret".into(),
            identity_id: "identity".into(),
            device_id: "device".into(),
            device_kind: "native".into(),
            device_platform: "rust".into(),
            signing_key_id: "client-key".into(),
            signing_algorithm: "EdDSA".into(),
            signing_private_key: "private.pem".into(),
            server_key_id: "server-key".into(),
            server_algorithm: "EdDSA".into(),
            server_public_key: "server.pub".into(),
            server_key_not_before: chrono::Utc::now() - chrono::Duration::minutes(1),
            server_key_expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            timeout_ms: 1000,
        },
    }
}
#[test]
fn configured_native_profiles_resolve_relative_trust_paths_and_redact_credentials() {
    let local = config();
    local.validate().unwrap();
    assert!(!format!("{local:?}").contains("private-config-secret"));
    let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local");
    let dir = tempfile::tempdir_in(scratch).unwrap();
    let path = dir.path().join("sdk.toml");
    std::fs::write(&path, toml::to_string(&local).unwrap()).unwrap();
    let loaded = ClientConfig::load(&path).unwrap();
    assert_eq!(loaded.control.trust_certificate, dir.path().join("tls.pem"));
    assert_eq!(
        loaded.control.signing_private_key,
        dir.path().join("private.pem")
    );
    assert_eq!(
        loaded.control.server_public_key,
        dir.path().join("server.pub")
    );
    let mut remote = local.clone();
    remote.control.endpoint = "192.0.2.20:443".parse().unwrap();
    assert!(remote.validate().is_err());
    remote.control.network_profile = NativeNetworkProfile::Remote;
    remote.validate().unwrap();
}
#[tokio::test]
async fn mixed_or_untrusted_application_configuration_refuses_before_native_io() {
    let mut config = config();
    config.application = Some(ApplicationClientConfig {
        endpoint: "127.0.0.1:9".parse().unwrap(),
        credential: "other-actor".into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
        timeout_ms: 1000,
    });
    let e = Client::connect_configured(config.clone())
        .await
        .err()
        .unwrap();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Configuration, sdk::Certainty::NotSubmitted)
    );
    let credential = config.control.credential.clone();
    config.application.as_mut().unwrap().credential = credential;
    config.application.as_mut().unwrap().endpoint = "192.0.2.20:80".parse().unwrap();
    assert_eq!(
        config.validate().unwrap_err().kind,
        sdk::ErrorKind::Configuration
    );
}

#[tokio::test]
async fn receipt_capacity_bounds_queued_reads_and_close_skips_every_unwritten_request() {
    use std::future::Future;
    let http = http().await;
    let (client, host) = setup(&http).await;
    control(&client).await;
    let room = client.conversation_by_id("conv_one".parse().unwrap());
    let held = room.clone();
    let first =
        tokio::spawn(async move { held.receipts("held".parse().unwrap(), options()).await });
    tokio::time::timeout(Duration::from_secs(2), http.ready.notified())
        .await
        .unwrap();
    let mut queued = Vec::new();
    for index in 0..15 {
        let room = room.clone();
        queued.push(Box::pin(async move {
            room.receipts(format!("queued-{index}").parse().unwrap(), options())
                .await
        }));
    }
    std::future::poll_fn(|cx| {
        for pending in &mut queued {
            assert!(pending.as_mut().poll(cx).is_pending());
        }
        std::task::Poll::Ready(())
    })
    .await;
    let e = room
        .receipts(
            "seventeenth".parse().unwrap(),
            RequestOptions::new(Duration::from_millis(30)).unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Deadline, sdk::Certainty::ReadOnly)
    );
    client.close(options()).await.unwrap();
    assert_eq!(
        first.await.unwrap().unwrap_err().kind,
        sdk::ErrorKind::Cancelled
    );
    for pending in queued {
        assert_eq!(pending.await.unwrap_err().kind, sdk::ErrorKind::Cancelled);
    }
    assert_eq!(http.bodies.lock().await.len(), 1);
    finish(&client, host, http).await;
}
