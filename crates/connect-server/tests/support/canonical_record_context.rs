//! Record context through the installed HTTP application API and public SDK.
use super::*;
use connect_client::application::ApplicationClient;
use connect_protocol::{ContextRecordState, ContextView, Operation, Outcome};
use sha2::Digest;

async fn context(r: &RecordFixture, credential: &str) -> ContextView {
    let mut client = ApplicationClient::open(ApplicationClientConfig {
        endpoint: r.f.server.http_address.unwrap(),
        credential: credential.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 8000,
    })
    .await
    .unwrap();
    let Outcome::ContextView(view) = client
        .execute(
            Operation::ContextView {
                conversation_id: r.cid.clone(),
                limit: 100,
            }
            .try_into()
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("context")
    };
    view
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn record_context_http_preserves_source_identity_permission_digest_and_erasure() {
    let mut samples = Vec::new();
    for algorithm in ["EdDSA", "ES256"] {
        let r = RecordFixture::new(algorithm, false).await;
        let key = |records: bool, recipients: Vec<String>| {
            let mut operations = vec!["context_view".into()];
            if records {
                operations.push("record_get".into());
            }
            r.f.server
                .store
                .issue(
                    &r.f.client.identity_id,
                    Scope {
                        operations,
                        recipients,
                    },
                    chrono::Utc::now().timestamp() + 300,
                )
                .unwrap()
        };
        let full = key(true, vec!["*".into()]);
        let view = context(&r, &full.token).await;
        let item = view.items.iter().find(|i| i.source_id == r.uuid).unwrap();
        let reference = item.record.as_ref().unwrap();
        assert_eq!(reference.state, ContextRecordState::Available);
        assert_eq!(reference.uuid, r.uuid);
        assert_eq!(reference.session_id, r.sid);
        assert_eq!(
            reference.content_hash.as_deref(),
            Some(
                format!(
                    "sha512-{}",
                    URL_SAFE_NO_PAD.encode(sha2::Sha512::digest(item.body.as_bytes()))
                )
                .as_str()
            )
        );
        assert_eq!(
            serde_json::from_str::<Value>(&item.body).unwrap()["uuid"],
            r.uuid
        );
        let encoded = serde_json::to_string(reference).unwrap();
        assert!(!encoded.contains("body_hash") && !encoded.contains("manifest_hash"));
        let no_records = key(false, vec!["*".into()]);
        assert!(!context(&r, &no_records.token)
            .await
            .items
            .iter()
            .any(|i| i.record.is_some()));
        let narrow = key(true, vec![r.f.client.identity_id.clone()]);
        let narrowed = context(&r, &narrow.token).await;
        let narrowed_item = narrowed
            .items
            .iter()
            .find(|i| i.source_id == r.uuid)
            .unwrap();
        assert_ne!(
            narrowed_item.record.as_ref().unwrap().content_hash,
            reference.content_hash
        );
        assert!(narrowed_item.body.len() < item.body.len());
        for (case, view) in [("available", &view), ("recipient-ceiling", &narrowed)] {
            samples.push(json!({"algorithm":algorithm,"case":case,"response":{"profile":"connect-control/1","result":{"Ok":{"kind":"context_view","data":view}}}}));
        }
        let mut admin = ApplicationClient::open(ApplicationClientConfig {
            endpoint: r.f.server.http_address.unwrap(),
            credential: r.f.admin.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            timeout_ms: 8000,
        })
        .await
        .unwrap();
        admin
            .mutate_record(connect_protocol::record_mutations::Request {
                operation_id: "context-http-delete".into(),
                conversation_id: r.cid.clone(),
                session_id: r.sid.clone(),
                uuid: r.uuid.clone(),
                expected_revision: reference.revision,
                action: connect_protocol::record_mutations::Action::Delete,
            })
            .await
            .unwrap();
        let deleted = context(&r, &full.token).await;
        let item = deleted
            .items
            .iter()
            .find(|i| i.source_id == r.uuid)
            .unwrap();
        assert_eq!(
            item.record.as_ref().unwrap().state,
            ContextRecordState::Deleted
        );
        assert!(item.body.is_empty());
        samples.push(json!({"algorithm":algorithm,"case":"deleted","response":{"profile":"connect-control/1","result":{"Ok":{"kind":"context_view","data":deleted}}}}));
        r.f.server.shutdown().await.unwrap();
    }
    if let Some(path) = std::env::var_os("CONNECT_G6_RECORD_CONTEXT_SAMPLES") {
        std::fs::write(path, serde_json::to_vec_pretty(&samples).unwrap()).unwrap();
    }
}
