//! Chat dispatch shares the durable context projection used by voice.
use crate::{identity::actor_from_connection, Route};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection};

impl crate::Store {
    /// Acknowledge a definite conversational connector rejection without resubmitting the job.
    /// Unknown remote outcomes cannot be discarded through this operation.
    pub fn discard_failed_chat_job(&self, operator: &crate::Actor, job_id: &str) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let authority = actor_from_connection(&tx, &operator.credential_id)?;
        authority.require("admin")?;
        let (conversation, message): (String,String) = tx.query_row(
            "SELECT r.conversation_id,j.message_id FROM provider_jobs j JOIN connector_routes r ON j.route_id=r.id
             WHERE j.id=?1 AND j.state='failed' AND j.provider_id IS NULL",
            [job_id], |row| Ok((row.get(0)?,row.get(1)?)),
        ).map_err(|_|Error::forbidden())?;
        let connector: String = tx.query_row("SELECT r.connector FROM provider_jobs j JOIN connector_routes r ON j.route_id=r.id WHERE j.id=?1", [job_id], |r| r.get(0)).map_err(|_| Error::storage())?;
        if !self.connector(&connector)?.conversational {
            return Err(Error::forbidden());
        }
        let members = {
            let mut statement = tx
                .prepare("SELECT identity_id FROM participants WHERE conversation_id=?1")
                .map_err(|_| Error::storage())?;
            let members = statement
                .query_map([&conversation], |row| row.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| Error::storage())?;
            members
        };
        if members.is_empty()
            || members
                .iter()
                .any(|member| !authority.permits_recipient(member))
        {
            return Err(Error::forbidden());
        }
        if !crate::conversations::capacity(&tx, members.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable event capacity exceeded",
            ));
        }
        tx.execute(
            "UPDATE provider_jobs SET state='cancelled' WHERE id=?1",
            [job_id],
        )
        .map_err(|_| Error::storage())?;
        for member in members {
            crate::conversations::insert_event(
                &tx,
                &member,
                &conversation,
                "provider_job_discarded",
                Some(&message),
            )?;
        }
        // The context-delivery row retains its failed state and exact selection.
        tx.commit().map_err(|_| Error::storage())
    }
}

pub(crate) fn begin(
    connection: &Connection,
    credential_id: &str,
    route: &Route,
    job_id: &str,
    conversational: bool,
    store: &crate::Store,
) -> Result<(Option<String>, bool)> {
    if !conversational {
        return Ok((None, false));
    }
    let delegate = actor_from_connection(connection, &route.credential_id)?;
    let context = crate::tasks::provider_task_context(connection, job_id, store)?;
    let task_origin = !matches!(context, crate::tasks::ProviderTaskContext::Ordinary);
    // No context permission is a definitive ceiling; never fall back to owner history.
    if !delegate.permits("context_view") {
        return Ok((None, task_origin));
    }
    let (serialized, watermark) = match context {
        crate::tasks::ProviderTaskContext::Restricted => return Ok((None, true)),
        crate::tasks::ProviderTaskContext::Selected { json, watermark } => (json, watermark),
        crate::tasks::ProviderTaskContext::Ordinary => {
            let actor = actor_from_connection(connection, credential_id)?;
            if !actor.permits("context_view") {
                return Ok((None, false));
            }
            let selection = crate::context::view(connection, &actor, &route.conversation_id, 1000)?;
            (
                serde_json::to_string(&selection).map_err(|_| Error::storage())?,
                selection.watermark,
            )
        }
    };
    let id = format!("provider-job:{job_id}");
    let timestamp = crate::now();
    connection.execute(
        "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence)
         VALUES(?1,?2,NULL,?6,?3,?4,'pending',?5,?5,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries))",
        params![id,route.conversation_id,watermark,serialized,timestamp,route.connector],
    ).map_err(|_|Error::storage())?;
    Ok((Some(serialized), task_origin))
}

pub(crate) fn finish(
    connection: &Connection,
    job_id: &str,
    state: &str,
    provider_ref: Option<&str>,
) -> Result<()> {
    let outcome = match state {
        "completed" => "delivered",
        "failed" => "failed",
        _ => "uncertain",
    };
    connection.execute(
        "UPDATE context_deliveries SET state=?2,provider_ref=?3,updated_at=?4 WHERE id=?1 AND state='pending'",
        params![format!("provider-job:{job_id}"),outcome,provider_ref,crate::now()],
    ).map_err(|_|Error::storage())?;
    Ok(())
}
