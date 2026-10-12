//! Non-secret names for authenticated task principals, distinct from credentials.
use crate::{Error, Result};

/// Reserved transport principal namespace; never a model-supplied authority field.
pub const TASK_PRINCIPAL_PREFIX: &str = "connect-task-grant:";

/// Construct an expected authentication subject from protected grant metadata.
/// Possessing this name alone grants no authority.
pub fn task_grant_subject(grant_id: &str) -> Result<String> {
    uuid::Uuid::parse_str(grant_id).map_err(|_| Error::unauthorized())?;
    Ok(format!("{TASK_PRINCIPAL_PREFIX}{grant_id}"))
}

/// Decode a previously authenticated subject. A malformed reserved name fails
/// closed rather than being interpreted as a backing credential.
pub fn task_grant_id(subject: &str) -> Result<&str> {
    let id = subject
        .strip_prefix(TASK_PRINCIPAL_PREFIX)
        .ok_or_else(Error::unauthorized)?;
    uuid::Uuid::parse_str(id).map_err(|_| Error::unauthorized())?;
    Ok(id)
}
