//! Domain-separated grant issuance. Plaintext never appears in a stored receipt.
use connect_protocol::{tasks::InvocationGrant, Error, Result};
use hmac::{Hmac, Mac};
use rusqlite::Connection;
use sha2::Sha256;
use std::fmt;

/// Secret returned only from trusted explicit issuance; not serializable.
pub struct IssuedTaskGrant {
    pub grant: InvocationGrant,
    secret: String,
}
impl fmt::Debug for IssuedTaskGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IssuedTaskGrant")
            .field("grant", &self.grant)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}
impl IssuedTaskGrant {
    /// Deliberately export to protected host configuration. Never log this value.
    pub fn expose_secret(&self) -> &str {
        &self.secret
    }
    pub(super) fn new(grant: InvocationGrant, secret: String) -> Self {
        Self { grant, secret }
    }
}
fn mac(c: &Connection, id: &str, nonce: &str, digest: &str) -> Result<Hmac<Sha256>> {
    let key: String = c
        .query_row(
            "SELECT cursor_key FROM installation WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| Error::storage())?;
    mac.update(b"connect-task-grant/1\0");
    for part in [id, nonce, digest] {
        mac.update(part.as_bytes());
        mac.update(b"\0");
    }
    Ok(mac)
}
pub(super) fn mint(c: &Connection, id: &str, nonce: &str, digest: &str) -> Result<String> {
    let signature = hex::encode(mac(c, id, nonce, digest)?.finalize().into_bytes());
    Ok(format!("ctg1.{id}.{signature}"))
}
pub(super) fn split(token: &str) -> Result<(&str, Vec<u8>)> {
    if token.len() > 256 {
        return Err(Error::unauthorized());
    }
    let mut parts = token.split('.');
    if parts.next() != Some("ctg1") {
        return Err(Error::unauthorized());
    }
    let id = parts.next().ok_or_else(Error::unauthorized)?;
    uuid::Uuid::parse_str(id).map_err(|_| Error::unauthorized())?;
    let signature = hex::decode(parts.next().ok_or_else(Error::unauthorized)?)
        .map_err(|_| Error::unauthorized())?;
    if signature.len() != 32 || parts.next().is_some() {
        return Err(Error::unauthorized());
    }
    Ok((id, signature))
}
pub(super) fn verify(
    c: &Connection,
    id: &str,
    nonce: &str,
    digest: &str,
    signature: &[u8],
) -> Result<()> {
    mac(c, id, nonce, digest)?
        .verify_slice(signature)
        .map_err(|_| Error::unauthorized())
}
