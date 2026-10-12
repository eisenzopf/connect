//! Independent artifact submissions use authenticated provenance and actual bytes.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/connected_app.rs"]
mod support;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{Artifact, Operation, Outcome};
use connect_review_loop::connected_app::{Draft, DraftFile, Request};
use sha2::{Digest, Sha256};
use support::Fixture;

async fn conversation(f: &Fixture, who: &str) -> String {
    let Outcome::Conversation(c) = f
        .request(
            who,
            Operation::Create {
                request_id: uuid::Uuid::new_v4().to_string(),
                participants: vec![f.identity("reviews")],
            },
        )
        .await
    else {
        panic!("conversation");
    };
    c.id
}
async fn upload(f: &Fixture, who: &str, cid: &str, bytes: &[u8]) -> Artifact {
    let Outcome::Artifact(a) = f
        .request(
            who,
            Operation::ArtifactBegin {
                request_id: uuid::Uuid::new_v4().to_string(),
                conversation_id: cid.into(),
                name: "draft.json".into(),
                media_type: "application/json".into(),
                size: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
                expires_at: None,
            },
        )
        .await
    else {
        panic!("begin");
    };
    for (i, part) in bytes.chunks(4096).enumerate() {
        f.request(
            who,
            Operation::ArtifactAppend {
                request_id: uuid::Uuid::new_v4().to_string(),
                conversation_id: cid.into(),
                artifact_id: a.id.clone(),
                offset: (i * 4096) as u64,
                data: STANDARD.encode(part),
            },
        )
        .await;
    }
    let Outcome::Artifact(a) = f
        .request(
            who,
            Operation::ArtifactFinish {
                request_id: uuid::Uuid::new_v4().to_string(),
                conversation_id: cid.into(),
                artifact_id: a.id,
            },
        )
        .await
    else {
        panic!("finish");
    };
    a
}
async fn submit(
    f: &Fixture,
    who: &str,
    cid: &str,
    artifact: &Artifact,
    request_id: &str,
) -> String {
    let Outcome::Accepted { message_id, .. } = f
        .request(
            who,
            Operation::Send {
                request_id: uuid::Uuid::new_v4().to_string(),
                conversation_id: cid.into(),
                body: serde_json::to_string(&Request::SubmitArtifact {
                    request_id: request_id.into(),
                    artifact: DraftFile {
                        id: artifact.id.clone(),
                        sha256: artifact.sha256.clone(),
                        size: artifact.size,
                    },
                })
                .unwrap(),
            },
        )
        .await
    else {
        panic!("send");
    };
    message_id
}
async fn event_result(f: &Fixture, message: &str, expected: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let c = rusqlite::Connection::open_with_flags(
                f.state(),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let result: Option<String> = c
                .query_row(
                    "SELECT result FROM app_events WHERE json_extract(event,'$.message.id')=?1",
                    [message],
                    |r| r.get(0),
                )
                .ok()
                .flatten();
            drop(c);
            if let Some(result) = result {
                assert_eq!(result, expected);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn incoming_artifact_bytes_duplicates_conflicts_and_unavailable_sources() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None);
    let _ = f.question().await;
    let cid = conversation(&f, "agent").await;
    let draft = Draft {
        id: "incoming-file".into(),
        title: "Uploaded submission".into(),
        body: "\u{1}".repeat(1800),
    };
    // Preserve the source byte digest even when serialization differs from canonical Draft JSON.
    let bytes = serde_json::to_vec_pretty(&draft).unwrap();
    let artifact = upload(&f, "agent", &cid, &bytes).await;
    let message = submit(&f, "agent", &cid, &artifact, "file-1").await;
    event_result(&f, &message, "committed").await;
    let _ = f.settled(0, 1).await;
    let payload = f.business_payload("intake");
    assert_eq!(payload["draft"], serde_json::to_value(&draft).unwrap());
    assert_eq!(payload["artifact"]["id"], artifact.id);
    assert_eq!(payload["artifact"]["sha256"], artifact.sha256);
    assert_ne!(draft.digest().unwrap(), artifact.sha256);
    let duplicate = submit(&f, "agent", &cid, &artifact, "file-1").await;
    event_result(&f, &duplicate, "duplicate").await;
    let other = upload(&f, "agent", &cid, &serde_json::to_vec(&draft).unwrap()).await;
    let conflict = submit(&f, "agent", &cid, &other, "file-1").await;
    event_result(&f, &conflict, "conflict").await;
    let mut bad_digest = artifact.clone();
    bad_digest.sha256 = "0".repeat(64);
    let invalid = submit(&f, "agent", &cid, &bad_digest, "bad-digest").await;
    event_result(&f, &invalid, "artifact-invalid").await;
    let foreign_cid = conversation(&f, "agent").await;
    let foreign = submit(&f, "agent", &foreign_cid, &artifact, "wrong-cid").await;
    event_result(&f, &foreign, "artifact-unavailable").await;
    let bad_json = upload(&f, "agent", &cid, b"not a draft").await;
    let invalid = submit(&f, "agent", &cid, &bad_json, "bad-json").await;
    event_result(&f, &invalid, "artifact-invalid").await;
    f.request(
        "agent",
        Operation::ArtifactDelete {
            request_id: "delete".into(),
            conversation_id: cid.clone(),
            artifact_id: other.id.clone(),
        },
    )
    .await;
    let deleted = submit(&f, "agent", &cid, &other, "deleted").await;
    event_result(&f, &deleted, "artifact-unavailable").await;
    let outside_cid = conversation(&f, "outside").await;
    let denied = submit(&f, "outside", &outside_cid, &artifact, "outside").await;
    event_result(&f, &denied, "denied").await;
    let stats = f.settled(0, 1).await;
    assert_eq!(stats.results.len(), 1);
    assert_eq!(stats.invalid_artifacts, 2);
    assert_eq!(stats.unavailable_artifacts, 2);
    assert_eq!(
        f.history("agent", &cid)
            .await
            .iter()
            .filter(|m| m.body.contains("\"draft_received\""))
            .count(),
        1
    );
    f.finish(&mut app).await;
}

#[tokio::test]
async fn deleted_source_is_not_reused_after_intake_ack_crash() {
    let mut f = Fixture::start().await;
    let mut app = f.spawn(Some("after-intake-task-ack"));
    let _ = f.question().await;
    let cid = conversation(&f, "agent").await;
    let draft = Draft {
        id: "retention".into(),
        title: "Temporary draft".into(),
        body: "Do not use cached bytes after deletion.".into(),
    };
    let artifact = upload(&f, "agent", &cid, &serde_json::to_vec(&draft).unwrap()).await;
    let message = submit(&f, "agent", &cid, &artifact, "retention").await;
    support::exited(&mut app, 86).await;
    f.request(
        "agent",
        Operation::ArtifactDelete {
            request_id: "remove-before-recovery".into(),
            conversation_id: cid.clone(),
            artifact_id: artifact.id,
        },
    )
    .await;
    f.restart_server().await;
    app = f.spawn(None);
    event_result(&f, &message, "artifact-unavailable").await;
    assert!(f.settled(0, 0).await.results.is_empty());
    assert!(f
        .history("agent", &cid)
        .await
        .iter()
        .all(|m| m.sender == f.identity("agent")));
    f.finish(&mut app).await;
}

#[tokio::test]
async fn deleted_outgoing_draft_remains_unavailable_after_restart() {
    use connect_client::tasks::{TaskCommand, TaskRequest};
    let mut f = Fixture::start().await;
    let mut app = f.spawn(None);
    let question = f.question().await;
    let body: serde_json::Value = serde_json::from_str(&question.body).unwrap();
    let artifact: Artifact = serde_json::from_value(body["artifact"].clone()).unwrap();
    let _ = f.settled(0, 0).await;
    support::stop(&mut app).await;
    f.request(
        "reviews",
        Operation::ArtifactDelete {
            request_id: "delete-draft".into(),
            conversation_id: question.conversation_id.clone(),
            artifact_id: artifact.id,
        },
    )
    .await;
    f.restart_server().await;
    app = f.spawn(None);
    let Outcome::Task(result) = f.request("agent", Operation::Task {request: Box::new(TaskRequest::new(TaskCommand::Reply {
        request_id: "approve-deleted".into(), conversation_id: question.conversation_id.clone(), reply_to: question.id.clone(),
        body: serde_json::json!({"action":"review_decision","draft_id":"draft-002","decision":"approved","note":"Late reply"}).to_string(),
    }))}).await else { panic!("reply"); };
    let connect_client::tasks::TaskOutcome::ReplyAccepted { message_id, .. } = *result else {
        panic!("reply receipt");
    };
    event_result(&f, &message_id, "artifact-unavailable").await;
    assert!(f.settled(0, 0).await.results.is_empty());
    assert_eq!(
        f.history("agent", &question.conversation_id)
            .await
            .iter()
            .filter(|m| m.body.contains("\"review_request\""))
            .count(),
        1
    );
    f.finish(&mut app).await;
}
