//! Durable canonical control receipts, without a separate task implementation.
use super::invalid;
use crate::Actor;
use connect_protocol::{Error, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension};
pub(super) fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(invalid(
            "Logical request ID must be 1–128 ASCII letters, digits, hyphens or underscores",
        ));
    };
    Ok(())
}
pub(super) struct Saved {
    pub digest: String,
    pub operation: String,
    pub resources: Vec<String>,
    pub result: Result<Outcome>,
}
pub(super) fn receipt(c: &Connection, actor: &str, id: &str) -> Result<Option<Saved>> {
    let row: Option<(String, String, String, String)> = c
        .query_row(
            "SELECT digest,operation,resources,result FROM requests WHERE actor=?1 AND id=?2",
            params![actor, id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    row.map(|(digest, operation, resources, result)| {
        Ok(Saved {
            digest,
            operation,
            resources: serde_json::from_str(&resources).map_err(|_| Error::storage())?,
            result: serde_json::from_str(&result).map_err(|_| Error::storage())?,
        })
    })
    .transpose()
}
pub(super) fn check_receipt_authority(actor: &Actor, saved: &Saved) -> Result<()> {
    actor.require(
        if matches!(
            saved.operation.as_str(),
            "contact_pairing_start" | "contact_pairing_complete" | "contact_pairing_revoke"
        ) {
            "resolve"
        } else {
            &saved.operation
        },
    )?;
    if saved.resources.iter().any(|p| !actor.permits_recipient(p)) {
        return Err(Error::forbidden());
    };
    Ok(())
}
