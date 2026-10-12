//! Actual draft bytes and upload recovery across fresh process boundaries.
#![cfg(feature = "test-support")]
#![allow(clippy::unwrap_used)]
#[path = "support/connected_app.rs"]
mod support;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{Artifact, Operation, Outcome};
use connect_review_loop::connected_app::{Draft, Policy};
use sha2::{Digest, Sha256};
use support::{exited, Fixture};

async fn read_draft(f: &Fixture, question: &connect_client::Message) -> Artifact {
    let body: serde_json::Value = serde_json::from_str(&question.body).unwrap();
    let expected: Artifact = serde_json::from_value(body["artifact"].clone()).unwrap();
    assert_eq!(expected.conversation_id, question.conversation_id);
    assert_eq!(expected.owner_id, f.identity("reviews"));
    let mut bytes = Vec::new();
    loop {
        let Outcome::ArtifactChunk(chunk) = f
            .request(
                "agent",
                Operation::ArtifactGet {
                    conversation_id: question.conversation_id.clone(),
                    artifact_id: expected.id.clone(),
                    offset: bytes.len() as u64,
                    limit: 1024,
                },
            )
            .await
        else {
            panic!("Expected file bytes");
        };
        assert_eq!(chunk.artifact, expected);
        assert_eq!(chunk.offset, bytes.len() as u64);
        bytes.extend(STANDARD.decode(&chunk.data).unwrap());
        if chunk.eof {
            break;
        }
    }
    assert_eq!(bytes.len() as u64, expected.size);
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected.sha256);
    let draft: Draft = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&draft).unwrap(), body["draft"]);
    assert_eq!(draft.digest().unwrap(), body["digest"]);
    expected
}

#[tokio::test]
async fn upload_ack_loss_recovers_one_file_and_question() {
    for fault in ["after-begin-ack", "after-append-ack", "after-finish-ack"] {
        let mut f = Fixture::start().await;
        // Escaped bytes require multiple chunks despite the 4 KiB body bound.
        let path = f.directory.join("connected-app.json");
        let mut policy: Policy = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        policy.draft.body = "\u{1}".repeat(1800);
        std::fs::write(path, serde_json::to_vec(&policy).unwrap()).unwrap();
        let mut app = f.spawn(Some(fault));
        exited(&mut app, 86).await;
        f.restart_server().await;
        app = f.spawn(None);
        let question = f.question().await;
        let artifact = read_draft(&f, &question).await;
        assert!(artifact.size > 8192);
        let _ = f.settled(0, 0).await;
        support::stop(&mut app).await;
        f.restart_server().await;
        app = f.spawn(None);
        let again = f.question().await;
        let _ = f.settled(0, 0).await;
        assert_eq!(question.id, again.id);
        assert_eq!(read_draft(&f, &again).await.id, artifact.id);
        let Outcome::Inbox { events, .. } = f
            .request(
                "agent",
                Operation::Inbox {
                    cursor: None,
                    limit: 100,
                    wait_ms: 0,
                },
            )
            .await
        else {
            panic!("inbox");
        };
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == "artifact_started")
                .count(),
            1
        );
        assert_eq!(
            events.iter().filter(|e| e.kind == "artifact_ready").count(),
            1
        );
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
}
