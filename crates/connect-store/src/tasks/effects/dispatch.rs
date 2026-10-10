//! Task-qualified execution reuses ordinary operations, including explicit replies.
use super::rows;
use crate::Actor;
use connect_protocol::{
    tasks::{TaskCommand, TaskOutcome, TaskRecord, TaskRequest},
    Error, Operation, Outcome, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn authorize_reply(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    event: Option<&str>,
) -> Result<()> {
    let Some(event) = event else {
        return Ok(());
    };
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM journal j WHERE j.id=?1 AND j.recipient=?2 AND j.conversation_id=?3 AND j.kind='message' AND (j.id=?4 OR EXISTS(SELECT 1 FROM task_replies r WHERE r.event_id=j.id AND r.task_id=?5)))",params![event,actor.identity_id,task.conversation_id,task.initiating_event_id,task.id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if valid {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
pub(super) fn execute(
    execution: super::Execution<'_>,
    request: &TaskRequest,
    operation: &Operation,
) -> Result<Result<Outcome>> {
    let super::Execution {
        c,
        actor,
        task,
        runtime,
        timestamp,
        store,
    } = execution;
    let TaskCommand::Invoke {
        request_id,
        reply_to,
        ..
    } = &request.command
    else {
        return Err(Error::forbidden());
    };
    if let (
        Operation::Send {
            request_id: wire,
            conversation_id,
            body,
        },
        Some(event),
    ) = (operation, reply_to)
    {
        let question:Option<String>=c.query_row("SELECT j.message_id FROM journal j WHERE j.id=?1 AND EXISTS(SELECT 1 FROM task_effects e WHERE json_extract(e.outcome_json,'$.kind')='accepted' AND json_extract(e.outcome_json,'$.data.message_id')=j.message_id)",[event],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        if let Some(question) = question {
            let reply = super::super::native_reply::execute(
                c,
                actor,
                super::super::native_reply::Input {
                    request_id: wire,
                    conversation_id,
                    reply_to: &question,
                    body,
                },
                &super::super::storage::digest(request)?,
                timestamp,
                store,
            )?;
            let TaskOutcome::ReplyAccepted { message_id, .. } = reply else {
                return Err(Error::storage());
            };
            return Ok(Ok(Outcome::Accepted {
                request_id: request_id.clone(),
                message_id,
                conversation_id: task.conversation_id.clone(),
            }));
        }
    }
    let mut result = crate::operations::execute(c, actor, operation, runtime, store)?.outcome;
    if let Ok(Outcome::Accepted {
        request_id: accepted,
        ..
    }) = &mut result
    {
        *accepted = request_id.clone();
    }
    Ok(result)
}

/// Last-mile admission for durable provider jobs. These identifiers come only
/// from committed effect metadata, never from provider headers or model inputs.
pub(crate) fn authorize_effect(c: &Connection, id: &str, timestamp: i64) -> Result<()> {
    let task_id: String = c
        .query_row("SELECT task_id FROM task_effects WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .map_err(|_| Error::forbidden())?;
    let effect = rows::load(c, id, &task_id)?;
    rows::verify(&effect)?;
    let credential = effect
        .credential_id
        .as_deref()
        .ok_or_else(Error::forbidden)?;
    let actor = crate::identity::actor_from_connection_at(c, credential, timestamp)?;
    let task = super::super::authority::task(c, &actor, &task_id)?;
    super::super::authority::continuation(c, &task, timestamp, false)?;
    super::super::authority::operation(c, &actor, &task.record, &effect.operation)?;
    if let Some(grant) = &effect.grant_id {
        super::super::grants::authorize_effect(c, grant, &effect.request(), timestamp)?;
    }
    Ok(())
}
