//! Read-only conversation queries kept separate from the mutation dispatcher.

use crate::{
    conversations::{authorized_conversation, page_limit},
    cursor, Actor,
};
use connect_protocol::{Error, Event, Identity, Message, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn resolve(connection: &Connection, actor: &Actor, address: &str) -> Result<Outcome> {
    let alias = address
        .strip_prefix("connect:")
        .and_then(|value| value.strip_suffix("@local"))
        .ok_or_else(|| invalid("Use connect:alias@local"))?;
    let (id, alias, capabilities): (String, String, String) = connection
        .query_row(
            "SELECT id,alias,capabilities FROM identities WHERE alias=?1",
            [alias],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    if !actor.permits_recipient(&id) {
        return Err(Error::forbidden());
    }
    Ok(Outcome::Identity(Identity {
        id,
        alias,
        capabilities: serde_json::from_str(&capabilities).map_err(|_| Error::storage())?,
    }))
}

pub(crate) fn history(
    connection: &Connection,
    actor: &Actor,
    conversation_id: &str,
    after: Option<&str>,
    limit: u32,
    maximum_page_size: u32,
) -> Result<Outcome> {
    authorized_conversation(connection, actor, conversation_id)?;
    page_limit(limit, maximum_page_size)?;
    let after = cursor::decode(
        connection,
        &actor.identity_id,
        &format!("history:{conversation_id}"),
        after,
    )?;
    let maximum: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM messages",
            [],
            |row| row.get(0),
        )
        .map_err(|_| Error::storage())?;
    if after > maximum {
        return Err(invalid("Cursor is newer than this state snapshot"));
    }

    let mut statement = connection
        .prepare(
            "SELECT sequence,id,conversation_id,sender,body FROM messages \
             WHERE conversation_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        )
        .map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(params![conversation_id, after, limit], |row| {
            Ok((row.get::<_, i64>(0)?, message_row(row, 1)?))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let bounded = bounded_rows(rows)?;
    let next = bounded
        .last()
        .map(|(sequence, _)| {
            cursor::encode(
                connection,
                &actor.identity_id,
                &format!("history:{conversation_id}"),
                *sequence,
            )
        })
        .transpose()?;
    Ok(Outcome::History {
        messages: bounded.into_iter().map(|(_, message)| message).collect(),
        next,
    })
}

pub(crate) fn inbox(
    connection: &Connection,
    actor: &Actor,
    after: Option<&str>,
    limit: u32,
    wait_ms: u32,
    maximum_page_size: u32,
    maximum_wait_ms: u32,
) -> Result<Outcome> {
    page_limit(limit, maximum_page_size)?;
    if wait_ms > maximum_wait_ms {
        return Err(invalid("Inbox wait exceeds configured limit"));
    }
    let after_sequence = cursor::decode(connection, &actor.identity_id, "inbox", after)?;
    let maximum: i64 = connection
        .query_row("SELECT COALESCE(MAX(sequence),0) FROM journal", [], |row| {
            row.get(0)
        })
        .map_err(|_| Error::storage())?;
    if after_sequence > maximum {
        return Err(invalid("Cursor is newer than this state snapshot"));
    }

    let mut statement = connection
        .prepare(
            "SELECT sequence,id,conversation_id,kind,message_id,call_json,session_json,artifact_json \
             FROM journal WHERE recipient=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        )
        .map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(params![actor.identity_id, after_sequence, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;

    let mut events = Vec::new();
    let mut next = after.map(str::to_owned);
    let mut bytes = 0;
    for (sequence, id, conversation_id, kind, message_id, call, session, artifact) in rows {
        let token = cursor::encode(connection, &actor.identity_id, "inbox", sequence)?;
        match authorized_conversation(connection, actor, &conversation_id) {
            Ok(_) => {}
            Err(error) if error.code == "forbidden" => {
                next = Some(token);
                continue;
            }
            Err(error) => return Err(error),
        }
        let message = message_id
            .map(|id| {
                connection
                    .query_row(
                        "SELECT id,conversation_id,sender,body FROM messages WHERE id=?1",
                        [id],
                        |row| message_row(row, 0),
                    )
                    .map_err(|_| Error::storage())
            })
            .transpose()?;
        let event = Event {
            id,
            kind,
            conversation_id,
            message,
            cursor: token,
            call: decode_optional(call)?,
            session: decode_optional(session)?,
            artifact: decode_optional(artifact)?,
        };
        let event_bytes = serde_json::to_vec(&event)
            .map_err(|_| Error::storage())?
            .len();
        if bytes + event_bytes > connect_protocol::MAX_PAGE_BYTES {
            break;
        }
        bytes += event_bytes;
        next = Some(event.cursor.clone());
        events.push(event);
    }
    Ok(Outcome::Inbox {
        events,
        cursor: next,
    })
}

fn bounded_rows(rows: Vec<(i64, Message)>) -> Result<Vec<(i64, Message)>> {
    let mut bounded = Vec::new();
    let mut bytes = 0;
    for row in rows {
        let row_bytes = serde_json::to_vec(&row.1)
            .map_err(|_| Error::storage())?
            .len();
        if bytes + row_bytes > connect_protocol::MAX_PAGE_BYTES {
            break;
        }
        bytes += row_bytes;
        bounded.push(row);
    }
    Ok(bounded)
}

fn decode_optional<T: serde::de::DeserializeOwned>(value: Option<String>) -> Result<Option<T>> {
    value
        .map(|encoded| serde_json::from_str(&encoded).map_err(|_| Error::storage()))
        .transpose()
}

fn message_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(offset)?,
        conversation_id: row.get(offset + 1)?,
        sender: row.get(offset + 2)?,
        body: row.get(offset + 3)?,
    })
}

fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
