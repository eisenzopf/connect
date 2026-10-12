//! Shared resolution for native and server-routed assistant voice Sessions.
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection};

/// Resolve a single enabled assistant from either supported Vapi route kind.
/// The parent conversation and assistant identity remain the authority boundary.
pub(crate) fn vapi_assistant(
    connection: &Connection,
    conversation_id: &str,
    assistant_identity_id: &str,
) -> Result<Option<String>> {
    let mut statement = connection
        .prepare(
            "SELECT assistant_id FROM voice_routes
             WHERE conversation_id=?1 AND identity_id=?2 AND enabled=1
             UNION
             SELECT target FROM connector_routes
             WHERE conversation_id=?1 AND identity_id=?2 AND connector='vapi' AND enabled=1
             LIMIT 2",
        )
        .map_err(|_| Error::storage())?;
    let mut assistants = statement
        .query_map(params![conversation_id, assistant_identity_id], |row| {
            row.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if assistants.len() > 1 {
        return Err(Error::new(
            "conflict",
            "Session has more than one enabled assistant route",
        ));
    }
    Ok(assistants.pop())
}
