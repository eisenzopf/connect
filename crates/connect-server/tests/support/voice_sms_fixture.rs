//! Public task setup and request construction for the voice-to-SMS journey.
#![allow(clippy::unwrap_used)]
use super::{media, native_tasks::record, task_sms::Enrollment, voice_decision};
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_client::{tasks::*, Artifact, ConnectClient};
use connect_protocol::Operation;
use sha2::{Digest, Sha256};

pub async fn prepare(f: &mut media::Fixture) -> (Enrollment, TaskRecord) {
    let sibling = f.task.clone();
    let contrast =
        f.h.owner
            .create("contrast", vec![f.h.peer_id.clone()])
            .await
            .unwrap();
    f.h.owner
        .send("contrast-fact", &contrast.id, voice_decision::CONTRAST)
        .await
        .unwrap();
    f.h.owner
        .send("review-goal", &f.h.cid, voice_decision::GOAL)
        .await
        .unwrap();
    let enrollment = Enrollment::install(&f.h);
    enrollment.enabled(&f.h, false);
    let mut definition = f.h.definition();
    definition.scope.operations = media::Fixture::permissions();
    definition
        .scope
        .operations
        .extend([TaskPermission::Inbox, TaskPermission::RequestStatus]);
    definition.scope.sms_enrollments = vec![enrollment.reference()];
    f.task = record(
        f.h.owner
            .task(TaskCommand::Create {
                request_id: "voice-sms-review".into(),
                definition,
            })
            .await
            .unwrap(),
    );
    (enrollment, sibling)
}

pub fn followup(task: &TaskRecord, id: &str, body: &str) -> TaskCommand {
    TaskCommand::Invoke {
        request_id: id.into(),
        task_id: task.id.clone(),
        step_id: "post-voice-review".into(),
        operation: Box::new(Operation::Send {
            request_id: id.into(),
            conversation_id: task.conversation_id.clone(),
            message: connect_protocol::messages::MessageDraft::text_all(body),
        }),
        reply_to: None,
        wait_for_reply: true,
    }
}
pub async fn replies(client: &mut ConnectClient, cid: &str) -> Vec<TaskReply> {
    let TaskOutcome::Replies { items, .. } = client
        .task(TaskCommand::Replies {
            conversation_id: cid.into(),
            after: None,
            limit: 100,
        })
        .await
        .unwrap()
    else {
        panic!("replies")
    };
    items
}

/// A separately retrievable draft is available in voice, then deliberately
/// deleted. Recall must preserve its unavailable reference without its bytes.
pub async fn draft(f: &mut media::Fixture) -> Artifact {
    let bytes = b"Private obsolete draft bytes: amber-517.";
    let artifact =
        f.h.owner
            .artifact_begin(
                "draft-begin",
                &f.h.cid,
                "release-draft.txt",
                "text/plain",
                bytes.len() as u64,
                &hex::encode(Sha256::digest(bytes)),
            )
            .await
            .unwrap();
    f.h.owner
        .artifact_append(
            "draft-append",
            &f.h.cid,
            &artifact.id,
            0,
            &STANDARD.encode(bytes),
        )
        .await
        .unwrap();
    let artifact =
        f.h.owner
            .artifact_finish("draft-finish", &f.h.cid, &artifact.id)
            .await
            .unwrap();
    let chunk =
        f.h.owner
            .artifact_get(&f.h.cid, &artifact.id, 0, 32768)
            .await
            .unwrap();
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), bytes);
    artifact
}

pub fn unavailable(sources: &[TaskSource], id: &str) {
    let references: Vec<_> = sources.iter().filter(|s| s.source_id == id).collect();
    assert!(!references.is_empty());
    assert!(references
        .iter()
        .all(|s| s.state == "deleted" && s.body.is_none()));
    assert!(!serde_json::to_string(sources)
        .unwrap()
        .contains("amber-517"));
}
