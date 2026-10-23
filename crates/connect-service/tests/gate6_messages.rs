//! Canonical payloads and control operations share one durable acceptance engine.
#![allow(clippy::unwrap_used)]
use connect_protocol::{messages::*, Limits, Operation, Outcome};
use connect_service::Service;
use connect_store::{Scope, Store};
use rvoip_uctp::v2::payloads as wire;
use serde_json::json;
use std::sync::Arc;
#[tokio::test]
async fn canonical_send_replays_through_control_and_history_filters_share_current_policy() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let mut ids = vec![];
    let mut keys = vec![];
    for alias in ["alice", "bob", "carol"] {
        let id = store.create_identity(alias).unwrap().id;
        keys.push(
            store
                .issue(
                    &id,
                    Scope::collaboration(vec!["*".into()]),
                    chrono::Utc::now().timestamp() + 600,
                )
                .unwrap()
                .id,
        );
        ids.push(id);
    }
    let service = Service::new(store.clone());
    let Outcome::Conversation(c) = service
        .execute(
            keys[0].clone(),
            Operation::Create {
                request_id: "create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(
                    ids[1..].to_vec(),
                ),
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let snapshot = store.membership_snapshot(&keys[0], &c.id).unwrap();
    let participant = |n: usize| {
        snapshot
            .participants
            .iter()
            .find(|p| p.identity_id == ids[n])
            .unwrap()
            .participant_id
            .clone()
    };
    let payload = json!({"msg_id":"canonical-one","from":participant(0),"to":[participant(1)],"content_type":"application/octet-stream","body":"AP8=","body_encoding":"base64","label":"binary","reliability":{"mode":"reliable_ordered"}});
    let receipt = service
        .canonical_message_send(
            keys[0].clone(),
            "canonical-send".into(),
            c.id.clone(),
            serde_json::from_value(payload.clone()).unwrap(),
        )
        .await
        .unwrap();
    let mut draft = MessageDraft::text(MessageAudience::Participants(vec![participant(1)]), "AP8=");
    draft.msg_id = Some("canonical-one".into());
    draft.label = "binary".into();
    draft.content_type = "application/octet-stream".into();
    draft.body_encoding = BodyEncoding::Base64;
    assert_eq!(
        service
            .execute(
                keys[0].clone(),
                Operation::Send {
                    request_id: "canonical-send".into(),
                    conversation_id: c.id.clone(),
                    message: draft
                }
            )
            .await
            .unwrap(),
        receipt
    );
    let mut spoof = payload.clone();
    spoof["from"] = json!(participant(1));
    assert_eq!(
        service
            .canonical_message_send(
                keys[0].clone(),
                "spoof".into(),
                c.id.clone(),
                serde_json::from_value(spoof).unwrap()
            )
            .await
            .unwrap_err()
            .code,
        "forbidden"
    );
    let query: wire::MessageHistory =
        serde_json::from_value(json!({"limit":1,"include_attachments":false})).unwrap();
    let Outcome::History { messages, next } = service
        .canonical_message_history(keys[1].clone(), c.id.clone(), query)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "AP8=");
    assert!(next.is_some());
    let query = serde_json::from_value(json!({"since_msg_id":"canonical-one","limit":1})).unwrap();
    let Outcome::History { messages, .. } = service
        .canonical_message_history(keys[1].clone(), c.id.clone(), query)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(messages.is_empty());
    let denied = serde_json::from_value(json!({"since_msg_id":"canonical-one"})).unwrap();
    assert_eq!(
        service
            .canonical_message_history(keys[2].clone(), c.id.clone(), denied)
            .await
            .unwrap_err()
            .code,
        "forbidden"
    );
    let query = serde_json::from_value(json!({"since":"9999-01-01T00:00:00Z"})).unwrap();
    let Outcome::History { messages, .. } = service
        .canonical_message_history(keys[1].clone(), c.id.clone(), query)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(messages.is_empty());
    let mut external = payload;
    external["attachments"] = json!([{"id":"artifact","content_type":"image/png","size_bytes":4,"url":"https://private.invalid"}]);
    assert_eq!(
        service
            .canonical_message_send(
                keys[0].clone(),
                "external".into(),
                c.id.clone(),
                serde_json::from_value(external).unwrap()
            )
            .await
            .unwrap_err()
            .code,
        "invalid-request"
    );
    assert!(
        matches!(service.execute(keys[2].clone(),Operation::History{conversation_id:c.id,after:None,limit:100}).await.unwrap(),Outcome::History{messages,..} if messages.is_empty())
    );
}
#[tokio::test]
async fn canonical_default_label_and_summary_history_keep_private_attachment_metadata() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let store =
        Arc::new(Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap());
    let a = store.create_identity("attachment-owner").unwrap().id;
    let b = store.create_identity("attachment-peer").unwrap().id;
    let key = store
        .issue(
            &a,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .id;
    let peer = store
        .issue(
            &b,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .id;
    let service = Service::new(store.clone());
    let Outcome::Conversation(c) = service
        .execute(
            key.clone(),
            Operation::Create {
                request_id: "room".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(
                    vec![b.clone()],
                ),
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let participant = store
        .membership_snapshot(&key, &c.id)
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id == b)
        .unwrap()
        .participant_id;
    let Outcome::Artifact(artifact) = service
        .execute(
            key.clone(),
            Operation::ArtifactBegin {
                request_id: "begin".into(),
                conversation_id: c.id.clone(),
                to: MessageAudience::Participants(vec![participant.clone()]),
                name: "private.txt".into(),
                media_type: "text/plain".into(),
                size: 1,
                sha256: hex::encode(Sha256::digest(b"a")),
                expires_at: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    service
        .execute(
            key.clone(),
            Operation::ArtifactAppend {
                request_id: "append".into(),
                conversation_id: c.id.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: "YQ==".into(),
            },
        )
        .await
        .unwrap();
    service
        .execute(
            key.clone(),
            Operation::ArtifactFinish {
                request_id: "finish".into(),
                conversation_id: c.id.clone(),
                artifact_id: artifact.id.clone(),
            },
        )
        .await
        .unwrap();
    service.canonical_message_send(key,"canonical-file".into(),c.id.clone(),serde_json::from_value(json!({"msg_id":"file-message","to":[participant],"content_type":"text/plain","body":"file","reliability":{"mode":"reliable_ordered"},"attachments":[{"id":artifact.id,"content_type":"text/plain","size_bytes":1}]})).unwrap()).await.unwrap();
    let Outcome::History { messages, .. } = service
        .canonical_message_history(
            peer.clone(),
            c.id.clone(),
            serde_json::from_value(json!({"include_attachments":false})).unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].label, "rvoip-messages");
    assert_eq!(
        messages[0].attachments,
        vec![AttachmentRef {
            id: artifact.id.clone(),
            content_type: "text/plain".into(),
            size_bytes: 1
        }]
    );
    let Outcome::ArtifactLink(link) = service
        .execute(
            peer.clone(),
            Operation::ArtifactDownloadLink {
                conversation_id: c.id.clone(),
                artifact_id: artifact.id.clone(),
                message_id: "file-message".into(),
                expires_in: 60,
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let Outcome::ArtifactChunk(chunk) = service
        .execute(
            peer,
            Operation::ArtifactLinkGet {
                conversation_id: c.id,
                artifact_id: artifact.id,
                link: link.path,
                offset: 0,
                limit: 32768,
            },
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(chunk.data, "YQ==");
}
