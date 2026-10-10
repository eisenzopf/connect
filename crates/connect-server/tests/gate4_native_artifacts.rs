//! Actual native task-grant byte transfer, replay and current resource checks.
#![allow(clippy::unwrap_used)]
#[path = "support/native_artifacts.rs"]
mod artifacts;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use artifacts::{artifact, completed, Fixture};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::{Operation, Outcome};
use sha2::{Digest, Sha256};

#[tokio::test]
async fn native_artifact_bytes_resume_and_reads_apply_current_scope_and_deletion() {
    let mut f = Fixture::start().await;
    let bytes: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
    let upload = f
        .begin(bytes.len() as u64, &hex::encode(Sha256::digest(&bytes)))
        .await;
    let first = Operation::ArtifactAppend {
        request_id: "chunk-1".into(),
        conversation_id: f.h.cid.clone(),
        artifact_id: upload.id.clone(),
        offset: 0,
        data: STANDARD.encode(&bytes[..2048]),
    };
    let initial = f.invoke("chunk-1", first.clone()).await;
    assert_eq!(artifact(initial.clone()).received, 2048);
    f.client.close().await;
    f.client = ConnectClient::open(f.config.clone()).await.unwrap();
    f.client.negotiate_task(&f.task.id).await.unwrap();
    assert_eq!(
        f.invoke("chunk-1", first).await,
        initial,
        "reconnect duplicated a chunk"
    );
    let second = f
        .invoke(
            "chunk-2",
            Operation::ArtifactAppend {
                request_id: "chunk-2".into(),
                conversation_id: f.h.cid.clone(),
                artifact_id: upload.id.clone(),
                offset: 2048,
                data: STANDARD.encode(&bytes[2048..]),
            },
        )
        .await;
    assert_eq!(artifact(second).received, bytes.len() as u64);
    let finish = f
        .invoke(
            "finish",
            Operation::ArtifactFinish {
                request_id: "finish".into(),
                conversation_id: f.h.cid.clone(),
                artifact_id: upload.id.clone(),
            },
        )
        .await;
    assert_eq!(artifact(finish).state, "ready");

    let read_grant = grants::issue(
        &f.h,
        &f.task,
        "read-grant",
        vec![TaskPermission::ArtifactGet],
        chrono::Utc::now().timestamp() + 120,
    )
    .await;
    let config =
        f.h.owner_config
            .clone()
            .with_task_grant(&read_grant)
            .unwrap();
    let mut read = ConnectClient::open(config).await.unwrap();
    read.negotiate_task(&f.task.id).await.unwrap();
    let mut received = Vec::new();
    let mut final_read = None;
    for offset in [0, 2048] {
        let request = grants::wrapped(
            &f.task.id,
            &format!("read-{offset}"),
            Operation::ArtifactGet {
                conversation_id: f.h.cid.clone(),
                artifact_id: upload.id.clone(),
                offset,
                limit: 2048,
            },
        );
        let Outcome::ArtifactChunk(chunk) = completed(read.task(request.clone()).await.unwrap())
        else {
            panic!("chunk")
        };
        assert_eq!(chunk.offset, offset);
        assert_eq!(chunk.eof, offset == 2048);
        received.extend(STANDARD.decode(&chunk.data).unwrap());
        final_read = Some(request);
    }
    assert_eq!(received, bytes);
    // No primitive access or owner-token fallback, even for a permitted read.
    assert_eq!(
        grants::code(
            read.artifact_get(&f.h.cid, &upload.id, 0, 2048)
                .await
                .unwrap_err()
        ),
        "forbidden"
    );
    let other =
        f.h.owner
            .create("other", vec![f.h.peer_id.clone()])
            .await
            .unwrap();
    let before = connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let deletion = grants::wrapped(
        &f.task.id,
        "delete-denied",
        Operation::ArtifactDelete {
            request_id: "delete-denied".into(),
            conversation_id: f.h.cid.clone(),
            artifact_id: upload.id.clone(),
        },
    );
    assert_eq!(
        grants::code(read.task(deletion).await.unwrap_err()),
        "forbidden"
    );
    let foreign = grants::wrapped(
        &f.task.id,
        "foreign",
        Operation::ArtifactGet {
            conversation_id: other.id,
            artifact_id: upload.id.clone(),
            offset: 0,
            limit: 2048,
        },
    );
    assert_eq!(
        grants::code(read.task(foreign).await.unwrap_err()),
        "forbidden"
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    let deleted = f
        .invoke(
            "delete",
            Operation::ArtifactDelete {
                request_id: "delete".into(),
                conversation_id: f.h.cid.clone(),
                artifact_id: upload.id,
            },
        )
        .await;
    assert_eq!(artifact(deleted).state, "deleted");
    // Reads are current projections: replaying a prior read cannot resurrect
    // cached bytes after deletion, even with an otherwise valid grant.
    assert_eq!(
        grants::code(read.task(final_read.unwrap()).await.unwrap_err()),
        "forbidden"
    );
    read.close().await;
    f.close().await;
}

#[tokio::test]
async fn grant_revocation_prevents_upload_continuation_without_losing_accepted_bytes() {
    let mut f = Fixture::start().await;
    let bytes = b"first-second";
    let upload = f
        .begin(bytes.len() as u64, &hex::encode(Sha256::digest(bytes)))
        .await;
    let accepted = f
        .invoke(
            "chunk",
            Operation::ArtifactAppend {
                request_id: "chunk".into(),
                conversation_id: f.h.cid.clone(),
                artifact_id: upload.id.clone(),
                offset: 0,
                data: STANDARD.encode(&bytes[..6]),
            },
        )
        .await;
    assert_eq!(artifact(accepted).received, 6);
    let response: TaskAccessResponse = grants::http()
        .post(format!("{}/v1/task-access", grants::url(&f.h)))
        .bearer_auth(&f.h.owner_config.credential)
        .json(&TaskAccessRequest::new(TaskAccessCommand::Revoke {
            grant_id: f.grant.grant.id.clone(),
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    response.result.unwrap();
    let before = connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let continuation = grants::wrapped(
        &f.task.id,
        "revoked-continuation",
        Operation::ArtifactAppend {
            request_id: "revoked-continuation".into(),
            conversation_id: f.h.cid.clone(),
            artifact_id: upload.id.clone(),
            offset: 6,
            data: STANDARD.encode(&bytes[6..]),
        },
    );
    assert!(f.client.task(continuation).await.is_err());
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before,
        "revoked live peer changed accepted artifact state"
    );
    assert!(ConnectClient::open(f.config.clone()).await.is_err());
    let retained =
        f.h.owner
            .artifact_append(
                "owner-remaining",
                &f.h.cid,
                &upload.id,
                6,
                &STANDARD.encode(&bytes[6..]),
            )
            .await
            .unwrap();
    assert_eq!(retained.received, bytes.len() as u64);
    f.h.owner
        .artifact_finish("owner-finish", &f.h.cid, &upload.id)
        .await
        .unwrap();
    let chunk =
        f.h.owner
            .artifact_get(&f.h.cid, &upload.id, 0, 32)
            .await
            .unwrap();
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), bytes);
    f.close().await;
}
