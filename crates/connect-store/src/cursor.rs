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

/// New event cursors use AEAD so positions and filtered gaps are opaque even to
/// a client that inspects its token. Existing inbox/history cursor formats are
/// independent APIs; these tokens are never accepted as legacy aliases.
pub(crate) fn seal<T: Serialize>(c: &rusqlite::Connection, value: &T) -> Result<String> {
    use base64::Engine;
    use rand::RngCore;
    let key = event_key(c)?;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let json = serde_json::to_vec(value).map_err(|_| Error::storage())?;
    // Fixed plaintext size hides positions, index digit lengths and scope size.
    if json.len() > 1020 {
        return Err(Error::storage());
    }
    let mut data = (json.len() as u32).to_be_bytes().to_vec();
    data.extend(json);
    data.resize(1024, 0);
    key.seal_in_place_append_tag(
        ring::aead::Nonce::assume_unique_for_key(nonce),
        ring::aead::Aad::from(b"connect-events/1"),
        &mut data,
    )
    .map_err(|_| Error::storage())?;
    let mut bytes = nonce.to_vec();
    bytes.extend(data);
    Ok(format!(
        "ev1_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}
pub(crate) fn open<T: serde::de::DeserializeOwned>(
    c: &rusqlite::Connection,
    value: &str,
) -> Result<T> {
    use base64::Engine;
    let invalid = || Error::new("invalid-request", "Invalid event or snapshot cursor");
    if value.len() > 2048 {
        return Err(invalid());
    }
    let mut bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(value.strip_prefix("ev1_").ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    if bytes.len() < 28 {
        return Err(invalid());
    }
    let nonce: [u8; 12] = bytes[..12].try_into().map_err(|_| invalid())?;
    let data = event_key(c)?
        .open_in_place(
            ring::aead::Nonce::assume_unique_for_key(nonce),
            ring::aead::Aad::from(b"connect-events/1"),
            &mut bytes[12..],
        )
        .map_err(|_| invalid())?;
    if data.len() != 1024 {
        return Err(invalid());
    }
    let length = u32::from_be_bytes(data[..4].try_into().map_err(|_| invalid())?) as usize;
    if length > 1020 || data[4 + length..].iter().any(|byte| *byte != 0) {
        return Err(invalid());
    }
    serde_json::from_slice(&data[4..4 + length]).map_err(|_| invalid())
}
fn event_key(c: &rusqlite::Connection) -> Result<ring::aead::LessSafeKey> {
    let secret: String = c
        .query_row(
            "SELECT cursor_key FROM installation WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(b"connect-events/aead-key/1");
    let key =
        ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, &mac.finalize().into_bytes())
            .map_err(|_| Error::storage())?;
    Ok(ring::aead::LessSafeKey::new(key))
}
