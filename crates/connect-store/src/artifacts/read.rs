//! Authorized blob reads allocate and read only the requested byte window.
use super::{expired, invalid, metadata, valid_blob_name};
use crate::Store;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_protocol::{ArtifactChunk, Error, Result};
use rusqlite::Connection;
use std::io::{self, Read, Seek, SeekFrom};

pub(crate) fn get_chunk(
    c: &Connection,
    conversation_id: &str,
    artifact_id: &str,
    offset: u64,
    limit: u32,
    store: &Store,
) -> Result<ArtifactChunk> {
    if limit == 0 || limit > store.limits.artifact_chunk_bytes {
        return Err(invalid("Artifact chunk limit exceeds configured bounds"));
    }
    let artifact = metadata(c, conversation_id, artifact_id)?;
    if artifact.state != "ready" || expired(&artifact) || offset > artifact.size {
        return Err(Error::forbidden());
    }
    let blob_name: String = c
        .query_row(
            "SELECT blob_name FROM artifacts WHERE id=?1 AND state='ready'",
            [artifact_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if !valid_blob_name(&blob_name) {
        return Err(Error::storage());
    }
    let path = store.blob_directory.join(blob_name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| Error::storage())?;
    if !metadata.file_type().is_file() || metadata.len() != artifact.size {
        return Err(Error::storage());
    }
    let mut file = std::fs::File::open(path).map_err(|_| Error::storage())?;
    // Validate the opened handle too: a replaced/truncated blob must not change
    // the declared range or turn a short read into a successful padded response.
    let opened = file.metadata().map_err(|_| Error::storage())?;
    if !opened.is_file() || opened.len() != artifact.size {
        return Err(Error::storage());
    }
    let length = (artifact.size - offset).min(u64::from(limit));
    let content = read_at(
        &mut file,
        offset,
        usize::try_from(length).map_err(|_| Error::storage())?,
    )
    .map_err(|_| Error::storage())?;
    Ok(ArtifactChunk {
        eof: offset + length == artifact.size,
        artifact,
        offset,
        data: STANDARD.encode(content),
    })
}

fn read_at(reader: &mut (impl Read + Seek), offset: u64, length: usize) -> io::Result<Vec<u8>> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut content = vec![0; length];
    reader.read_exact(&mut content)?;
    Ok(content)
}

#[cfg(test)]
mod tests;
