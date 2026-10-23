//! Bounded draft bytes shared through task-scoped public SDK operations.
use super::{
    ledger::{execute, id},
    state::AppStore,
    workflow, Draft,
};
use anyhow::{ensure, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{tasks::*, Artifact, ConnectClient, Operation, Outcome};
use sha2::{Digest, Sha256};

const CHUNK: usize = 4096;
const MAX_BYTES: usize = 32768;

/// Publish one original upload before sending the review question. Every chunk
/// has a durable intent; replay never invents a new upload or advances an offset.
pub(super) async fn publish(
    store: &mut AppStore,
    client: &mut ConnectClient,
    task: &TaskRecord,
    draft: &Draft,
) -> Result<Option<Artifact>> {
    let bytes = serde_json::to_vec(draft)?;
    ensure!(
        bytes.len() <= MAX_BYTES,
        "Serialized draft exceeds artifact limit"
    );
    let cid = &task.conversation_id;
    let operation = Operation::ArtifactBegin {
        to: connect_client::messages::MessageAudience::all(),
        request_id: id(),
        conversation_id: cid.clone(),
        name: format!("{}.json", draft.id),
        media_type: "application/json".into(),
        size: bytes.len() as u64,
        sha256: draft.digest()?,
        expires_at: None,
    };
    let Some(start) = mutate(store, client, task, "begin", operation).await? else {
        return Ok(None);
    };
    verify_metadata(&start, task, draft, bytes.len())?;
    for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
        let offset = (index * CHUNK) as u64;
        let operation = Operation::ArtifactAppend {
            request_id: id(),
            conversation_id: cid.clone(),
            artifact_id: start.id.clone(),
            offset,
            data: STANDARD.encode(chunk),
        };
        let Some(progress) =
            mutate(store, client, task, &format!("append:{offset}"), operation).await?
        else {
            return Ok(None);
        };
        verify_metadata(&progress, task, draft, bytes.len())?;
        ensure!(
            progress.id == start.id && progress.received == offset + chunk.len() as u64,
            "Upload progress does not match original chunk"
        );
    }
    let operation = Operation::ArtifactFinish {
        request_id: id(),
        conversation_id: cid.clone(),
        artifact_id: start.id.clone(),
    };
    let Some(artifact) = mutate(store, client, task, "finish", operation).await? else {
        return Ok(None);
    };
    verify_metadata(&artifact, task, draft, bytes.len())?;
    ensure!(
        artifact.id == start.id && artifact.state == "ready",
        "Draft is not ready"
    );
    // Accepted mutation receipts are historical. A fresh read enforces retention
    // and current authorization before we publish a reference to the recipient.
    verify_bytes(client, &artifact).await?;
    store.save("outgoing-artifact", &serde_json::to_string(&artifact)?)?;
    Ok(Some(artifact))
}

async fn mutate(
    store: &mut AppStore,
    client: &mut ConnectClient,
    task: &TaskRecord,
    stage: &str,
    operation: Operation,
) -> Result<Option<Artifact>> {
    let request_id = match &operation {
        Operation::ArtifactBegin { request_id, .. }
        | Operation::ArtifactAppend { request_id, .. }
        | Operation::ArtifactFinish { request_id, .. } => request_id.clone(),
        _ => anyhow::bail!("Expected artifact mutation"),
    };
    let outcome = execute(
        store,
        client,
        &format!("outgoing-artifact:{stage}"),
        workflow::command(TaskCommand::Invoke {
            request_id: request_id.clone(),
            task_id: task.id.clone(),
            step_id: format!("app-{request_id}"),
            operation: Box::new(operation),
            reply_to: None,
            wait_for_reply: false,
        }),
    )
    .await?;
    let Some(outcome) = workflow::tool_outcome(client, outcome).await? else {
        return Ok(None);
    };
    match outcome {
        Outcome::Artifact(artifact) => Ok(Some(artifact)),
        _ => anyhow::bail!("Expected artifact receipt"),
    }
}

fn verify_metadata(
    artifact: &Artifact,
    task: &TaskRecord,
    draft: &Draft,
    size: usize,
) -> Result<()> {
    ensure!(
        artifact.conversation_id == task.conversation_id
            && artifact.owner_id == task.owner_id
            && artifact.size == size as u64
            && artifact.sha256 == draft.digest()?
            && artifact.media_type == "application/json",
        "Draft artifact binding changed"
    );
    Ok(())
}

/// Read current bytes in bounded chunks; never trust a digest supplied in a
/// message or a retained receipt without hashing the actual authorized content.
pub(super) async fn verify_bytes(client: &mut ConnectClient, expected: &Artifact) -> Result<Draft> {
    ensure!(
        expected.size > 0 && expected.size <= MAX_BYTES as u64,
        "Invalid draft size"
    );
    let mut bytes = Vec::with_capacity(expected.size as usize);
    loop {
        let chunk = client
            .artifact_get(
                &expected.conversation_id,
                &expected.id,
                bytes.len() as u64,
                CHUNK as u32,
            )
            .await?;
        ensure!(
            chunk.artifact == *expected
                && chunk.artifact.state == "ready"
                && chunk.offset == bytes.len() as u64,
            "Artifact metadata changed during read"
        );
        let decoded = STANDARD.decode(&chunk.data)?;
        ensure!(
            !decoded.is_empty()
                && decoded.len() <= CHUNK
                && bytes.len() + decoded.len() <= expected.size as usize,
            "Invalid artifact chunk"
        );
        bytes.extend(decoded);
        ensure!(
            chunk.eof == (bytes.len() == expected.size as usize),
            "Invalid artifact EOF"
        );
        if chunk.eof {
            break;
        }
    }
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == expected.sha256,
        "Draft digest mismatch"
    );
    let draft: Draft = serde_json::from_slice(&bytes)?;
    draft.validate()?;
    Ok(draft)
}

/// Resolve only the artifact in the authenticated event's conversation, owned by
/// its sender. Peer input cannot select another conversation or filesystem path.
pub(super) async fn receive(
    client: &mut ConnectClient,
    cid: &str,
    sender: &str,
    reference: &super::DraftFile,
) -> Result<(Draft, Artifact)> {
    let source = client
        .artifact_get(cid, &reference.id, 0, CHUNK as u32)
        .await?
        .artifact;
    ensure!(
        source.id == reference.id
            && source.conversation_id == cid
            && source.owner_id == sender
            && source.sha256 == reference.sha256
            && source.size == reference.size
            && source.media_type == "application/json",
        "Submission artifact binding mismatch"
    );
    let draft = verify_bytes(client, &source).await?;
    Ok((draft, source))
}

/// Peer content failures are visible inbox results. Transport, server and
/// credential failures retain the event for recovery instead of discarding it.
pub(super) fn rejection(error: anyhow::Error) -> Result<&'static str> {
    if let Some(error_kind) = error.downcast_ref::<connect_client::ClientError>() {
        if matches!(error_kind, connect_client::ClientError::Application(e) if e.code == "forbidden")
        {
            return Ok("artifact-unavailable");
        }
        return Err(error);
    }
    Ok("artifact-invalid")
}
