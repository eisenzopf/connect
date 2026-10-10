use super::{
    fixture::Fixture,
    recall_helpers::{grant, page, request, send},
    replies,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_protocol::{tasks::*, Artifact, Operation, Outcome};
use sha2::{Digest, Sha256};

fn artifact(f: &Fixture, id: &str, expires_at: Option<i64>) -> Artifact {
    let bytes = b"PRIVATE_BYTES";
    let Outcome::Artifact(artifact) = f
        .store
        .execute(
            &f.key.id,
            &Operation::ArtifactBegin {
                request_id: format!("begin-{id}"),
                conversation_id: f.cid.clone(),
                name: format!("{id}.bin"),
                media_type: "application/octet-stream".into(),
                size: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
                expires_at,
            },
        )
        .unwrap()
    else {
        panic!("artifact")
    };
    f.store
        .execute(
            &f.key.id,
            &Operation::ArtifactAppend {
                request_id: format!("append-{id}"),
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id.clone(),
                offset: 0,
                data: STANDARD.encode(bytes),
            },
        )
        .unwrap();
    let Outcome::Artifact(ready) = f
        .store
        .execute(
            &f.key.id,
            &Operation::ArtifactFinish {
                request_id: format!("finish-{id}"),
                conversation_id: f.cid.clone(),
                artifact_id: artifact.id,
            },
        )
        .unwrap()
    else {
        panic!("ready")
    };
    ready
}

#[test]
fn artifact_recall_uses_current_state_and_never_returns_bytes_or_server_paths() {
    let f = Fixture::new();
    let ready = artifact(&f, "ready", None);
    let expired = artifact(&f, "expired", Some(f.now + 60));
    let deleted = artifact(&f, "deleted", None);
    let missing = artifact(&f, "missing", None);
    f.store
        .execute(
            &f.key.id,
            &Operation::ArtifactDelete {
                request_id: "delete".into(),
                conversation_id: f.cid.clone(),
                artifact_id: deleted.id.clone(),
            },
        )
        .unwrap();
    std::fs::remove_file(f.store.blob_directory.join(format!("{}.blob", missing.id))).unwrap();
    let task = f.task();
    let result = f
        .store
        .execute_task_at(&f.key.id, &request(&task, None, 100), f.now + 60)
        .unwrap();
    let (sources, _, omitted) = page(result);
    assert_eq!(omitted, 0);
    for (artifact, state) in [
        (&ready, "available"),
        (&expired, "expired"),
        (&deleted, "deleted"),
        (&missing, "unavailable"),
    ] {
        let selected: Vec<_> = sources
            .iter()
            .filter(|s| s.source_id == artifact.id)
            .collect();
        assert!(!selected.is_empty());
        for source in selected {
            assert_eq!(source.state, state);
            assert_eq!(source.created_at, Some(artifact.created_at));
            if state == "available" {
                let metadata: Artifact =
                    serde_json::from_str(source.body.as_ref().unwrap()).unwrap();
                assert_eq!(metadata.id, ready.id);
                assert!(!source.body.as_ref().unwrap().contains("PRIVATE_BYTES"));
                assert!(!source.body.as_ref().unwrap().contains(".blob"));
                assert!(!source
                    .body
                    .as_ref()
                    .unwrap()
                    .contains(f.store.blob_directory.to_str().unwrap()));
            } else {
                assert!(source.body.is_none());
            }
        }
    }
    let (_, invocation) = grant(&f, &task);
    let get = TaskRequest::new(TaskCommand::Invoke {
        request_id: "get".into(),
        task_id: task.id.clone(),
        step_id: "read".into(),
        reply_to: None,
        wait_for_reply: false,
        operation: Box::new(Operation::ArtifactGet {
            conversation_id: f.cid.clone(),
            artifact_id: ready.id,
            offset: 0,
            limit: 4,
        }),
    });
    assert_eq!(
        f.store
            .execute_task_invocation(&invocation, &get)
            .unwrap_err()
            .code,
        "forbidden"
    );
}

#[test]
fn symlink_artifact_is_unavailable_without_following_the_file() {
    let f = Fixture::new();
    let a = artifact(&f, "symlink", None);
    let blob = f.store.blob_directory.join(format!("{}.blob", a.id));
    std::fs::remove_file(&blob).unwrap();
    let outside = f._dir.path().join("private.bin");
    std::fs::write(&outside, b"PRIVATE_BYTES").unwrap();
    std::os::unix::fs::symlink(&outside, &blob).unwrap();
    let task = f.task();
    let (sources, _, _) = page(f.run(&request(&task, None, 100)).unwrap());
    assert!(sources
        .iter()
        .all(|s| s.source_id == a.id && s.state == "unavailable" && s.body.is_none()));
}

#[test]
fn session_recall_has_recorded_timestamp_and_current_ended_leg_metadata() {
    let (f, _, human) = replies::setup();
    let Outcome::Session(session) = f
        .store
        .execute(
            &f.key.id,
            &Operation::SessionInvite {
                request_id: "invite".into(),
                conversation_id: f.cid.clone(),
                participant_id: human.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("session")
    };
    f.store
        .execute(
            &f.key.id,
            &Operation::SessionCancel {
                request_id: "end".into(),
                conversation_id: f.cid.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    let task = replies::task(&f, &human, "task");
    let (sources, _, omitted) = page(f.run(&request(&task, None, 100)).unwrap());
    assert_eq!(omitted, 0);
    assert_eq!(sources.len(), 2);
    for source in sources {
        assert_eq!(source.source_id, session.id);
        assert_eq!(source.created_at, Some(session.created_at));
        let metadata: connect_protocol::Session =
            serde_json::from_str(source.body.as_ref().unwrap()).unwrap();
        assert_eq!(metadata.state, "cancelled");
        assert_eq!(source.actor_id, f.owner);
    }
}

#[test]
fn missing_message_source_is_explicit_and_does_not_prevent_later_recall() {
    let f = Fixture::new();
    let missing = send(&f, &f.cid, "missing", "old");
    send(&f, &f.cid, "retained", "still here");
    // Isolated damaged-source fixture: keep the retained journal reference while
    // removing its body. Normal public operations never bypass foreign keys.
    let c = f.store.connection().unwrap();
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    c.execute("DELETE FROM messages WHERE id=?1", [&missing])
        .unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    drop(c);
    let task = f.task();
    let (sources, _, omitted) = page(f.run(&request(&task, None, 100)).unwrap());
    assert_eq!(omitted, 0);
    assert_eq!(sources[0].source_id, missing);
    assert_eq!(sources[0].state, "unavailable");
    assert!(sources[0].body.is_none());
    assert_eq!(sources[1].body.as_deref(), Some("still here"));
}
