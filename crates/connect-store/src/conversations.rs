use crate::{identity::actor_from_connection, Actor, Store};
use connect_protocol::{Conversation, Error, Operation, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

impl Store {
    /// Execute one currently authorized operation and atomically retain its receipt.
    pub fn execute(&self, credential_id: &str, operation: &Operation) -> Result<Outcome> {
        self.execute_with_runtime(
            credential_id,
            operation,
            &crate::DiscoveryRuntime::default(),
        )
    }
    /// Runtime discovery does not change transaction ownership or resource authority.
    pub fn execute_with_runtime(
        &self,
        credential_id: &str,
        operation: &Operation,
        runtime: &crate::DiscoveryRuntime,
    ) -> Result<Outcome> {
        if let Operation::Task { request } = operation {
            return self
                .execute_task_with_runtime(credential_id, request, runtime)
                .map(|outcome| Outcome::Task(Box::new(outcome)));
        }
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential_id)?;
        let result = crate::operations::execute(&tx, &actor, operation, runtime, self)?;
        #[cfg(feature = "test-support")]
        if result.new_receipt {
            if let Some(id) = operation.request_id() {
                crate::fault_point("before-commit", id);
            }
        }
        tx.commit().map_err(|_| Error::storage())?;
        #[cfg(feature = "test-support")]
        if result.new_receipt {
            if let Some(id) = operation.request_id() {
                crate::fault_point("after-commit", id);
            }
        }
        result.outcome
    }
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
pub(crate) fn page_limit(value: u32, max: u32) -> Result<()> {
    if value == 0 || value > max {
        Err(invalid("Page size exceeds configured bounds"))
    } else {
        Ok(())
    }
}
pub(crate) fn authorized_conversation(
    c: &Connection,
    actor: &Actor,
    id: &str,
) -> Result<Conversation> {
    let closed: Option<bool> = c
        .query_row("SELECT closed FROM conversations WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|_| Error::storage())?;
    let closed = closed.ok_or_else(Error::forbidden)?;
    let mut stmt = c
        .prepare(
            "SELECT identity_id FROM participants WHERE conversation_id=?1 ORDER BY identity_id",
        )
        .map_err(|_| Error::storage())?;
    let participants = stmt
        .query_map([id], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if !participants.contains(&actor.identity_id)
        || participants.iter().any(|p| !actor.permits_recipient(p))
    {
        return Err(Error::forbidden());
    }
    Ok(Conversation {
        id: id.into(),
        participants,
        closed,
    })
}
pub(crate) fn capacity(c: &Connection, events: u64, bytes: u64, store: &Store) -> Result<bool> {
    let stored_events: u64 = c
        .query_row(
            "SELECT (SELECT COUNT(*) FROM journal)+(SELECT COUNT(*) FROM task_events)",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let stored_bytes: u64 = c
        .query_row(
            "SELECT COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM messages",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(
        crate::tasks::budget::fits(c, store.limits.journal_events, stored_events, events)?
            && stored_bytes.saturating_add(bytes) <= store.limits.message_storage_bytes,
    )
}
pub(crate) fn insert_event(
    c: &Connection,
    recipient: &str,
    conversation: &str,
    kind: &str,
    message: Option<&str>,
) -> Result<()> {
    c.execute(
        "INSERT INTO journal(id,recipient,conversation_id,kind,message_id) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            recipient,
            conversation,
            kind,
            message
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(crate) fn insert_session_event(
    c: &Connection,
    recipient: &str,
    session: &connect_protocol::Session,
    kind: &str,
) -> Result<()> {
    c.execute(
        "INSERT INTO journal(id,recipient,conversation_id,kind,session_json) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            recipient,
            session.conversation_id,
            kind,
            serde_json::to_string(session).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(crate) fn insert_artifact_event(
    c: &Connection,
    recipient: &str,
    artifact: &connect_protocol::Artifact,
    kind: &str,
) -> Result<()> {
    c.execute(
        "INSERT INTO journal(id,recipient,conversation_id,kind,artifact_json) VALUES(?1,?2,?3,?4,?5)",
        params![
            uuid::Uuid::new_v4().to_string(),
            recipient,
            artifact.conversation_id,
            kind,
            serde_json::to_string(artifact).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
