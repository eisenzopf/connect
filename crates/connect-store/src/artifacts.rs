//! Durable artifact metadata, upload publication and logical deletion.
mod read;
pub(crate) use read::get_chunk;

use crate::{conversations::capacity, conversations::insert_artifact_event, now, Actor, Store};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_protocol::{Artifact, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::io::Write;

pub(crate) struct BeginRequest<'a> {
    pub conversation_id: &'a str,
    pub name: &'a str,
    pub media_type: &'a str,
    pub size: u64,
    pub sha256: &'a str,
    pub expires_at: Option<i64>,
    pub recipients: &'a [String],
}

pub(crate) fn begin(
    c: &Connection,
    actor: &Actor,
    request: BeginRequest<'_>,
    store: &Store,
) -> Result<Artifact> {
    let BeginRequest {
        conversation_id,
        name,
        media_type,
        size,
        sha256,
        expires_at,
        recipients,
    } = request;
    if name.is_empty() || name.len() > 255 || name.contains(['/', '\\', '\0']) {
        return Err(invalid("Artifact name is invalid"));
    }
    if media_type.is_empty()
        || media_type.len() > 127
        || !media_type
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '+' | '-' | '.' | '_'))
    {
        return Err(invalid("Artifact media type is invalid"));
    }
    if size == 0 || size > store.limits.artifact_file_bytes {
        return Err(invalid("Artifact size is outside configured bounds"));
    }
    if sha256.len() != 64 || !sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(invalid(
            "Artifact SHA-256 must be 64 hexadecimal characters",
        ));
    }
    if expires_at.is_some_and(|value| value <= now()) {
        return Err(invalid("Artifact expiry must be in the future"));
    }
    let reserved: u64 = c
        .query_row(
            "SELECT COALESCE(SUM(expected_size),0) FROM artifacts WHERE state!='deleted' AND (expires_at IS NULL OR expires_at>?1)",
            [now()],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if reserved.saturating_add(size) > store.limits.artifact_bytes {
        return Err(Error::new(
            "capacity-exceeded",
            "Artifact capacity exceeded",
        ));
    }
    if !capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    let artifact = Artifact {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.into(),
        owner_id: actor.identity_id.clone(),
        name: name.into(),
        media_type: media_type.into(),
        size,
        received: 0,
        sha256: sha256.to_ascii_lowercase(),
        state: "uploading".into(),
        created_at: now(),
        expires_at,
    };
    c.execute(
        "INSERT INTO artifacts(id,conversation_id,owner_id,name,media_type,expected_size,expected_sha256,state,created_at,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![artifact.id, artifact.conversation_id, artifact.owner_id, artifact.name, artifact.media_type, artifact.size, artifact.sha256, artifact.state, artifact.created_at, artifact.expires_at],
    )
    .map_err(|_| Error::storage())?;
    emit(c, recipients, &artifact, "artifact_started")?;
    Ok(artifact)
}

pub(crate) fn append(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    artifact_id: &str,
    offset: u64,
    data: &str,
    store: &Store,
) -> Result<Artifact> {
    let artifact = metadata(c, conversation_id, artifact_id)?;
    if artifact.owner_id != actor.identity_id {
        return Err(Error::forbidden());
    }
    if expired(&artifact) {
        return Err(Error::forbidden());
    }
    if artifact.state != "uploading" {
        return Err(conflict("Artifact is not accepting chunks"));
    }
    if offset != artifact.received {
        return Err(conflict(
            "Artifact chunk offset does not match durable progress",
        ));
    }
    let decoded = STANDARD
        .decode(data)
        .map_err(|_| invalid("Artifact chunk is not valid padded base64"))?;
    if decoded.is_empty() || decoded.len() > store.limits.artifact_chunk_bytes as usize {
        return Err(invalid("Artifact chunk is outside configured bounds"));
    }
    if artifact.received.saturating_add(decoded.len() as u64) > artifact.size {
        return Err(invalid("Artifact chunk exceeds declared size"));
    }
    let changed = c
        .execute(
            "UPDATE artifacts SET content=CAST(content || ?1 AS BLOB) WHERE id=?2 AND conversation_id=?3 AND state='uploading' AND length(content)=?4",
            params![decoded, artifact_id, conversation_id, offset],
        )
        .map_err(|_| Error::storage())?;
    if changed != 1 {
        return Err(conflict(
            "Artifact progress changed; read it before retrying",
        ));
    }
    metadata(c, conversation_id, artifact_id)
}

pub(crate) fn finish(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    artifact_id: &str,
    recipients: &[String],
    store: &Store,
) -> Result<Artifact> {
    let artifact = metadata(c, conversation_id, artifact_id)?;
    if artifact.owner_id != actor.identity_id {
        return Err(Error::forbidden());
    }
    if expired(&artifact) {
        return Err(Error::forbidden());
    }
    if artifact.state == "ready" {
        return Ok(artifact);
    }
    if artifact.state != "uploading" || artifact.received != artifact.size {
        return Err(conflict("Artifact is incomplete"));
    }
    let content: Vec<u8> = c
        .query_row(
            "SELECT content FROM artifacts WHERE id=?1",
            [artifact_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let digest = hex::encode(Sha256::digest(&content));
    if digest != artifact.sha256 {
        return Err(Error::new(
            "digest-mismatch",
            "Artifact SHA-256 does not match",
        ));
    }
    let blob_name = write_final_blob(store, artifact_id, &content, &digest)?;
    if !capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    c.execute(
        "UPDATE artifacts SET state='ready',blob_name=?1,content=X'',finalized_at=?2 WHERE id=?3",
        params![blob_name, now(), artifact_id],
    )
    .map_err(|_| Error::storage())?;
    let artifact = metadata(c, conversation_id, artifact_id)?;
    emit(c, recipients, &artifact, "artifact_ready")?;
    Ok(artifact)
}

pub(crate) fn delete(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    artifact_id: &str,
    recipients: &[String],
    store: &Store,
) -> Result<Artifact> {
    let artifact = metadata(c, conversation_id, artifact_id)?;
    if artifact.owner_id != actor.identity_id {
        return Err(Error::forbidden());
    }
    if artifact.state == "deleted" {
        return Ok(artifact);
    }
    if !capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    c.execute(
        "UPDATE artifacts SET state='deleted',content=X'' WHERE id=?1",
        [artifact_id],
    )
    .map_err(|_| Error::storage())?;
    let artifact = metadata(c, conversation_id, artifact_id)?;
    emit(c, recipients, &artifact, "artifact_deleted")?;
    Ok(artifact)
}

pub(crate) fn metadata(
    c: &Connection,
    conversation_id: &str,
    artifact_id: &str,
) -> Result<Artifact> {
    c.query_row(
        "SELECT id,conversation_id,owner_id,name,media_type,expected_size,CASE WHEN state='ready' THEN expected_size ELSE length(content) END,expected_sha256,state,created_at,expires_at FROM artifacts WHERE id=?1 AND conversation_id=?2",
        params![artifact_id, conversation_id],
        |r| Ok(Artifact { id:r.get(0)?,conversation_id:r.get(1)?,owner_id:r.get(2)?,name:r.get(3)?,media_type:r.get(4)?,size:r.get(5)?,received:r.get(6)?,sha256:r.get(7)?,state:r.get(8)?,created_at:r.get(9)?,expires_at:r.get(10)? }),
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}

fn emit(c: &Connection, recipients: &[String], artifact: &Artifact, kind: &str) -> Result<()> {
    for recipient in recipients {
        insert_artifact_event(c, recipient, artifact, kind)?;
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
fn conflict(message: &str) -> Error {
    Error::new("conflict", message)
}

fn expired(artifact: &Artifact) -> bool {
    artifact.expires_at.is_some_and(|value| value <= now())
}

pub(crate) fn valid_blob_name(name: &str) -> bool {
    name.len() == 41
        && name.ends_with(".blob")
        && uuid::Uuid::parse_str(&name[..36]).is_ok()
        && !name.contains(['/', '\\', '\0'])
}

pub(crate) fn valid_temporary_blob_name(name: &str) -> bool {
    let Some(body) = name
        .strip_prefix('.')
        .and_then(|value| value.strip_suffix(".tmp"))
    else {
        return false;
    };
    let Some((artifact, nonce)) = body.split_once('.') else {
        return false;
    };
    uuid::Uuid::parse_str(artifact).is_ok()
        && uuid::Uuid::parse_str(nonce).is_ok()
        && !name.contains(['/', '\\', '\0'])
}

fn write_final_blob(
    store: &Store,
    artifact_id: &str,
    content: &[u8],
    expected_digest: &str,
) -> Result<String> {
    let name = format!("{artifact_id}.blob");
    if !valid_blob_name(&name) {
        return Err(Error::storage());
    }
    let destination = store.blob_directory.join(&name);
    if destination.exists() {
        let metadata = std::fs::symlink_metadata(&destination).map_err(|_| Error::storage())?;
        if !metadata.file_type().is_file() {
            return Err(Error::storage());
        }
        let existing = std::fs::read(&destination).map_err(|_| Error::storage())?;
        if hex::encode(Sha256::digest(existing)) != expected_digest {
            return Err(Error::storage());
        }
        return Ok(name);
    }
    let temporary = store
        .blob_directory
        .join(format!(".{artifact_id}.{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| Error::storage())?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(content)?;
        file.sync_all()?;
        std::fs::rename(&temporary, &destination)?;
        std::fs::File::open(&store.blob_directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(Error::storage());
    }
    Ok(name)
}
