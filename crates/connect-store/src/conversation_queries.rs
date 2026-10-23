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
            "SELECT target.id,target.alias,target.capabilities FROM identities target JOIN identities owner ON owner.tenant_id=target.tenant_id WHERE target.alias=?1 AND owner.id=?2 AND target.revoked=0 AND owner.revoked=0",
            params![alias,actor.identity_id],
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
    history_query(
        connection,
        actor,
        conversation_id,
        &connect_protocol::messages::HistoryQuery {
            cursor: after.map(str::to_owned),
            limit: Some(limit),
            ..Default::default()
        },
        maximum_page_size,
    )
}

pub(crate) fn history_query(
    connection: &Connection,
    actor: &Actor,
    conversation_id: &str,
    query: &connect_protocol::messages::HistoryQuery,
    maximum_page_size: u32,
) -> Result<Outcome> {
    let after = query.cursor.as_deref();
    let limit = query.limit.unwrap_or(maximum_page_size.min(100));
    if query.since.zip(query.until).is_some_and(|(a, b)| a > b) {
        return Err(invalid("Invalid history interval"));
    }
    authorized_conversation(connection, actor, conversation_id)?;
    page_limit(limit, maximum_page_size)?;
    let mut after = cursor::decode(
        connection,
        &actor.identity_id,
        &format!("history:{conversation_id}"),
        after,
    )?;
    if let Some(id) = &query.since_msg_id {
        crate::audience::require(
            connection,
            actor,
            conversation_id,
            connect_protocol::membership::ContentKind::Message,
            id,
        )?;
        let sequence: i64 = connection
            .query_row(
                "SELECT sequence FROM messages WHERE id=?1 AND conversation_id=?2",
                params![id, conversation_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        after = after.max(sequence);
    }
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
            "SELECT m.sequence,m.id,m.conversation_id,m.sender,m.body FROM messages m JOIN content_audiences a ON a.kind='message' AND a.id=m.id AND a.conversation_id=m.conversation_id JOIN participants p ON p.participant_id=a.participant_id AND p.membership_epoch=a.membership_epoch AND p.active=1 JOIN identities sender ON sender.id=m.sender AND sender.tenant_id=p.tenant_id WHERE m.conversation_id=?1 AND m.sequence>?2 AND p.identity_id=?4 AND (m.sender=?4 OR EXISTS(SELECT 1 FROM json_each(?5) WHERE value IN (m.sender,'*'))) AND (?6 IS NULL OR m.accepted_at>=?6) AND (?7 IS NULL OR m.accepted_at<=?7) ORDER BY m.sequence LIMIT ?3",
        )
        .map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(
            params![
                conversation_id,
                after,
                limit,
                actor.identity_id,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?,
                query.since,
                query.until
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let projected = rows
        .into_iter()
        .map(|(seq, id)| {
            Ok((
                seq,
                crate::messages::project(connection, actor, conversation_id, &id, None, None)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    // Stored attachment references are summaries; omitting full attachments
    // must retain these IDs/type/size rather than erase discoverability.
    let bounded = bounded_rows(projected)?;
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
        .prepare(&format!(
            "SELECT sequence,id,conversation_id,kind,message_id,call_json,session_json,artifact_json,data_json FROM visible_journal WHERE recipient=?1 AND sequence>?2 AND (source_owner IS NULL OR source_owner=?1 OR EXISTS(SELECT 1 FROM json_each(?4) WHERE value IN (source_owner,'*'))) AND {} ORDER BY sequence LIMIT ?3", crate::audience::journal_scope(4)
        ))
        .map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(
            params![
                actor.identity_id,
                after_sequence,
                limit,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?
            ],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;

    let mut events = Vec::new();
    let mut next = after.map(str::to_owned);
    let mut bytes = 0;
    for (sequence, id, conversation_id, kind, message_id, call, session, artifact, data) in rows {
        let token = cursor::encode(connection, &actor.identity_id, "inbox", sequence)?;
        match authorized_conversation(connection, actor, &conversation_id) {
            Ok(_) => {}
            Err(error) if error.code == "forbidden" => {
                next = Some(token);
                continue;
            }
            Err(error) => return Err(error),
        }
        let source_id = |raw: &Option<String>| -> Result<Option<String>> {
            raw.as_ref()
                .map(|s| {
                    serde_json::from_str::<serde_json::Value>(s)
                        .map_err(|_| Error::storage())
                        .map(|v| v.get("id").and_then(|v| v.as_str()).map(str::to_owned))
                })
                .transpose()
                .map(Option::flatten)
        };
        if !crate::audience::event_visible(
            connection,
            actor,
            &conversation_id,
            message_id.as_deref(),
            source_id(&session)?.as_deref(),
            source_id(&artifact)?.as_deref(),
            source_id(&call)?.as_deref(),
        )? {
            next = Some(token);
            continue;
        }
        let message = message_id
            .map(|id| {
                crate::messages::project(connection, actor, &conversation_id, &id, None, None)
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
            receipt: data
                .map(|v| {
                    serde_json::from_str::<serde_json::Value>(&v).map_err(|_| Error::storage())
                })
                .transpose()?
                .and_then(|v| v.get("receipt").cloned())
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| Error::storage())?,
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

fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
