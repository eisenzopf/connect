//! Real private/public Connection provenance over the pinned HTTPS reference
//! endpoint. Only transport size uses trusted padding; all source audiences,
//! Connection epochs, Session readiness and public erasure remain actual.
use super::*;
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::{CanonicalRecordSelector, RecordTlsConfig},
};
use connect_protocol::record_mutations::{Action, Mode, Request as Mutation, Source, Target};
use connect_store::records::{RecordBytes, RecordMutationKind, RecordScope};

async fn reader(f: &Fixture, credential: &str) -> CanonicalHttpClient {
    let mut config = canonical_http_config(f);
    config.credential = credential.into();
    CanonicalHttpClient::open(config)
        .await
        .unwrap()
        .with_record_tls(RecordTlsConfig {
            server_name: "localhost".into(),
            trust_certificate: f.client.trust_certificate.clone(),
        })
        .unwrap()
}
async fn reference(client: &mut CanonicalHttpClient, cid: &str, uuid: &str) -> Value {
    let batch = client
        .request_value(
            "recording.vcon-fetch",
            json!({"uuid":uuid}),
            CommandScope {
                conversation_id: Some(cid.into()),
                session_id: Some("native-session".into()),
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
pub(super) async fn verify(
    f: &Fixture,
    cid: &str,
    consultant: &str,
    private_key: &str,
    remote_key: &str,
) {
    let mut private = reader(f, private_key).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let uuid = loop {
        let page = private
            .conversation_records(cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        if let Some(uuid) = page.records.into_iter().find_map(|r| match r {
            connect_protocol::events::RecordAvailability::Ready {
                session_id, uuid, ..
            } if session_id == "native-session" => Some(uuid),
            _ => None,
        }) {
            break uuid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "actual ended private Session must materialize"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let scope = RecordScope {
        tenant_id: "tnt_default".into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
    };
    let original = f
        .server
        .store
        .list_records("tnt_default", cid, Some("native-session"))
        .unwrap()
        .pop()
        .unwrap();
    let padded = f
        .server
        .store
        .mutate_record(
            &scope,
            "private-tls-size-fixture",
            RecordMutationKind::Redact,
            "private-tls-size-v1",
            &original,
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
        .unwrap()
        .unwrap();
    let mut remote = reader(f, remote_key).await;
    let private_ref = reference(&mut private, cid, &uuid).await;
    let remote_ref = reference(&mut remote, cid, &uuid).await;
    assert_ne!(private_ref["content_hash"], remote_ref["content_hash"]);
    let download = reqwest::Client::builder()
        .tls_built_in_root_certs(false)
        .add_root_certificate(
            reqwest::Certificate::from_der(&std::fs::read(&f.client.trust_certificate).unwrap())
                .unwrap(),
        )
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .http1_only()
        .timeout(Duration::from_secs(5))
        .resolve("localhost", f.server.record_https_address.unwrap())
        .build()
        .unwrap();
    let selected = || CanonicalRecordSelector {
        uuid: Some(uuid.clone()),
        ..Default::default()
    };
    let private_before = private
        .fetch_record(cid, "native-session", selected())
        .await
        .unwrap();
    assert!(private_before.body.contains("private-consultant"));
    assert!(private_before.body.contains("public-consultant"));
    let remote_before = remote
        .fetch_record(cid, "native-session", selected())
        .await
        .unwrap();
    assert!(!remote_before.body.contains("private-consultant"));
    assert!(!remote_before.body.contains("public-consultant"));
    assert!(remote_before.body.contains("codec-source"));
    // A reference is bound to its original viewer even when the other viewer
    // can independently obtain its own narrower reference to this same UUID.
    assert_eq!(
        download
            .get(private_ref["url"].as_str().unwrap())
            .bearer_auth(remote_key)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::FORBIDDEN
    );
    let delivered = download
        .get(private_ref["url"].as_str().unwrap())
        .bearer_auth(private_key)
        .send()
        .await
        .unwrap();
    assert_eq!(delivered.status(), reqwest::StatusCode::OK);
    let delivered = delivered.text().await.unwrap();
    assert_eq!(delivered, private_before.body);

    let credential = f
        .server
        .store
        .issue(
            consultant,
            Scope {
                operations: vec!["record_get".into(), "record_redact".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let mut writer = ApplicationClient::open(ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: credential.token,
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 8000,
    })
    .await
    .unwrap();
    let mutation = Mutation {
        operation_id: "private-tls-source-erasure".into(),
        conversation_id: cid.into(),
        session_id: "native-session".into(),
        uuid: uuid.clone(),
        expected_revision: padded.result.revision,
        action: Action::Redact {
            sources: vec![Source {
                target: Target::Connection {
                    connection_id: "private-consultant".into(),
                    generation: 1,
                },
                mode: Mode::Metadata,
            }],
        },
    };
    let receipt = writer.mutate_record(mutation.clone()).await.unwrap();
    assert_eq!(receipt.revision, padded.result.revision + 1);
    assert!(writer.mutate_record(mutation).await.unwrap() == receipt);
    for (old, key) in [(&private_ref, private_key), (&remote_ref, remote_key)] {
        assert_eq!(
            download
                .get(old["url"].as_str().unwrap())
                .bearer_auth(key)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::CONFLICT
        );
    }
    assert!(f.server.store.authenticate(private_key).is_ok());
    assert!(f.server.store.authenticate(remote_key).is_ok());
    let current = private
        .fetch_record(cid, "native-session", selected())
        .await
        .unwrap();
    assert!(!current.body.contains("private-consultant"));
    assert!(current.body.contains("public-consultant"));
    assert_ne!(current.content_hash, private_before.content_hash);
    let current_remote = remote
        .fetch_record(cid, "native-session", selected())
        .await
        .unwrap();
    assert!(!current_remote.body.contains("private-consultant"));
    assert!(current_remote.body.contains("codec-source"));
    assert!(
        matches!(private.fetch_record(cid, "native-session", CanonicalRecordSelector {content_hash:Some(private_before.content_hash), ..selected()}).await, Err(ClientError::Application(e)) if e.code=="conflict")
    );
}
