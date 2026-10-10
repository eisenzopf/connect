//! Explicit assignment owns task routing. Sender matching never selects a task.
use super::{authority, lifecycle, storage};
use crate::{
    conversations::{authorized_conversation, page_limit},
    cursor, Actor, Store,
};
use connect_protocol::{
    tasks::{
        TaskEventData, TaskOutcome, TaskRecord, TaskReply, TaskState, MAX_TASK_INBOX_EVENTS,
        TASK_CONTEXT_BYTES,
    },
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) struct Source {
    pub reply: TaskReply,
    pub sender: String,
}
pub(super) fn list(
    c: &Connection,
    actor: &Actor,
    conversation: &str,
    after: Option<&str>,
    limit: u32,
    store: &Store,
) -> Result<TaskOutcome> {
    authorized_conversation(c, actor, conversation)?;
    page_limit(limit, store.limits.page_size)?;
    let resource = format!("task-replies:{conversation}");
    let position = cursor::decode(c, &actor.identity_id, &resource, after)?;
    let maximum: i64 = c
        .query_row("SELECT COALESCE(MAX(sequence),0) FROM journal", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    if position > maximum {
        return Err(Error::new(
            "invalid-request",
            "Cursor is newer than this conversation snapshot",
        ));
    }
    let mut statement = c.prepare("SELECT j.sequence,j.id FROM journal j JOIN messages m ON m.id=j.message_id WHERE j.recipient=?1 AND j.conversation_id=?2 AND j.kind='message' AND m.sender!=?1 AND j.sequence>?3 ORDER BY j.sequence LIMIT ?4").map_err(|_|Error::storage())?;
    let rows = statement
        .query_map(
            params![actor.identity_id, conversation, position, limit],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(|_| Error::storage())?;
    let mut items = Vec::new();
    let mut next = after.map(str::to_owned);
    let mut bytes = 0;
    for row in rows {
        let (sequence, id) = row.map_err(|_| Error::storage())?;
        let reply = source(c, actor, &id)?.reply;
        let length = storage::json(&reply)?.len();
        if bytes + length > TASK_CONTEXT_BYTES {
            break;
        }
        bytes += length;
        next = Some(cursor::encode(c, &actor.identity_id, &resource, sequence)?);
        items.push(reply);
    }
    Ok(TaskOutcome::Replies { items, next })
}
pub(super) fn source(c: &Connection, actor: &Actor, event_id: &str) -> Result<Source> {
    let source = c
        .query_row(
            "SELECT j.conversation_id,m.sender,t.task_id,t.reply_to,t.assigned_by,
                COALESCE(t.consumed,0)
         FROM journal j JOIN messages m ON m.id=j.message_id
         LEFT JOIN task_replies t ON t.event_id=j.id
         WHERE j.id=?1 AND j.recipient=?2 AND j.kind='message'",
            params![event_id, actor.identity_id],
            |r| {
                Ok(Source {
                    sender: r.get(1)?,
                    reply: TaskReply {
                        event_id: event_id.into(),
                        conversation_id: r.get(0)?,
                        task_id: r.get(2)?,
                        reply_to: r.get(3)?,
                        assigned_by: r.get(4)?,
                        consumed: r.get(5)?,
                    },
                })
            },
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    authorized_conversation(c, actor, &source.reply.conversation_id)?;
    Ok(source)
}
pub(super) fn authorize(
    c: &Connection,
    actor: &Actor,
    task: &TaskRecord,
    event: &str,
    consume: bool,
) -> Result<Source> {
    let source = source(c, actor, event)?;
    if source.reply.conversation_id != task.conversation_id
        || source.sender == task.owner_id
        || !task.scope.recipients.contains(&source.sender)
        || source
            .reply
            .task_id
            .as_ref()
            .is_some_and(|id| id != &task.id)
        || (consume && source.reply.task_id.as_ref() != Some(&task.id))
    {
        return Err(Error::forbidden());
    }
    Ok(source)
}
pub(super) fn assign(
    c: &Connection,
    actor: &Actor,
    task: TaskRecord,
    event: &str,
    reply_to: Option<&str>,
    timestamp: i64,
    store: &Store,
) -> Result<TaskRecord> {
    authority::continuation(c, &storage::load(c, &task.id)?, timestamp, false)?;
    let mut source = authorize(c, actor, &task, event, false)?;
    if source.reply.task_id.is_some() {
        return Err(Error::new(
            "reply-conflict",
            "Reply already has an assignment; reconcile its original request",
        ));
    }
    if source
        .reply
        .reply_to
        .as_deref()
        .is_some_and(|original| reply_to.is_some_and(|supplied| original != supplied))
    {
        return Err(Error::new(
            "reply-conflict",
            "An assignment cannot replace the verified question reference",
        ));
    }
    let reply_to = reply_to.or(source.reply.reply_to.as_deref());
    if let Some(question) = reply_to {
        let delivered:bool = c.query_row("SELECT EXISTS(SELECT 1 FROM messages m JOIN journal j ON j.message_id=m.id WHERE m.id=?1 AND m.conversation_id=?2 AND m.sender=?3 AND j.recipient=?4)",params![question,task.conversation_id,actor.identity_id,source.sender],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !delivered {
            return Err(Error::forbidden());
        }
    }
    let pending: u32 = c
        .query_row(
            "SELECT COUNT(*) FROM task_replies WHERE task_id=?1 AND consumed=0",
            [&task.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if pending >= MAX_TASK_INBOX_EVENTS {
        return Err(Error::new("capacity-exceeded", "Task reply inbox is full"));
    }
    source.reply.task_id = Some(task.id.clone());
    let reply_to = reply_to.map(str::to_owned);
    source.reply.reply_to = reply_to.clone();
    source.reply.assigned_by = Some(actor.identity_id.clone());
    c.execute("INSERT INTO task_replies(event_id,task_id,reply_to,assigned_by) VALUES(?1,?2,?3,?4) ON CONFLICT(event_id) DO UPDATE SET task_id=excluded.task_id,reply_to=excluded.reply_to,assigned_by=excluded.assigned_by WHERE task_replies.task_id IS NULL AND task_replies.consumed=0",params![event,task.id,reply_to,actor.identity_id]).map_err(|_|Error::storage())?;
    let task = touch(c, actor, task, timestamp, store)?;
    storage::event_data(c, actor, &task, &TaskEventData::Reply(source.reply), store)?;
    Ok(task)
}
pub(super) fn consume(
    c: &Connection,
    actor: &Actor,
    task: TaskRecord,
    event: &str,
    expected_revision: u64,
    timestamp: i64,
    store: &Store,
) -> Result<TaskRecord> {
    authority::continuation(c, &storage::load(c, &task.id)?, timestamp, false)?;
    let mut source = authorize(c, actor, &task, event, true)?;
    if task.revision != expected_revision {
        return Err(Error::new("task-state-conflict", "Task revision changed"));
    }
    if source.reply.consumed {
        return Err(Error::new("reply-consumed", "Reply was already consumed"));
    }
    c.execute(
        "UPDATE task_replies SET consumed=1 WHERE event_id=?1 AND task_id=?2 AND consumed=0",
        params![event, task.id],
    )
    .map_err(|_| Error::storage())?;
    source.reply.consumed = true;
    let task = touch(c, actor, task, timestamp, store)?;
    storage::event_data(c, actor, &task, &TaskEventData::Reply(source.reply), store)?;
    Ok(task)
}
pub(super) fn touch(
    c: &Connection,
    actor: &Actor,
    mut task: TaskRecord,
    timestamp: i64,
    store: &Store,
) -> Result<TaskRecord> {
    if task.state == TaskState::Waiting {
        return lifecycle::change(c, actor, task, TaskState::Ready, None, timestamp, store);
    }
    task.revision = task.revision.checked_add(1).ok_or_else(Error::storage)?;
    task.updated_at = timestamp;
    c.execute(
        "UPDATE tasks SET revision=?1,updated_at=?2 WHERE id=?3",
        params![task.revision, timestamp, task.id],
    )
    .map_err(|_| Error::storage())?;
    storage::event(c, actor, &task, store)?;
    Ok(task)
}
