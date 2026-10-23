//! Actual record writer custody. Large metadata is a privileged, bounded transport
//! fixture in an otherwise real terminal Session; no media capture is asserted.
#[path = "canonical_record_quic.rs"]
mod quic;
use super::*;
use connect_client::{
    application::ApplicationClientConfig,
    canonical::{CanonicalHttpClient, CanonicalRecordSelector},
};
use connect_protocol::{
    session_control::{Action, Request},
    session_decisions::{Decision, Request as DecisionRequest},
};
use connect_store::{
    records::{RecordBytes, RecordMutationKind, RecordScope},
    AuthorityMode, AuthorityScope, AuthorityWait,
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

async fn http(f: &Fixture, credential: &str) -> CanonicalHttpClient {
    CanonicalHttpClient::open(ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: credential.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 8000,
    })
    .await
    .unwrap()
}
struct RecordFixture {
    f: Fixture,
    cid: String,
    sid: String,
    uuid: String,
    own: String,
}
impl RecordFixture {
    async fn new(algorithm: &str, large: bool) -> Self {
        Self::with_capacity(algorithm, large, 8).await
    }
    async fn with_capacity(algorithm: &str, large: bool, capacity: usize) -> Self {
        Self::with_retention(algorithm, large, capacity, None).await
    }
    async fn with_retention(
        algorithm: &str,
        large: bool,
        capacity: usize,
        retention: Option<connect_protocol::record_retention::Policy>,
    ) -> Self {
        let mut f = Fixture::with_membership_scope(algorithm, capacity, true).await;
        if let Some(policy) = retention {
            f.config.record_retention = policy;
            f.server.shutdown().await.unwrap();
            f.server = RunningCanonicalServer::start(f.config.clone())
                .await
                .unwrap();
            f.client.endpoint = f.server.address;
        }
        let mut client = http(&f, &f.client.credential).await;
        let mut create = connect_client::conversations::CreateRequest::persistent(vec![f
            .peer
            .identity_id
            .clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = client
            .create_conversation("http-record-room", create)
            .await
            .unwrap();
        let own = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let peer = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let sid = "http-record-session".to_owned();
        client
            .session_command(Request {
                operation_id: "http-record-invite".into(),
                conversation_id: room.id.clone(),
                session_id: sid.clone(),
                by_participant: Some(own.clone()),
                expected_revision: None,
                action: Action::Invite {
                    participants: vec![peer],
                    medium: "voice".into(),
                    intent: "conference".into(),
                    capabilities: json!({"audio":["opus"]}),
                },
            })
            .await
            .unwrap();
        client
            .decide_session_invitation(DecisionRequest {
                operation_id: "http-record-cancel".into(),
                conversation_id: room.id.clone(),
                session_id: sid.clone(),
                decision: Decision::Cancel,
                by_participant: Some(own.clone()),
                expected_revision: None,
                reason_code: 487,
                reason: "metadata only".into(),
            })
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let uuid = loop {
            let page = client
                .conversation_records(
                    &room.id,
                    rvoip_uctp::v2::payloads::ConversationGet {
                        page: None,
                        limit: Some(100),
                    },
                )
                .await
                .unwrap();
            if let Some(connect_protocol::events::RecordAvailability::Ready { uuid, .. }) =
                page.records.into_iter().find(|r| r.session_id() == sid)
            {
                break uuid;
            }
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let value = Self {
            f,
            cid: room.id,
            sid,
            uuid,
            own,
        };
        if large {
            // Seed a large allowed metadata source through the trusted backend
            // CAS solely for physical transport backpressure qualification.
            let h = value.handle();
            value
                .f
                .server
                .store
                .mutate_record(
                    &value.scope(),
                    "transport-fixture",
                    RecordMutationKind::Redact,
                    "transport-fixture-v1",
                    &h,
                    |bytes| {
                        let mut body: Value = serde_json::from_slice(&bytes.body).unwrap();
                        body["analysis"][0]["body"]["transport_fixture_padding"] =
                            json!("p".repeat(100 * 1024));
                        Ok(Some(RecordBytes {
                            uuid: bytes.uuid.clone(),
                            body: serde_json::to_vec(&body).unwrap(),
                            manifest: bytes.manifest.clone(),
                        }))
                    },
                )
                .unwrap();
        }
        value
    }
    fn selector(&self) -> CanonicalRecordSelector {
        CanonicalRecordSelector {
            uuid: Some(self.uuid.clone()),
            ..Default::default()
        }
    }
    fn scope(&self) -> RecordScope {
        RecordScope {
            tenant_id: "tnt_default".into(),
            conversation_id: self.cid.clone(),
            session_id: self.sid.clone(),
        }
    }
    fn handle(&self) -> connect_store::records::RecordHandle {
        self.f
            .server
            .store
            .list_records("tnt_default", &self.cid, Some(&self.sid))
            .unwrap()
            .pop()
            .unwrap()
    }
    async fn blocked(&self, credential: &str) -> (RawPeer, usize) {
        let body=serde_json::to_vec(&json!({"v":2,"type":"recording.vcon-fetch","id":"http-raw-record","ts":chrono::Utc::now(),"cid":self.cid,"sid":self.sid,"payload":{"uuid":self.uuid}})).unwrap();
        let header=format!("POST /v2/commands HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {credential}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len());
        let mut child = tokio::process::Command::new("python3")
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/support/http_record_peer.py"),
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = tokio::io::BufReader::new(child.stdout.take().unwrap());
        let mut peer = RawPeer {
            child,
            input,
            output,
        };
        peer.send(json!({"port":self.f.server.http_address.unwrap().port(),"request":format!("{header}{}",String::from_utf8(body).unwrap())})).await;
        let ready = peer.line().await;
        assert_eq!(ready["no_options"], cfg!(target_os = "macos"));
        let headers = ready["headers"].as_str().unwrap().to_ascii_lowercase();
        assert!(
            headers.starts_with("http/1.1 200"),
            "unexpected HTTP status"
        );
        assert!(headers.contains("connection: close"));
        assert!(headers.contains("cache-control: no-store"));
        let length = headers
            .lines()
            .find_map(|l| l.strip_prefix("content-length: "))
            .unwrap()
            .parse()
            .unwrap();
        assert!(length > 100 * 1024);
        (peer, length)
    }
}
struct RawPeer {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::io::BufReader<tokio::process::ChildStdout>,
}
impl RawPeer {
    async fn send(&mut self, value: Value) {
        let mut line = serde_json::to_vec(&value).unwrap();
        line.push(b'\n');
        self.input.write_all(&line).await.unwrap();
        self.input.flush().await.unwrap();
    }
    async fn line(&mut self) -> Value {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(5), self.output.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        assert!(line.len() <= 4 * 1024 * 1024);
        serde_json::from_str(&line).unwrap()
    }
    async fn finish(mut self, complete: bool) -> Vec<u8> {
        self.send(json!("read")).await;
        let result = self.line().await;
        if complete {
            assert!(result["error"].is_null());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(result["body"].as_str().unwrap())
            .unwrap();
        assert!(self.child.wait().await.unwrap().success());
        bytes
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_http_record_fetch_matches_native_digest_and_current_attenuated_sources() {
    for algorithm in ["EdDSA", "ES256"] {
        let r = RecordFixture::new(algorithm, false).await;
        let mut client = http(&r.f, &r.f.client.credential).await;
        let record = client
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        let mut native = CanonicalClient::open(r.f.client.clone()).await.unwrap();
        assert_eq!(
            record.body,
            native
                .fetch_record(&r.cid, &r.sid, r.selector())
                .await
                .unwrap()
                .body
        );
        let key =
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope {
                        operations: vec!["record_get".into()],
                        recipients: vec![],
                    },
                    chrono::Utc::now().timestamp() + 300,
                )
                .unwrap();
        let mut narrow = http(&r.f, &key.token).await;
        let masked = narrow
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        let body: Value = serde_json::from_str(&masked.body).unwrap();
        assert_eq!(body["parties"].as_array().unwrap().len(), 1);
        assert!(!masked.body.contains("connection_omissions"));
        assert_ne!(masked.content_hash, record.content_hash);
        assert!(narrow
            .fetch_record(
                &r.cid,
                &r.sid,
                CanonicalRecordSelector {
                    content_hash: Some(record.content_hash),
                    ..r.selector()
                }
            )
            .await
            .is_err());
        assert_eq!(
            narrow
                .fetch_record(
                    &r.cid,
                    &r.sid,
                    CanonicalRecordSelector {
                        content_hash: Some(masked.content_hash),
                        ..r.selector()
                    }
                )
                .await
                .unwrap()
                .body,
            masked.body
        );
        assert!(client
            .fetch_record(
                &r.cid,
                &r.sid,
                CanonicalRecordSelector {
                    uuid: Some(uuid::Uuid::new_v4().to_string()),
                    ..Default::default()
                }
            )
            .await
            .is_err());
        native.close();
        r.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_http_backpressure_retains_credential_and_exact_record_until_socket_finishes() {
    let r = RecordFixture::new("EdDSA", true).await;
    let mut client = http(&r.f, &r.f.client.credential).await;
    let expected = client
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .unwrap();
    let (stream, length) = r.blocked(&r.f.client.credential).await;
    let store = r.f.server.store.clone();
    let admin = store.authenticate(&r.f.admin).unwrap();
    let credential = store
        .authenticate(&r.f.client.credential)
        .unwrap()
        .credential_id;
    let revoking = tokio::task::spawn_blocking(move || store.revoke(&admin, &credential));
    let store = r.f.server.store.clone();
    let scope = r.scope();
    let handle = r.handle();
    let deleting = tokio::task::spawn_blocking(move || {
        store.mutate_record(
            &scope,
            "delete-after-http-write",
            RecordMutationKind::Delete,
            "delete-v1",
            &handle,
            |_| Ok(None),
        )
    });
    let queue_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let reader = AuthorityScope::identity(&r.f.client.identity_id, AuthorityMode::Read).unwrap();
    loop {
        match r.f.server.store.try_acquire_authority(&[reader.clone()]) {
            Err(e) if e.code == "authority-busy" => break,
            Ok(held) => drop(held),
            Err(e) => panic!("unexpected reader admission {}", e.code),
        }
        assert!(
            tokio::time::Instant::now() < queue_deadline,
            "actual revocation writer never queued"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !revoking.is_finished(),
        "credential guard released before response IO"
    );
    assert!(
        !deleting.is_finished(),
        "record custody released before response IO"
    );
    let bytes = stream.finish(true).await;
    assert_eq!(bytes.len(), length);
    let batch: connect_protocol::canonical_http::ReplyBatch =
        serde_json::from_slice(&bytes).unwrap();
    let payload = &batch.envelopes[0]["payload"];
    assert_eq!(payload["vcon"], expected.body);
    assert_eq!(payload["content_hash"], expected.content_hash);
    revoking.await.unwrap().unwrap();
    assert!(deleting.await.unwrap().unwrap().is_some());
    assert!(client
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .is_err());
    r.f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_http_expiry_aborts_stalled_body_and_releases_authority_and_capacity() {
    let r = RecordFixture::with_capacity("EdDSA", true, 1).await;
    let short =
        r.f.server
            .store
            .issue(
                &r.f.client.identity_id,
                Scope::collaboration(vec!["*".into()]),
                chrono::Utc::now().timestamp() + 3,
            )
            .unwrap();
    let (stream, length) = r.blocked(&short.token).await;
    let mut competitor = http(&r.f, &r.f.client.credential).await;
    let refusal = competitor
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .unwrap_err();
    assert!(matches!(refusal,ClientError::Application(e) if e.code=="capacity-exceeded"));
    tokio::time::sleep(Duration::from_secs(4)).await;
    let scope = AuthorityScope::identity(&r.f.client.identity_id, AuthorityMode::Write).unwrap();
    let held = r.f.server.store.try_acquire_authority(&[scope]).unwrap();
    drop(held);
    let bytes = stream.finish(false).await;
    assert!(
        bytes.len() < length,
        "expired socket sent a complete record"
    );
    let mut client = http(&r.f, &r.f.client.credential).await;
    assert!(client
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .unwrap()
        .body
        .contains("transport_fixture_padding"));
    r.f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_http_disconnect_releases_record_custody_without_recreating_or_deleting_data() {
    let r = RecordFixture::new("EdDSA", true).await;
    let (stream, _) = r.blocked(&r.f.client.credential).await;
    drop(stream);
    let store = r.f.server.store.clone();
    let identity = r.f.client.identity_id.clone();
    let held = tokio::task::spawn_blocking(move || {
        store.acquire_authority(
            &[AuthorityScope::identity(&identity, AuthorityMode::Write).unwrap()],
            &AuthorityWait::new(Duration::from_secs(2)),
        )
    })
    .await
    .unwrap()
    .unwrap();
    drop(held);
    let mut client = http(&r.f, &r.f.client.credential).await;
    assert!(client
        .fetch_record(&r.cid, &r.sid, r.selector())
        .await
        .unwrap()
        .body
        .contains("transport_fixture_padding"));
    r.f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_http_shutdown_cancels_stalled_record_io_before_runtime_retirement() {
    let r = RecordFixture::new("EdDSA", true).await;
    let (stream, length) = r.blocked(&r.f.client.credential).await;
    tokio::time::timeout(Duration::from_secs(6), r.f.server.shutdown())
        .await
        .unwrap()
        .unwrap();
    let bytes = stream.finish(false).await;
    assert!(bytes.len() < length);
}

impl RecordFixture {
    async fn https(mut self, ttl: u32) -> Self {
        self.f.config.bind = self.f.server.address;
        self.f.config.http_bind = self.f.server.http_address;
        self.f.config.record_https = Some(connect_server::canonical::RecordHttpsConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            server_name: "localhost".into(),
            capacity: 8,
            ttl_seconds: ttl,
        });
        self.f.server.shutdown().await.unwrap();
        self.f.server = RunningCanonicalServer::start(self.f.config.clone())
            .await
            .unwrap();
        self.f.client.endpoint = self.f.server.address;
        let h = self.handle();
        self.f
            .server
            .store
            .mutate_record(
                &self.scope(),
                "https-transport-fixture",
                RecordMutationKind::Redact,
                "https-transport-fixture-v1",
                &h,
                |bytes| {
                    let mut body: Value = serde_json::from_slice(&bytes.body).unwrap();
                    body["analysis"][0]["body"]["transport_fixture_padding"] =
                        json!("p".repeat(256 * 1024));
                    Ok(Some(RecordBytes {
                        uuid: bytes.uuid.clone(),
                        body: serde_json::to_vec(&body).unwrap(),
                        manifest: bytes.manifest.clone(),
                    }))
                },
            )
            .unwrap();
        self
    }
    fn pin(&self) -> connect_client::canonical::RecordTlsConfig {
        connect_client::canonical::RecordTlsConfig {
            server_name: "localhost".into(),
            trust_certificate: self.f.client.trust_certificate.clone(),
        }
    }
    fn downloader(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .tls_built_in_root_certs(false)
            .add_root_certificate(
                reqwest::Certificate::from_der(
                    &std::fs::read(&self.f.client.trust_certificate).unwrap(),
                )
                .unwrap(),
            )
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .http1_only()
            .resolve("localhost", self.f.server.record_https_address.unwrap())
            .build()
            .unwrap()
    }
    async fn reference(&self) -> Value {
        let mut client = http(&self.f, &self.f.client.credential).await;
        let batch = client
            .request_value(
                "recording.vcon-fetch",
                json!({"uuid":self.uuid}),
                connect_client::canonical::CommandScope {
                    conversation_id: Some(self.cid.clone()),
                    session_id: Some(self.sid.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        batch.ack().unwrap();
        let value = batch
            .frames
            .iter()
            .find(|f| f.kind() == "recording.vcon-fetched")
            .unwrap()
            .original()["payload"]
            .clone();
        assert_eq!(value["delivery"], "url");
        value
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_record_https_reference_downloads_with_native_and_explicit_http_pins() {
    for algorithm in ["EdDSA", "ES256"] {
        let r = RecordFixture::new(algorithm, false).await.https(30).await;
        let mut native = CanonicalClient::open(r.f.client.clone()).await.unwrap();
        let record = native
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        assert!(record.body.len() > 128 * 1024);
        let mut client = http(&r.f, &r.f.client.credential)
            .await
            .with_record_tls(r.pin())
            .unwrap();
        let via_http = client
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        assert_eq!(via_http.content_hash, record.content_hash);
        assert_eq!(via_http.body, record.body);
        let mut unpinned = http(&r.f, &r.f.client.credential).await;
        assert!(matches!(
            unpinned.fetch_record(&r.cid, &r.sid, r.selector()).await,
            Err(ClientError::Configuration)
        ));
        let reference = r.reference().await;
        let url = reference["url"].as_str().unwrap();
        let download = r.downloader();
        assert_eq!(
            download.get(url).send().await.unwrap().status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        let response = download
            .get(url)
            .bearer_auth(&r.f.client.credential)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(
            response.headers()[reqwest::header::CACHE_CONTROL],
            "no-store"
        );
        assert_eq!(response.text().await.unwrap(), record.body);
        let other =
            r.f.server
                .store
                .issue(
                    &r.f.peer.identity_id,
                    Scope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 60,
                )
                .unwrap();
        assert_eq!(
            download
                .get(url)
                .bearer_auth(other.token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::FORBIDDEN
        );
        let narrow =
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope {
                        operations: vec!["record_get".into()],
                        recipients: vec![],
                    },
                    chrono::Utc::now().timestamp() + 60,
                )
                .unwrap();
        assert_eq!(
            download
                .get(url)
                .bearer_auth(narrow.token)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::CONFLICT
        );
        let wrong_der = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let wrong_path = r.f.temp.path().join("wrong-record-pin.der");
        std::fs::write(&wrong_path, wrong_der.cert.der()).unwrap();
        let mut wrong = http(&r.f, &r.f.client.credential)
            .await
            .with_record_tls(connect_client::canonical::RecordTlsConfig {
                server_name: "localhost".into(),
                trust_certificate: wrong_path,
            })
            .unwrap();
        assert!(matches!(
            wrong.fetch_record(&r.cid, &r.sid, r.selector()).await,
            Err(ClientError::Transport)
        ));
        let mut wrong = http(&r.f, &r.f.client.credential)
            .await
            .with_record_tls(connect_client::canonical::RecordTlsConfig {
                server_name: "other.invalid".into(),
                trust_certificate: r.f.client.trust_certificate.clone(),
            })
            .unwrap();
        assert!(matches!(
            wrong.fetch_record(&r.cid, &r.sid, r.selector()).await,
            Err(ClientError::Protocol)
        ));
        let next =
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 120,
                )
                .unwrap();
        bind(
            &r.f,
            &next.id,
            r.f.key_revision,
            "reference-refresh-binding",
        );
        native.refresh_credential(next.token).await.unwrap();
        let admin = r.f.server.store.authenticate(&r.f.admin).unwrap();
        let old =
            r.f.server
                .store
                .authenticate(&r.f.client.credential)
                .unwrap()
                .credential_id;
        r.f.server.store.revoke(&admin, &old).unwrap();
        assert_eq!(
            native
                .fetch_record(&r.cid, &r.sid, r.selector())
                .await
                .unwrap()
                .content_hash,
            record.content_hash
        );
        native.close();
        assert!(native
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .is_err());
        r.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn record_https_reference_rejects_changed_deleted_expired_and_restarted_views() {
    let mut r = RecordFixture::new("EdDSA", false).await.https(2).await;
    let reference = r.reference().await;
    let url = reference["url"].as_str().unwrap();
    let download = r.downloader();
    let h = r.handle();
    r.f.server
        .store
        .mutate_record(
            &r.scope(),
            "changed-https-view",
            RecordMutationKind::Redact,
            "changed-https-v1",
            &h,
            |bytes| {
                let mut body: Value = serde_json::from_slice(&bytes.body).unwrap();
                body["analysis"][0]["body"]["transport_fixture_padding"] =
                    json!("q".repeat(256 * 1024));
                Ok(Some(RecordBytes {
                    uuid: bytes.uuid.clone(),
                    body: serde_json::to_vec(&body).unwrap(),
                    manifest: bytes.manifest.clone(),
                }))
            },
        )
        .unwrap();
    assert_eq!(
        download
            .get(url)
            .bearer_auth(&r.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let fresh = r.reference().await;
    let expires: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(fresh["expires_at"].clone()).unwrap();
    if let Ok(wait) = (expires + chrono::Duration::milliseconds(100) - chrono::Utc::now()).to_std()
    {
        tokio::time::sleep(wait).await;
    }
    assert_eq!(
        download
            .get(fresh["url"].as_str().unwrap())
            .bearer_auth(&r.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    let fresh = r.reference().await;
    let old_address = r.f.server.record_https_address.unwrap();
    r.f.config.record_https.as_mut().unwrap().bind = old_address;
    r.f.server.shutdown().await.unwrap();
    r.f.server = RunningCanonicalServer::start(r.f.config.clone())
        .await
        .unwrap();
    assert_eq!(
        download
            .get(fresh["url"].as_str().unwrap())
            .bearer_auth(&r.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    let fresh = r.reference().await;
    let h = r.handle();
    r.f.server
        .store
        .mutate_record(
            &r.scope(),
            "delete-https-view",
            RecordMutationKind::Delete,
            "delete-https-v1",
            &h,
            |_| Ok(None),
        )
        .unwrap();
    assert_eq!(
        download
            .get(fresh["url"].as_str().unwrap())
            .bearer_auth(&r.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    r.f.server.shutdown().await.unwrap();
}

impl RecordFixture {
    async fn blocked_https(&self, url: &str, credential: &str) -> (RawPeer, usize) {
        let url = reqwest::Url::parse(url).unwrap();
        let header=format!("GET {} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {credential}\r\nConnection: close\r\n\r\n", url.path());
        let mut child = tokio::process::Command::new("python3")
            .arg(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/support/http_record_peer.py"),
            )
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = tokio::io::BufReader::new(child.stdout.take().unwrap());
        let mut peer = RawPeer {
            child,
            input,
            output,
        };
        peer.send(json!({"port":self.f.server.record_https_address.unwrap().port(),"request":header,"server_name":"localhost",
            "trust_der":base64::engine::general_purpose::STANDARD.encode(std::fs::read(&self.f.client.trust_certificate).unwrap())})).await;
        let ready = peer.line().await;
        assert_eq!(ready["no_options"], cfg!(target_os = "macos"));
        let headers = ready["headers"].as_str().unwrap().to_ascii_lowercase();
        assert!(
            headers.starts_with("http/1.1 200"),
            "unexpected HTTPS status"
        );
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        (peer, length)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_https_backpressure_retains_source_and_record_through_tls_socket_io() {
    let r = RecordFixture::new("EdDSA", false).await.https(30).await;
    let reference = r.reference().await;
    let url = reference["url"].as_str().unwrap();
    let (stream, length) = r.blocked_https(url, &r.f.client.credential).await;
    let store = r.f.server.store.clone();
    let admin = store.authenticate(&r.f.admin).unwrap();
    let credential = store
        .authenticate(&r.f.client.credential)
        .unwrap()
        .credential_id;
    let revoking = tokio::task::spawn_blocking(move || store.revoke(&admin, &credential));
    let store = r.f.server.store.clone();
    let scope = r.scope();
    let handle = r.handle();
    let deleting = tokio::task::spawn_blocking(move || {
        store.mutate_record(
            &scope,
            "delete-after-https-write",
            RecordMutationKind::Delete,
            "delete-https-v1",
            &handle,
            |_| Ok(None),
        )
    });
    let queue_deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let reader = AuthorityScope::identity(&r.f.client.identity_id, AuthorityMode::Read).unwrap();
    loop {
        match r.f.server.store.try_acquire_authority(&[reader.clone()]) {
            Err(e) if e.code == "authority-busy" => break,
            Ok(held) => drop(held),
            Err(e) => panic!("unexpected reader admission {}", e.code),
        }
        assert!(
            tokio::time::Instant::now() < queue_deadline,
            "actual HTTPS revocation never queued"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !revoking.is_finished(),
        "HTTPS credential custody ended before socket IO"
    );
    assert!(
        !deleting.is_finished(),
        "HTTPS record custody ended before socket IO"
    );
    let bytes = stream.finish(true).await;
    assert_eq!(bytes.len(), length);
    assert_eq!(
        format!(
            "sha512-{}",
            URL_SAFE_NO_PAD.encode(<sha2::Sha512 as sha2::Digest>::digest(&bytes))
        ),
        reference["content_hash"]
    );
    revoking.await.unwrap().unwrap();
    assert!(deleting.await.unwrap().unwrap().is_some());
    r.f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_https_reference_expiry_and_shutdown_abort_stalled_tls_and_release_guards() {
    for shutdown in [false, true] {
        let r = RecordFixture::new("EdDSA", false)
            .await
            .https(if shutdown { 30 } else { 2 })
            .await;
        let reference = r.reference().await;
        let (stream, length) = r
            .blocked_https(reference["url"].as_str().unwrap(), &r.f.client.credential)
            .await;
        let mut store = r.f.server.store.clone();
        let scope = r.scope();
        let handle = r.handle();
        let mut server = Some(r.f.server);
        if shutdown {
            server.take().unwrap().shutdown().await.unwrap();
        } else {
            let expires: chrono::DateTime<chrono::Utc> =
                serde_json::from_value(reference["expires_at"].clone()).unwrap();
            if let Ok(wait) =
                (expires + chrono::Duration::milliseconds(150) - chrono::Utc::now()).to_std()
            {
                tokio::time::sleep(wait).await;
            }
        }
        // Runtime retirement intentionally rejects old Store handles. Reopen
        // administration to prove the old socket no longer owns record guards.
        if shutdown {
            store = std::sync::Arc::new(
                connect_store::Store::open_administration(
                    &r.f.config.database,
                    r.f.config.limits.clone(),
                )
                .unwrap(),
            );
        }
        let bytes = stream.finish(false).await;
        assert!(
            bytes.len() < length,
            "expired/stopped HTTPS socket sent a complete record"
        );
        assert!(tokio::task::spawn_blocking(move || store.mutate_record(
            &scope,
            "delete-after-https-abort",
            RecordMutationKind::Delete,
            "delete-abort-v1",
            &handle,
            |_| Ok(None)
        ))
        .await
        .unwrap()
        .unwrap()
        .is_some());
        if let Some(server) = server {
            server.shutdown().await.unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn issued_https_url_selectors_use_fresh_native_and_http_source_authority() {
    for algorithm in ["EdDSA", "ES256"] {
        let r = RecordFixture::new(algorithm, false).await.https(30).await;
        let reference = r.reference().await;
        let selected = CanonicalRecordSelector {
            uuid: None,
            url: Some(reference["url"].as_str().unwrap().into()),
            content_hash: Some(reference["content_hash"].as_str().unwrap().into()),
        };
        let mut native = CanonicalClient::open(r.f.client.clone()).await.unwrap();
        let mut client = http(&r.f, &r.f.client.credential)
            .await
            .with_record_tls(r.pin())
            .unwrap();
        let full = native
            .fetch_record(&r.cid, &r.sid, selected.clone())
            .await
            .unwrap();
        assert_eq!(full.uuid, r.uuid);
        assert!(full.body.len() > 128 * 1024);
        assert_eq!(
            client
                .fetch_record(&r.cid, &r.sid, selected.clone())
                .await
                .unwrap()
                .body,
            full.body
        );
        assert_eq!(
            native
                .fetch_record(
                    &r.cid,
                    &r.sid,
                    CanonicalRecordSelector {
                        uuid: Some(r.uuid.clone()),
                        ..selected.clone()
                    }
                )
                .await
                .unwrap()
                .content_hash,
            full.content_hash
        );
        for bad in [
            CanonicalRecordSelector { uuid: Some(uuid::Uuid::new_v4().to_string()), ..selected.clone() },
            CanonicalRecordSelector { content_hash: Some("sha512-another-view".into()), ..selected.clone() },
            CanonicalRecordSelector { url: Some("https://unconfigured.invalid/v2/record-references/0123456789abcdef0123456789abcdef".into()), ..selected.clone() },
        ] {
            assert!(native.fetch_record(&r.cid,&r.sid,bad.clone()).await.is_err());
            assert!(client.fetch_record(&r.cid,&r.sid,bad).await.is_err());
        }
        assert!(native
            .fetch_record(&r.cid, "another-session", selected.clone())
            .await
            .is_err());
        assert!(client
            .fetch_record(&r.cid, "another-session", selected.clone())
            .await
            .is_err());
        let trap = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let arbitrary = format!(
            "https://localhost:{}/v2/record-references/0123456789abcdef0123456789abcdef",
            trap.local_addr().unwrap().port()
        );
        for uuid in [None, Some(r.uuid.clone())] {
            let bad = CanonicalRecordSelector {
                uuid,
                url: Some(arbitrary.clone()),
                content_hash: Some(full.content_hash.clone()),
            };
            assert!(
                matches!(native.fetch_record(&r.cid,&r.sid,bad.clone()).await,Err(ClientError::Application(error)) if error.code=="not-found")
            );
            assert!(
                matches!(client.fetch_record(&r.cid,&r.sid,bad).await,Err(ClientError::Application(error)) if error.code=="not-found")
            );
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), trap.accept())
                .await
                .is_err(),
            "an arbitrary selector caused network IO"
        );
        let narrow =
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope {
                        operations: vec!["record_get".into()],
                        recipients: vec![],
                    },
                    chrono::Utc::now().timestamp() + 60,
                )
                .unwrap();
        let mut masked = http(&r.f, &narrow.token)
            .await
            .with_record_tls(r.pin())
            .unwrap();
        assert!(masked
            .fetch_record(&r.cid, &r.sid, selected.clone())
            .await
            .is_err());
        let masked = masked
            .fetch_record(
                &r.cid,
                &r.sid,
                CanonicalRecordSelector {
                    content_hash: None,
                    ..selected.clone()
                },
            )
            .await
            .unwrap();
        assert_ne!(masked.content_hash, full.content_hash);
        assert_eq!(
            serde_json::from_str::<Value>(&masked.body).unwrap()["parties"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let denied =
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope {
                        operations: vec!["history".into()],
                        recipients: vec!["*".into()],
                    },
                    chrono::Utc::now().timestamp() + 60,
                )
                .unwrap();
        assert!(http(&r.f, &denied.token)
            .await
            .fetch_record(&r.cid, &r.sid, selected.clone())
            .await
            .is_err());
        assert_eq!(
            native
                .fetch_record(&r.cid, &r.sid, selected.clone())
                .await
                .unwrap()
                .content_hash,
            full.content_hash
        );
        let h = r.handle();
        r.f.server
            .store
            .mutate_record(
                &r.scope(),
                "changed-url-selector",
                RecordMutationKind::Redact,
                "changed-url-selector-v1",
                &h,
                |bytes| {
                    let mut body: Value = serde_json::from_slice(&bytes.body).unwrap();
                    body["analysis"][0]["body"]["transport_fixture_padding"] =
                        json!("z".repeat(256 * 1024));
                    Ok(Some(RecordBytes {
                        uuid: bytes.uuid.clone(),
                        body: serde_json::to_vec(&body).unwrap(),
                        manifest: bytes.manifest.clone(),
                    }))
                },
            )
            .unwrap();
        assert!(
            matches!(native.fetch_record(&r.cid,&r.sid,selected.clone()).await,Err(ClientError::Application(error)) if error.code=="conflict")
        );
        assert!(
            matches!(client.fetch_record(&r.cid,&r.sid,selected).await,Err(ClientError::Application(error)) if error.code=="conflict")
        );
        native.close();
        r.f.server.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_issued_url_cannot_be_reused_as_a_control_selector() {
    let r = RecordFixture::new("EdDSA", false).await.https(2).await;
    let reference = r.reference().await;
    let expires: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(reference["expires_at"].clone()).unwrap();
    if let Ok(wait) = (expires + chrono::Duration::milliseconds(100) - chrono::Utc::now()).to_std()
    {
        tokio::time::sleep(wait).await;
    }
    let selected = CanonicalRecordSelector {
        uuid: None,
        url: Some(reference["url"].as_str().unwrap().into()),
        content_hash: Some(reference["content_hash"].as_str().unwrap().into()),
    };
    let mut native = CanonicalClient::open(r.f.client.clone()).await.unwrap();
    let mut client = http(&r.f, &r.f.client.credential)
        .await
        .with_record_tls(r.pin())
        .unwrap();
    assert!(
        matches!(native.fetch_record(&r.cid,&r.sid,selected.clone()).await,Err(ClientError::Application(error)) if error.code=="not-found")
    );
    assert!(
        matches!(client.fetch_record(&r.cid,&r.sid,selected).await,Err(ClientError::Application(error)) if error.code=="not-found")
    );
    native.close();
    r.f.server.shutdown().await.unwrap();
}

async fn mutation_sample(
    r: &RecordFixture,
    credential: &str,
    request: &connect_protocol::record_mutations::Request,
    case: &str,
) {
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .post(format!(
            "http://{}/v2/records/{}/mutations",
            r.f.server.http_address.unwrap(),
            request.uuid
        ))
        .bearer_auth(credential)
        .json(request)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    assert_eq!(
        response.headers()[reqwest::header::CACHE_CONTROL],
        "no-store"
    );
    let value: Value = response.json().await.unwrap();
    if let Ok(directory) = std::env::var("CONNECT_G6_RECORD_MUTATION_SAMPLES") {
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            std::path::Path::new(&directory)
                .join(format!("{}-{case}.json", r.f.client.signing_algorithm)),
            serde_json::to_vec_pretty(
                &json!({"case":case,"status":status,"request":request,"response":value}),
            )
            .unwrap(),
        )
        .unwrap();
    }
}
async fn mutation_client(
    r: &RecordFixture,
    credential: &str,
) -> connect_client::application::ApplicationClient {
    connect_client::application::ApplicationClient::open(ApplicationClientConfig {
        endpoint: r.f.server.http_address.unwrap(),
        credential: credential.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 8000,
    })
    .await
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_record_erasure_uses_semantic_sources_and_replays_after_deletion_and_restart() {
    use connect_protocol::record_mutations::{
        Action, Mode, RemoteDeletion, Request as Mutation, Source, Target,
    };
    for algorithm in ["EdDSA", "ES256"] {
        let mut r = RecordFixture::new(algorithm, false).await.https(30).await;
        let mut reader = http(&r.f, &r.f.client.credential)
            .await
            .with_record_tls(r.pin())
            .unwrap();
        let original = reader
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        let page = reader
            .conversation_records(
                &r.cid,
                rvoip_uctp::v2::payloads::ConversationGet {
                    page: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap();
        let revision =
            page.records
                .into_iter()
                .find_map(|record| match record {
                    connect_protocol::events::RecordAvailability::Ready {
                        uuid, revision, ..
                    } if uuid == r.uuid => Some(revision),
                    _ => None,
                })
                .unwrap();
        let reference = r.reference().await;
        let request = Mutation {
            operation_id: "public-own-redaction".into(),
            conversation_id: r.cid.clone(),
            session_id: r.sid.clone(),
            uuid: r.uuid.clone(),
            expected_revision: revision,
            action: Action::Redact {
                sources: vec![Source {
                    target: Target::Participant {
                        participant_id: r.own.clone(),
                        membership_epoch: 1,
                    },
                    mode: Mode::Metadata,
                }],
            },
        };
        let mut ordinary = mutation_client(&r, &r.f.client.credential).await;
        assert!(
            matches!(ordinary.mutate_record(request.clone()).await,Err(ClientError::Application(error)) if error.code=="forbidden")
        );
        mutation_sample(&r, &r.f.client.credential, &request, "permission-denied").await;
        mutation_sample(
            &r,
            "not-a-current-credential",
            &request,
            "invalid-credential",
        )
        .await;
        let key =
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope {
                        operations: vec![
                            "record_get".into(),
                            "record_redact".into(),
                            "record_delete".into(),
                        ],
                        recipients: vec![],
                    },
                    chrono::Utc::now().timestamp() + 120,
                )
                .unwrap();
        let mut caller = mutation_client(&r, &key.token).await;
        let receipt = caller.mutate_record(request.clone()).await.unwrap();
        assert_eq!(receipt.revision, revision + 1);
        assert!(receipt.remote_deletion == RemoteDeletion::NotAttempted);
        assert!(caller.mutate_record(request.clone()).await.unwrap() == receipt);
        mutation_sample(&r, &key.token, &request, "redaction-receipt-replay").await;
        let next = reader
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        assert_ne!(next.content_hash, original.content_hash);
        let value: Value = serde_json::from_str(&next.body).unwrap();
        assert!(!value["parties"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["connect_participant_id"] == r.own));
        assert_eq!(
            r.downloader()
                .get(reference["url"].as_str().unwrap())
                .bearer_auth(&r.f.client.credential)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::CONFLICT
        );
        let mut changed = request.clone();
        changed.expected_revision += 1;
        assert!(
            matches!(caller.mutate_record(changed.clone()).await,Err(ClientError::Application(error)) if error.code=="conflict")
        );
        mutation_sample(&r, &key.token, &changed, "changed-intent").await;
        let deletion = Mutation {
            operation_id: "public-record-deletion".into(),
            expected_revision: receipt.revision,
            action: Action::Delete,
            ..request.clone()
        };
        let mut admin = mutation_client(&r, &r.f.admin).await;
        let deleted = admin.mutate_record(deletion.clone()).await.unwrap();
        assert_eq!(deleted.revision, revision + 2);
        mutation_sample(&r, &r.f.admin, &deletion, "deletion-receipt-replay").await;
        assert!(caller.mutate_record(request.clone()).await.unwrap() == receipt);
        assert!(
            matches!(reader.fetch_record(&r.cid,&r.sid,r.selector()).await,Err(ClientError::Application(error)) if error.code=="not-found")
        );
        let page = reader
            .conversation_records(
                &r.cid,
                rvoip_uctp::v2::payloads::ConversationGet {
                    page: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap();
        assert!(page.records.iter().any(|v|matches!(v,connect_protocol::events::RecordAvailability::Unavailable {session_id} if session_id==&r.sid)));
        let config = r.f.config.clone();
        r.f.server.shutdown().await.unwrap();
        r.f.server = RunningCanonicalServer::start(config).await.unwrap();
        let mut caller = mutation_client(&r, &key.token).await;
        assert!(caller.mutate_record(request).await.unwrap() == receipt);
        let mut admin = mutation_client(&r, &r.f.admin).await;
        assert!(admin.mutate_record(deletion).await.unwrap() == deleted);
        r.f.server.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn named_retention_worker_expires_public_metadata_and_keeps_unavailable_after_restart() {
    use connect_protocol::record_retention::{Lifetime, Policy};
    for algorithm in ["EdDSA", "ES256"] {
        let mut r = RecordFixture::with_retention(
            algorithm,
            false,
            8,
            Some(Policy {
                name: "http-five-second-test".into(),
                metadata: Lifetime::After { seconds: 5 },
                ..Default::default()
            }),
        )
        .await;
        let mut client = http(&r.f, &r.f.client.credential).await;
        client
            .fetch_record(&r.cid, &r.sid, r.selector())
            .await
            .unwrap();
        // Replacing the installation policy cannot extend this original record.
        r.f.server
            .store
            .configure_record_retention(&Policy::default())
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            if r.f
                .server
                .store
                .list_records("tnt_default", &r.cid, Some(&r.sid))
                .unwrap()
                .is_empty()
            {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "owned retention worker must commit expiry"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            matches!(client.fetch_record(&r.cid,&r.sid,r.selector()).await,Err(ClientError::Application(e)) if e.code=="not-found")
        );
        let page = client
            .conversation_records(
                &r.cid,
                rvoip_uctp::v2::payloads::ConversationGet {
                    page: None,
                    limit: Some(100),
                },
            )
            .await
            .unwrap();
        assert!(page.records.iter().any(|v|matches!(v,connect_protocol::events::RecordAvailability::Unavailable{session_id} if session_id==&r.sid)));
        let config = r.f.config.clone();
        r.f.server.shutdown().await.unwrap();
        r.f.server = RunningCanonicalServer::start(config).await.unwrap();
        let mut client = http(&r.f, &r.f.client.credential).await;
        assert!(
            matches!(client.fetch_record(&r.cid,&r.sid,r.selector()).await,Err(ClientError::Application(e)) if e.code=="not-found")
        );
        r.f.server.shutdown().await.unwrap();
    }
}

#[path = "canonical_record_readers.rs"]
mod tenant_readers;

#[path = "canonical_record_context.rs"]
mod record_context;
