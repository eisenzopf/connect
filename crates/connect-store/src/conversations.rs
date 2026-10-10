use crate::{identity::actor_from_connection, Actor, Store};
use connect_protocol::{Conversation, Error, Operation, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

impl Store {
    /// One transaction and current authority check own each application operation.
    /// Receipt/effect/event admission is serialized even across separate processes.
    pub fn execute(&self, credential_id: &str, operation: &Operation) -> Result<Outcome> {
        self.execute_with_runtime(
            credential_id,
            operation,
            &crate::DiscoveryRuntime::default(),
        )
    }
    /// Execute with the server's configured transport snapshot; current authority stays transactional.
    pub fn execute_with_runtime(
        &self,
        credential_id: &str,
        operation: &Operation,
        runtime: &crate::DiscoveryRuntime,
    ) -> Result<Outcome> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential_id)?;
        actor.require(operation.required_scope())?;
        let outcome = match operation {
            Operation::Capabilities {
                address,
                conversation_id,
            } => Outcome::Capabilities(crate::discovery::capabilities(
                &tx,
                &actor,
                address,
                conversation_id.as_deref(),
                runtime,
                self.limits(),
            )?),
            Operation::BrowserBinding {} => {
                Outcome::BrowserBinding(crate::discovery::browser_binding(&tx, &actor, runtime)?)
            }
            Operation::ContextDeliveries {
                conversation_id,
                before,
                limit,
            } => {
                authorized_conversation(&tx, &actor, conversation_id)?;
                page_limit(*limit, self.limits.page_size)?;
                Outcome::ContextDeliveries(crate::context::deliveries(
                    &tx,
                    &actor,
                    conversation_id,
                    before.as_deref(),
                    *limit,
                )?)
            }
            Operation::Resolve { address } => {
                crate::conversation_queries::resolve(&tx, &actor, address)?
            }
            Operation::ProviderStatus { conversation_id } => {
                authorized_conversation(&tx, &actor, conversation_id)?;
                Outcome::ProviderStatus {
                    deliveries: crate::providers::status(
                        &tx,
                        conversation_id,
                        self.limits.page_size,
                    )?,
                }
            }
            Operation::CallStatus {
                conversation_id,
                call_id,
            } => {
                authorized_conversation(&tx, &actor, conversation_id)?;
                Outcome::Call(crate::voice::get(&tx, conversation_id, call_id)?)
            }
            Operation::SessionGet {
                conversation_id,
                session_id,
            } => {
                authorized_conversation(&tx, &actor, conversation_id)?;
                Outcome::Session(crate::sessions::get(&tx, conversation_id, session_id)?)
            }
            Operation::SipStatus {
                conversation_id,
                sip_leg_id,
            } => {
                authorized_conversation(&tx, &actor, conversation_id)?;
                Outcome::SipLeg(crate::sip_legs::status(&tx, conversation_id, sip_leg_id)?)
            }
            Operation::AvailabilityGet { identity_id } => {
                if !actor.permits_recipient(identity_id) {
                    return Err(Error::forbidden());
                }
                Outcome::Availability(crate::availability::get(&tx, identity_id)?)
            }
            Operation::ContextView {
                conversation_id,
                limit,
            } => {
                page_limit(*limit, self.limits.page_size)?;
                authorized_conversation(&tx, &actor, conversation_id)?;
                Outcome::ContextView(crate::context::view(&tx, &actor, conversation_id, *limit)?)
            }
            Operation::ArtifactGet {
                conversation_id,
                artifact_id,
                offset,
                limit,
            } => {
                authorized_conversation(&tx, &actor, conversation_id)?;
                Outcome::ArtifactChunk(crate::artifacts::get_chunk(
                    &tx,
                    conversation_id,
                    artifact_id,
                    *offset,
                    *limit,
                    self,
                )?)
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
            } => crate::conversation_queries::history(
                &tx,
                &actor,
                conversation_id,
                after.as_deref(),
                *limit,
                self.limits.page_size,
            )?,
            Operation::Inbox {
                cursor,
                limit,
                wait_ms,
            } => crate::conversation_queries::inbox(
                &tx,
                &actor,
                cursor.as_deref(),
                *limit,
                *wait_ms,
                self.limits.page_size,
                self.limits.wait_ms,
            )?,
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
                    | Operation::CallStart {
                        conversation_id, ..
                    }
                    | Operation::CallEnd {
                        conversation_id, ..
                    }
                    | Operation::SessionAccept {
                        conversation_id, ..
                    }
                    | Operation::SessionReject {
                        conversation_id, ..
                    }
                    | Operation::SessionCancel {
                        conversation_id, ..
                    }
                    | Operation::SessionEnd {
                        conversation_id, ..
                    }
                    | Operation::SipDial {
                        conversation_id, ..
                    }
                    | Operation::SipEnd {
                        conversation_id, ..
                    }
                    | Operation::ArtifactAppend {
                        conversation_id, ..
                    }
                    | Operation::ArtifactFinish {
                        conversation_id, ..
                    }
                    | Operation::ArtifactDelete {
                        conversation_id, ..
                    }
                    | Operation::Close {
                        conversation_id, ..
                    } => authorized_conversation(&tx, &actor, conversation_id)?.participants,
                    Operation::SessionInvite {
                        conversation_id, ..
                    }
                    | Operation::ArtifactBegin {
                        conversation_id, ..
                    } => {
                        let conversation = authorized_conversation(&tx, &actor, conversation_id)?;
                        if conversation.closed {
                            return Err(Error::new(
                                "conversation-closed",
                                "Conversation no longer accepts new sessions or artifacts",
                            ));
                        }
                        conversation.participants
                    }
                    Operation::AvailabilitySet { .. } => vec![actor.identity_id.clone()],
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
                    Operation::CallStart {
                        conversation_id,
                        route_id,
                        destination,
                        ..
                    } => crate::voice::start(
                        &tx,
                        &actor,
                        credential_id,
                        conversation_id,
                        route_id,
                        destination,
                        self,
                    ),
                    Operation::CallEnd {
                        conversation_id,
                        call_id,
                        ..
                    } => crate::voice::end(&tx, credential_id, conversation_id, call_id, self),
                    Operation::SessionInvite {
                        conversation_id,
                        participant_id,
                        media,
                        ..
                    } => crate::sessions::invite(
                        &tx,
                        &actor,
                        conversation_id,
                        participant_id,
                        media,
                        &resources,
                        self,
                    )
                    .map(Outcome::Session),
                    Operation::SessionAccept {
                        conversation_id,
                        session_id,
                        ..
                    } => crate::sessions::transition(
                        &tx,
                        &actor,
                        conversation_id,
                        session_id,
                        crate::sessions::Transition::Accept,
                        &resources,
                        self,
                    )
                    .map(Outcome::Session),
                    Operation::SessionReject {
                        conversation_id,
                        session_id,
                        reason,
                        ..
                    } => crate::sessions::transition(
                        &tx,
                        &actor,
                        conversation_id,
                        session_id,
                        crate::sessions::Transition::Reject(reason.clone()),
                        &resources,
                        self,
                    )
                    .map(Outcome::Session),
                    Operation::SessionCancel {
                        conversation_id,
                        session_id,
                        ..
                    } => crate::sessions::transition(
                        &tx,
                        &actor,
                        conversation_id,
                        session_id,
                        crate::sessions::Transition::Cancel,
                        &resources,
                        self,
                    )
                    .map(Outcome::Session),
                    Operation::SessionEnd {
                        conversation_id,
                        session_id,
                        reason,
                        ..
                    } => crate::sessions::transition(
                        &tx,
                        &actor,
                        conversation_id,
                        session_id,
                        crate::sessions::Transition::End(reason.clone()),
                        &resources,
                        self,
                    )
                    .map(Outcome::Session),
                    Operation::SipDial {
                        conversation_id,
                        session_id,
                        destination,
                        ..
                    } => crate::sip_legs::dial(
                        &tx,
                        &actor,
                        credential_id,
                        conversation_id,
                        session_id,
                        destination,
                    )
                    .map(Outcome::SipLeg),
                    Operation::SipEnd {
                        conversation_id,
                        sip_leg_id,
                        ..
                    } => crate::sip_legs::end(&tx, &actor, conversation_id, sip_leg_id)
                        .map(Outcome::SipLeg),
                    Operation::AvailabilitySet {
                        state,
                        preferred_route,
                        expires_at,
                        ..
                    } => crate::availability::set(
                        &tx,
                        &actor,
                        state,
                        preferred_route.as_deref(),
                        *expires_at,
                    )
                    .map(Outcome::Availability),
                    Operation::ArtifactBegin {
                        conversation_id,
                        name,
                        media_type,
                        size,
                        sha256,
                        expires_at,
                        ..
                    } => crate::artifacts::begin(
                        &tx,
                        &actor,
                        crate::artifacts::BeginRequest {
                            conversation_id,
                            name,
                            media_type,
                            size: *size,
                            sha256,
                            expires_at: *expires_at,
                            recipients: &resources,
                        },
                        self,
                    )
                    .map(Outcome::Artifact),
                    Operation::ArtifactAppend {
                        conversation_id,
                        artifact_id,
                        offset,
                        data,
                        ..
                    } => crate::artifacts::append(
                        &tx,
                        &actor,
                        conversation_id,
                        artifact_id,
                        *offset,
                        data,
                        self,
                    )
                    .map(Outcome::Artifact),
                    Operation::ArtifactFinish {
                        conversation_id,
                        artifact_id,
                        ..
                    } => crate::artifacts::finish(
                        &tx,
                        &actor,
                        conversation_id,
                        artifact_id,
                        &resources,
                        self,
                    )
                    .map(Outcome::Artifact),
                    Operation::ArtifactDelete {
                        conversation_id,
                        artifact_id,
                        ..
                    } => crate::artifacts::delete(
                        &tx,
                        &actor,
                        conversation_id,
                        artifact_id,
                        &resources,
                        self,
                    )
                    .map(Outcome::Artifact),
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
                            crate::providers::enqueue(
                                &tx,
                                &id,
                                conversation_id,
                                &actor.identity_id,
                                credential_id,
                                self.limits.operation_receipts,
                            )?;
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
                        } else if let Err(error) =
                            crate::sessions::ensure_conversation_closable(&tx, conversation_id)
                        {
                            Err(error)
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
