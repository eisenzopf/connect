//! Authenticated cursors bind installation, identity, resource and sequence.
use connect_protocol::{Error, Result};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    installation: String,
    identity: String,
    resource: String,
    sequence: i64,
}
pub(crate) fn encode(
    connection: &rusqlite::Connection,
    identity: &str,
    resource: &str,
    sequence: i64,
) -> Result<String> {
    let (installation, key): (String, String) = connection
        .query_row(
            "SELECT id,cursor_key FROM installation WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    let bytes = serde_json::to_vec(&Cursor {
        version: 1,
        installation,
        identity: identity.into(),
        resource: resource.into(),
        sequence,
    })
    .map_err(|_| Error::storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(&bytes);
    Ok(format!(
        "{}.{}",
        hex::encode(&bytes),
        hex::encode(mac.finalize().into_bytes())
    ))
}
pub(crate) fn decode(
    connection: &rusqlite::Connection,
    identity: &str,
    resource: &str,
    cursor: Option<&str>,
) -> Result<i64> {
    let Some(value) = cursor else { return Ok(0) };
    let invalid = || {
        Error::new(
            "invalid-request",
            "Cursor is invalid for this inbox or history",
        )
    };
    if value.len() > 2048 {
        return Err(invalid());
    }
    let (content, signature) = value.split_once('.').ok_or_else(invalid)?;
    let bytes = hex::decode(content).map_err(|_| invalid())?;
    let signature = hex::decode(signature).map_err(|_| invalid())?;
    let (installation, key): (String, String) = connection
        .query_row(
            "SELECT id,cursor_key FROM installation WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(&bytes);
    mac.verify_slice(&signature).map_err(|_| invalid())?;
    let decoded: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if decoded.version != 1
        || decoded.installation != installation
        || decoded.identity != identity
        || decoded.resource != resource
        || decoded.sequence < 0
    {
        return Err(invalid());
    };
    Ok(decoded.sequence)
}
