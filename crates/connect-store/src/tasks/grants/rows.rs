//! Metadata projection; no plaintext bearer is stored.
use connect_protocol::{
    tasks::{GrantDefinition, InvocationBinding, InvocationGrant},
    Error, Result,
};
use rusqlite::{Connection, OptionalExtension, Row};
use serde::de::DeserializeOwned;

pub(super) struct GrantRow {
    pub grant: InvocationGrant,
    pub credential_id: String,
    pub nonce: String,
    pub digest: String,
    pub verifier: String,
}
impl GrantRow {
    pub fn definition(&self) -> GrantDefinition {
        GrantDefinition {
            task_id: self.grant.task_id.clone(),
            binding: self.grant.binding.clone(),
            operations: self.grant.operations.clone(),
            expires_at: self.grant.expires_at,
        }
    }
}
fn parse<T: DeserializeOwned>(s: String) -> rusqlite::Result<T> {
    serde_json::from_str(&s).map_err(|_| rusqlite::Error::InvalidQuery)
}
fn row(r: &Row<'_>) -> rusqlite::Result<GrantRow> {
    let kind = serde_json::from_value(serde_json::Value::String(r.get(4)?))
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(GrantRow {
        grant: InvocationGrant {
            id: r.get(0)?,
            task_id: r.get(1)?,
            binding: InvocationBinding {
                adapter: r.get(3)?,
                kind,
                external_id: r.get(5)?,
            },
            operations: parse(r.get(6)?)?,
            expires_at: r.get(7)?,
            revoked: r.get(11)?,
        },
        credential_id: r.get(2)?,
        nonce: r.get(8)?,
        digest: r.get(9)?,
        verifier: r.get(10)?,
    })
}
const COLUMNS: &str="id,task_id,credential_id,adapter,binding_kind,external_id,operations_json,expires_at,nonce,issuance_digest,verifier,revoked";
pub(super) fn get(c: &Connection, id: &str) -> Result<GrantRow> {
    c.query_row(
        &format!("SELECT {COLUMNS} FROM task_grants WHERE id=?1"),
        [id],
        row,
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::unauthorized)
}
pub(super) fn issuance(c: &Connection, actor: &str, request: &str) -> Result<Option<GrantRow>> {
    c.query_row(
        &format!(
            "SELECT {COLUMNS} FROM task_grants WHERE issuance_actor=?1 AND issuance_request_id=?2"
        ),
        rusqlite::params![actor, request],
        row,
    )
    .optional()
    .map_err(|_| Error::storage())
}
pub(super) fn kind(binding: &InvocationBinding) -> Result<String> {
    serde_json::from_value(serde_json::to_value(binding.kind).map_err(|_| Error::storage())?)
        .map_err(|_| Error::storage())
}
