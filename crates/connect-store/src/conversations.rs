use crate::{cursor, identity::actor_from_connection, Actor, Store};
use connect_protocol::{Conversation, Error, Event, Message, Operation, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

impl Store {
    /// One transaction and current authority check own each application operation.
    /// Receipt/effect/event admission is serialized even across separate processes.
    pub fn execute(&self, credential_id: &str, operation: &Operation) -> Result<Outcome> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential_id)?;
        actor.require(operation.name())?;
        let outcome = match operation {
            Operation::Resolve { address } => {
                let alias = address
                    .strip_prefix("connect:")
                    .and_then(|a| a.strip_suffix("@local"))
                    .ok_or_else(|| invalid("Use connect:alias@local"))?;
                let (id, alias, capabilities): (String, String, String) = tx
                    .query_row(
                        "SELECT id,alias,capabilities FROM identities WHERE alias=?1",
                        [alias],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()
                    .map_err(|_| Error::storage())?
                    .ok_or_else(Error::forbidden)?;
                if !actor.permits_recipient(&id) {
                    return Err(Error::forbidden());
                }
                Outcome::Identity(connect_protocol::Identity {
                    id,
                    alias,
                    capabilities: serde_json::from_str(&capabilities)
                        .map_err(|_| Error::storage())?,
                })
            }
            Operation::Health {} => {
                let id: String = tx
                    .query_row("SELECT id FROM installation", [], |r| r.get(0))
                    .map_err(|_| Error::storage())?;
                Outcome::Health {
                    installation_id: id,
                    schema: crate::SCHEMA_VERSION,
                    profile: connect_protocol::PROFILE.into(),
                }
            }
            Operation::History {
                conversation_id,
                after,
                limit,
            } => {
                let _ = authorized_conversation(&tx, &actor, conversation_id)?;
                page_limit(*limit, self.limits.page_size)?;
                let after = cursor::decode(
                    &tx,
                    &actor.identity_id,
                    &format!("history:{conversation_id}"),
                    after.as_deref(),
                )?;
                let max: i64 = tx
                    .query_row("SELECT COALESCE(MAX(sequence),0) FROM messages", [], |r| {
                        r.get(0)
                    })
                    .map_err(|_| Error::storage())?;
                if after > max {
                    return Err(invalid("Cursor is newer than this state snapshot"));
                }
                let mut statement=tx.prepare("SELECT sequence,id,conversation_id,sender,body FROM messages WHERE conversation_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(|_|Error::storage())?;
                let rows = statement
                    .query_map(params![conversation_id, after, limit], |r| {
                        Ok((r.get::<_, i64>(0)?, message_row_offset(r, 1)?))
                    })
                    .map_err(|_| Error::storage())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|_| Error::storage())?;
                let mut bounded = Vec::new();
                let mut bytes = 0;
                for row in rows {
                    let size = serde_json::to_vec(&row.1)
                        .map_err(|_| Error::storage())?
                        .len();
                    if bytes + size > connect_protocol::MAX_PAGE_BYTES {
                        break;
                    }
                    bytes += size;
                    bounded.push(row);
                }
                let next = bounded
                    .last()
                    .map(|(seq, _)| {
                        cursor::encode(
                            &tx,
                            &actor.identity_id,
                            &format!("history:{conversation_id}"),
                            *seq,
                        )
                    })
                    .transpose()?;
                Outcome::History {
                    messages: bounded.into_iter().map(|(_, m)| m).collect(),
                    next,
                }
            }
            Operation::Inbox {
                cursor: after,
                limit,
                wait_ms,
            } => {
                page_limit(*limit, self.limits.page_size)?;
                if *wait_ms > self.limits.wait_ms {
                    return Err(invalid("Inbox wait exceeds configured limit"));
                }
                let after_seq = cursor::decode(&tx, &actor.identity_id, "inbox", after.as_deref())?;
                let max: i64 = tx
                    .query_row("SELECT COALESCE(MAX(sequence),0) FROM journal", [], |r| {
                        r.get(0)
                    })
                    .map_err(|_| Error::storage())?;
                if after_seq > max {
                    return Err(invalid("Cursor is newer than this state snapshot"));
                }
                let mut statement=tx.prepare("SELECT sequence,id,conversation_id,kind,message_id FROM journal WHERE recipient=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3").map_err(|_|Error::storage())?;
                let rows = statement
                    .query_map(params![actor.identity_id, after_seq, limit], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    })
                    .map_err(|_| Error::storage())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|_| Error::storage())?;
                let mut events = Vec::new();
                let mut next = after.clone();
                let mut bytes = 0;
                for (sequence, id, conversation_id, kind, message_id) in rows {
                    let token = cursor::encode(&tx, &actor.identity_id, "inbox", sequence)?;
                    match authorized_conversation(&tx, &actor, &conversation_id) {
                        Ok(_) => {}
                        Err(e) if e.code == "forbidden" => {
                            next = Some(token);
                            continue;
                        }
                        Err(e) => return Err(e),
                    }
                    let message = message_id
                        .map(|id| {
                            tx.query_row(
                                "SELECT id,conversation_id,sender,body FROM messages WHERE id=?1",
                                [id],
                                |r| message_row_offset(r, 0),
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
                    };
                    let size = serde_json::to_vec(&event)
                        .map_err(|_| Error::storage())?
                        .len();
                    if bytes + size > connect_protocol::MAX_PAGE_BYTES {
                        break;
                    }
                    bytes += size;
                    next = Some(event.cursor.clone());
                    events.push(event);
                }
                Outcome::Inbox {
                    events,
                    cursor: next,
                }
            }
            Operation::RequestStatus { request_id } => {
                valid_id(request_id)?;
                let saved = receipt(&tx, &actor.identity_id, request_id)?;
                if let Some(saved) = saved {
                    check_receipt_authority(&actor, &saved)?;
                    match saved.result {
                        Ok(result) => Outcome::RequestStatus {
                            state: "accepted".into(),
                            outcome: Some(Box::new(result)),
                        },
                        Err(_) => Outcome::RequestStatus {
                            state: "rejected".into(),
                            outcome: None,
                        },
                    }
                } else {
                    Outcome::RequestStatus {
                        state: "not_found".into(),
                        outcome: None,
                    }
                }
            }
            _ => {
                let request_id = operation
                    .request_id()
                    .ok_or_else(|| invalid("Logical request ID is required"))?;
                valid_id(request_id)?;
                // Resource authority is checked before receipt lookup or mutation.
                let resources = match operation {
                    Operation::Create { participants, .. } => {
                        let mut members = participants.clone();
                        members.push(actor.identity_id.clone());
                        members.sort();
                        members.dedup();
                        if members.len() > self.limits.participants {
                            return Err(invalid("Participant limit exceeded"));
                        }
                        for id in &members {
                            if !actor.permits_recipient(id) {
                                return Err(Error::forbidden());
                            }
                            let exists: bool = tx
                                .query_row(
                                    "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1)",
                                    [id],
                                    |r| r.get(0),
                                )
                                .map_err(|_| Error::storage())?;
                            if !exists {
                                return Err(Error::forbidden());
                            }
                        }
                        members
                    }
                    Operation::Send {
                        conversation_id, ..
                    }
                    | Operation::Close {
                        conversation_id, ..
                    } => authorized_conversation(&tx, &actor, conversation_id)?.participants,
                    _ => return Err(invalid("Unsupported operation")),
                };
                let canonical =
                    serde_json::to_value(operation).map_err(|_| invalid("Invalid operation"))?;
                let digest = format!(
                    "v1:{}",
                    hex::encode(Sha256::digest(
                        serde_json::to_vec(&canonical).map_err(|_| Error::storage())?
                    ))
                );
                if let Some(saved) = receipt(&tx, &actor.identity_id, request_id)? {
                    check_receipt_authority(&actor, &saved)?;
                    if saved.digest != digest {
                        return Err(Error::new(
                            "conflict",
                            "Logical request ID already has different inputs",
                        ));
                    }
                    return saved.result;
                }
                let receipts: u64 = tx
                    .query_row("SELECT COUNT(*) FROM requests", [], |r| r.get(0))
                    .map_err(|_| Error::storage())?;
                if receipts >= self.limits.operation_receipts {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Durable request receipt capacity exceeded; no acceptance recorded",
                    ));
                }
                let result = match operation {
                    Operation::Create { .. } => {
                        if capacity(&tx, resources.len() as u64, 0, self)? {
                            let id = uuid::Uuid::new_v4().to_string();
                            tx.execute("INSERT INTO conversations(id) VALUES(?1)", [&id])
                                .map_err(|_| Error::storage())?;
                            for member in &resources {
                                tx.execute(
                                    "INSERT INTO participants VALUES(?1,?2)",
                                    params![id, member],
                                )
                                .map_err(|_| Error::storage())?;
                                insert_event(&tx, member, &id, "conversation_created", None)?;
                            }
                            Ok(Outcome::Conversation(Conversation {
                                id,
                                participants: resources.clone(),
                                closed: false,
                            }))
                        } else {
                            Err(Error::new(
                                "capacity-exceeded",
                                "Durable event capacity exceeded",
                            ))
                        }
                    }
                    Operation::Send {
                        conversation_id,
                        body,
                        ..
                    } => {
                        let c = authorized_conversation(&tx, &actor, conversation_id)?;
                        if c.closed {
                            Err(Error::new(
                                "conversation-closed",
                                "Conversation no longer accepts messages",
                            ))
                        } else if body.is_empty() || body.len() > self.limits.message_bytes {
                            Err(invalid(
                                "Message must be nonempty and within the byte limit",
                            ))
                        } else if !capacity(&tx, resources.len() as u64, body.len() as u64, self)? {
                            Err(Error::new(
                                "capacity-exceeded",
                                "Durable message or event capacity exceeded",
                            ))
                        } else {
                            let id = uuid::Uuid::new_v4().to_string();
                            tx.execute("INSERT INTO messages(id,conversation_id,sender,body) VALUES(?1,?2,?3,?4)",params![id,conversation_id,actor.identity_id,body]).map_err(|_|Error::storage())?;
                            for member in &resources {
                                insert_event(&tx, member, conversation_id, "message", Some(&id))?;
                            }
                            Ok(Outcome::Accepted {
                                request_id: request_id.into(),
                                message_id: id,
                                conversation_id: conversation_id.clone(),
                            })
                        }
                    }
                    Operation::Close {
                        conversation_id, ..
                    } => {
                        let c = authorized_conversation(&tx, &actor, conversation_id)?;
                        if c.closed {
                            Ok(Outcome::Conversation(c))
                        } else if !capacity(&tx, resources.len() as u64, 0, self)? {
                            Err(Error::new(
                                "capacity-exceeded",
                                "Durable event capacity exceeded",
                            ))
                        } else {
                            tx.execute(
                                "UPDATE conversations SET closed=1 WHERE id=?1",
                                [conversation_id],
                            )
                            .map_err(|_| Error::storage())?;
                            for member in &resources {
                                insert_event(
                                    &tx,
                                    member,
                                    conversation_id,
                                    "conversation_closed",
                                    None,
                                )?;
                            }
                            Ok(Outcome::Conversation(Conversation { closed: true, ..c }))
                        }
                    }
                    _ => return Err(invalid("Unsupported mutation")),
                };
                // Accepted and authorized rejected outcomes are durable. Storage failures
                // exit above and roll the entire transaction back without a receipt.
                tx.execute(
                    "INSERT INTO requests VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        actor.identity_id,
                        request_id,
                        digest,
                        operation.name(),
                        serde_json::to_string(&resources).map_err(|_| Error::storage())?,
                        serde_json::to_string(&result).map_err(|_| Error::storage())?
                    ],
                )
                .map_err(|_| Error::storage())?;
                #[cfg(feature = "test-support")]
                crate::fault_point("before-commit", request_id);
                tx.commit().map_err(|_| Error::storage())?;
                #[cfg(feature = "test-support")]
                crate::fault_point("after-commit", request_id);
                return result;
            }
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(outcome)
    }
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
fn valid_id(id: &str) -> Result<()> {
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
fn page_limit(value: u32, max: u32) -> Result<()> {
    if value == 0 || value > max {
        Err(invalid("Page size exceeds configured bounds"))
    } else {
        Ok(())
    }
}
fn authorized_conversation(c: &Connection, actor: &Actor, id: &str) -> Result<Conversation> {
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
fn capacity(c: &Connection, events: u64, bytes: u64, store: &Store) -> Result<bool> {
    let stored_events: u64 = c
        .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    let stored_bytes: u64 = c
        .query_row(
            "SELECT COALESCE(SUM(length(CAST(body AS BLOB))),0) FROM messages",
            [],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok(
        stored_events.saturating_add(events) <= store.limits.journal_events
            && stored_bytes.saturating_add(bytes) <= store.limits.message_storage_bytes,
    )
}
fn insert_event(
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
fn message_row_offset(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get(offset)?,
        conversation_id: r.get(offset + 1)?,
        sender: r.get(offset + 2)?,
        body: r.get(offset + 3)?,
    })
}
struct Saved {
    digest: String,
    operation: String,
    resources: Vec<String>,
    result: Result<Outcome>,
}
fn receipt(c: &Connection, actor: &str, id: &str) -> Result<Option<Saved>> {
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
fn check_receipt_authority(actor: &Actor, saved: &Saved) -> Result<()> {
    actor.require(&saved.operation)?;
    if saved.resources.iter().any(|p| !actor.permits_recipient(p)) {
        return Err(Error::forbidden());
    };
    Ok(())
}
