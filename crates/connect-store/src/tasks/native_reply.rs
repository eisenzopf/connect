//! A recipient answers a delivered task question under their own send authority.
//! Task identity is derived from the durable effect, never supplied by the sender.
use super::{authority, replies, storage};
use crate::{
    conversations::{authorized_conversation, capacity, insert_event},
    Actor, Store,
};
use connect_protocol::{
    tasks::{TaskEventData, TaskOutcome, TaskReply, MAX_TASK_INBOX_EVENTS},
    Error, Operation, Result,
};
use rusqlite::{params, Connection};

pub(super) struct Input<'a> {
    pub request_id: &'a str,
    pub conversation_id: &'a str,
    /// The message ID of the question, obtained from the recipient's own inbox.
    pub reply_to: &'a str,
    pub body: &'a str,
}
pub(super) fn execute(
    c: &Connection,
    actor: &Actor,
    input: Input<'_>,
    intent: &str,
    timestamp: i64,
    store: &Store,
) -> Result<TaskOutcome> {
    let conversation = authorized_conversation(c, actor, input.conversation_id)?;
    let task = question(c, actor, input.conversation_id, input.reply_to)?;
    if let Some((_, outcome)) = storage::saved(c, actor, input.request_id, intent)? {
        // This is the sender's communication receipt, not the owner's effect
        // authority. Current sender membership/send scope was checked above.
        return Ok(outcome);
    }
    if conversation.closed {
        return Err(Error::new(
            "conversation-closed",
            "Conversation no longer accepts messages",
        ));
    }
    if input.body.is_empty() || input.body.len() > store.limits.message_bytes {
        return Err(Error::new(
            "invalid-request",
            "Message must be nonempty and within the byte limit",
        ));
    }
    if !capacity(
        c,
        conversation.participants.len() as u64,
        input.body.len() as u64,
        store,
    )? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable message or event capacity exceeded",
        ));
    }
    let active = match authority::continuation(c, &task, timestamp, false) {
        Ok(()) => task.record.scope.recipients.contains(&actor.identity_id),
        Err(error) if matches!(error.code.as_str(), "forbidden" | "unauthorized") => false,
        Err(error) => return Err(error),
    };
    if active {
        let pending: u32 = c
            .query_row(
                "SELECT COUNT(*) FROM task_replies WHERE task_id=?1 AND consumed=0",
                [&task.record.id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if pending >= MAX_TASK_INBOX_EVENTS {
            return Err(Error::new("capacity-exceeded", "Task reply inbox is full"));
        }
    }
    let message_id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO messages(id,conversation_id,sender,body) VALUES(?1,?2,?3,?4)",
        params![
            message_id,
            input.conversation_id,
            actor.identity_id,
            input.body
        ],
    )
    .map_err(|_| Error::storage())?;
    for member in &conversation.participants {
        insert_event(
            c,
            member,
            input.conversation_id,
            "message",
            Some(&message_id),
        )?;
    }
    crate::providers::enqueue(
        c,
        &message_id,
        input.conversation_id,
        &actor.identity_id,
        &actor.credential_id,
        store.limits.operation_receipts,
    )?;
    let event_id: String = c
        .query_row(
            "SELECT id FROM journal WHERE message_id=?1 AND recipient=?2",
            params![message_id, task.record.owner_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let reply = TaskReply {
        event_id,
        conversation_id: input.conversation_id.into(),
        task_id: active.then(|| task.record.id.clone()),
        reply_to: Some(input.reply_to.into()),
        assigned_by: None,
        consumed: false,
    };
    c.execute(
        "INSERT INTO task_replies(event_id,task_id,reply_to) VALUES(?1,?2,?3)",
        params![reply.event_id, reply.task_id, reply.reply_to],
    )
    .map_err(|_| Error::storage())?;
    if active {
        let record = replies::touch(c, actor, task.record.clone(), timestamp, store)?;
        storage::event_data(
            c,
            actor,
            &record,
            &TaskEventData::Reply(reply.clone()),
            store,
        )?;
    }
    let outcome = TaskOutcome::ReplyAccepted {
        request_id: input.request_id.into(),
        message_id,
        reply,
    };
    storage::save_outcome(
        c,
        actor,
        input.request_id,
        intent,
        &task.record.id,
        &outcome,
        store,
    )?;
    Ok(outcome)
}
fn question(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    message: &str,
) -> Result<storage::StoredTask> {
    let delivered:bool = c.query_row("SELECT EXISTS(SELECT 1 FROM journal WHERE message_id=?1 AND recipient=?2 AND conversation_id=?3 AND kind='message')",params![message,actor.identity_id,conversation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !delivered {
        return Err(Error::forbidden());
    }
    // The effect and accepted Send receipt are written in the same transaction
    // before external provider dispatch. Conflicting mappings fail closed.
    let mut statement = c.prepare("SELECT task_id,operation_json FROM task_effects WHERE json_extract(outcome_json,'$.kind')='accepted' AND json_extract(outcome_json,'$.data.message_id')=?1 LIMIT 2").map_err(|_|Error::storage())?;
    let rows = statement
        .query_map([message], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let [(task_id, operation)] = rows.as_slice() else {
        return Err(Error::forbidden());
    };
    let operation: Operation = serde_json::from_str(operation).map_err(|_| Error::storage())?;
    let task = storage::load(c, task_id)?;
    let Operation::Send {
        conversation_id: cid,
        ..
    } = operation
    else {
        return Err(Error::forbidden());
    };
    let sender: String = c
        .query_row("SELECT sender FROM messages WHERE id=?1", [message], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    if cid != conversation_id
        || task.record.conversation_id != conversation_id
        || sender != task.record.owner_id
        || sender == actor.identity_id
    {
        return Err(Error::forbidden());
    }
    Ok(task)
}
