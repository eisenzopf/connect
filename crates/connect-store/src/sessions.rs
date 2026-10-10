use crate::{conversations::capacity, conversations::insert_session_event, now, Actor, Store};
use connect_protocol::{Error, Result, Session};
use rusqlite::{params, Connection, OptionalExtension};

#[derive(Clone, Debug, PartialEq, Eq)]
/// Resolved assistant route for one authorized durable Session.
pub struct VapiSessionRoute {
    /// Durable parent conversation.
    pub conversation_id: String,
    /// Durable bounded session.
    pub session_id: String,
    /// Invited assistant identity.
    pub assistant_identity_id: String,
    /// Provider assistant identifier retained server-side.
    pub assistant_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Result of admitting one explicitly routed inbound SIP/WebRTC leg.
pub struct RoutedInboundSession {
    /// Newly created accepted Session in the configured conversation.
    pub session: Session,
    /// Credential whose current authority admitted the leg.
    pub credential_id: String,
    /// Identity represented by that configured credential.
    pub caller_identity_id: String,
}

pub(crate) enum Transition {
    Accept,
    Reject(Option<String>),
    Cancel,
    End(Option<String>),
}

pub(crate) fn get(c: &Connection, conversation_id: &str, session_id: &str) -> Result<Session> {
    c.query_row(
        "SELECT id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at,ended_reason FROM sessions WHERE id=?1 AND conversation_id=?2",
        params![session_id, conversation_id],
        row,
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}

pub(crate) fn invite(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    participant_id: &str,
    media: &[String],
    recipients: &[String],
    store: &Store,
) -> Result<Session> {
    if participant_id == actor.identity_id
        || !recipients.iter().any(|id| id == participant_id)
        || !actor.permits_recipient(participant_id)
    {
        return Err(Error::forbidden());
    }
    if media.is_empty() || media.len() > 3 {
        return Err(invalid("A session needs one to three media kinds"));
    }
    let mut canonical = media.to_vec();
    canonical.sort();
    canonical.dedup();
    if canonical.len() != media.len()
        || canonical
            .iter()
            .any(|kind| !matches!(kind.as_str(), "audio" | "video" | "data"))
    {
        return Err(invalid(
            "Session media must be unique audio, video, or data values",
        ));
    }
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if count >= store.limits.sessions {
        return Err(Error::new("capacity-exceeded", "Session capacity exceeded"));
    }
    if !capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    let timestamp = now();
    let session = Session {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.into(),
        initiator_id: actor.identity_id.clone(),
        participant_id: participant_id.into(),
        state: "invited".into(),
        media: canonical,
        created_at: timestamp,
        updated_at: timestamp,
        ended_reason: None,
    };
    c.execute(
        "INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![session.id, session.conversation_id, session.initiator_id, session.participant_id, session.state, serde_json::to_string(&session.media).map_err(|_| Error::storage())?, timestamp, timestamp],
    )
    .map_err(|_| Error::storage())?;
    emit(c, recipients, &session, "session_invited")?;
    Ok(session)
}

pub(crate) fn transition(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    session_id: &str,
    transition: Transition,
    recipients: &[String],
    store: &Store,
) -> Result<Session> {
    let current = get(c, conversation_id, session_id)?;
    if actor.identity_id != current.initiator_id && actor.identity_id != current.participant_id {
        return Err(Error::forbidden());
    }
    let (next, reason, event) = match transition {
        Transition::Accept => {
            if actor.identity_id != current.participant_id {
                return Err(Error::forbidden());
            }
            if current.state == "accepted" || current.state == "ready" {
                return Ok(current);
            }
            if current.state != "invited" {
                return Err(conflict("Only an invited session can be accepted"));
            }
            ("accepted", None, "session_accepted")
        }
        Transition::Reject(reason) => {
            if actor.identity_id != current.participant_id {
                return Err(Error::forbidden());
            }
            if current.state == "rejected" {
                return Ok(current);
            }
            if current.state != "invited" {
                return Err(conflict("Only an invited session can be rejected"));
            }
            validate_reason(reason.as_deref())?;
            ("rejected", reason, "session_rejected")
        }
        Transition::Cancel => {
            if actor.identity_id != current.initiator_id {
                return Err(Error::forbidden());
            }
            if current.state == "cancelled" {
                return Ok(current);
            }
            if current.state != "invited" {
                return Err(conflict("Only an invited session can be cancelled"));
            }
            ("cancelled", None, "session_cancelled")
        }
        Transition::End(reason) => {
            if current.state == "ended" {
                return Ok(current);
            }
            if !matches!(current.state.as_str(), "accepted" | "ready") {
                return Err(conflict("Only an accepted or ready session can be ended"));
            }
            validate_reason(reason.as_deref())?;
            ("ended", reason, "session_ended")
        }
    };
    if !capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable event capacity exceeded",
        ));
    }
    let timestamp = now();
    c.execute(
        "UPDATE sessions SET state=?1,updated_at=?2,ended_reason=?3 WHERE id=?4 AND conversation_id=?5",
        params![next, timestamp, reason, session_id, conversation_id],
    )
    .map_err(|_| Error::storage())?;
    let session = get(c, conversation_id, session_id)?;
    emit(c, recipients, &session, event)?;
    Ok(session)
}

impl Store {
    /// Revalidate an administrator-selected credential and accepted Session
    /// before an authenticated transport leg joins that exact Session.
    pub fn admit_configured_session(
        &self,
        route_credential_id: &str,
        conversation_id: &str,
        assistant_identity_id: &str,
        session_id: &str,
    ) -> Result<RoutedInboundSession> {
        let c = self.connection()?;
        let actor = crate::identity::actor_from_connection(&c, route_credential_id)?;
        actor.require("session_get")?;
        let session = get(&c, conversation_id, session_id)?;
        let conversation =
            crate::conversations::authorized_conversation(&c, &actor, conversation_id)?;
        if conversation.closed
            || session.participant_id != assistant_identity_id
            || (actor.identity_id != session.initiator_id
                && actor.identity_id != session.participant_id)
            || !matches!(session.state.as_str(), "accepted" | "ready")
            || !session.media.iter().any(|kind| kind == "audio")
        {
            return Err(Error::forbidden());
        }
        Ok(RoutedInboundSession {
            session,
            credential_id: actor.credential_id,
            caller_identity_id: actor.identity_id,
        })
    }

    /// Admit one unauthenticated SIP/WebRTC connection through an explicit
    /// administrator-configured route. The route credential remains the
    /// authority; signaling metadata and caller ID never select an identity or
    /// conversation.
    pub fn admit_routed_inbound_session(
        &self,
        route_credential_id: &str,
        conversation_id: &str,
        assistant_identity_id: &str,
    ) -> Result<RoutedInboundSession> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, route_credential_id)?;
        actor.require("session_invite")?;
        let members = crate::conversations::authorized_conversation(&tx, &actor, conversation_id)?
            .participants;
        if !members
            .iter()
            .any(|identity| identity == assistant_identity_id)
            || !actor.permits_recipient(assistant_identity_id)
        {
            return Err(Error::forbidden());
        }
        if crate::session_routes::vapi_assistant(&tx, conversation_id, assistant_identity_id)?
            .is_none()
        {
            return Err(Error::forbidden());
        }
        let session = invite(
            &tx,
            &actor,
            conversation_id,
            assistant_identity_id,
            &["audio".into()],
            &members,
            self,
        )?;
        tx.execute(
            "UPDATE sessions SET state='accepted',updated_at=?1 WHERE id=?2",
            params![now(), session.id],
        )
        .map_err(|_| Error::storage())?;
        let session = get(&tx, conversation_id, &session.id)?;
        emit(&tx, &members, &session, "session_accepted")?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(RoutedInboundSession {
            session,
            credential_id: actor.credential_id,
            caller_identity_id: actor.identity_id,
        })
    }

    /// End a server-admitted SIP or WebRTC leg while retaining its parent
    /// conversation. This is an internal runtime transition: external callers
    /// still use the authenticated session operations.
    pub fn end_routed_inbound_session(&self, session_id: &str, reason: &str) -> Result<Session> {
        if reason.is_empty() || reason.len() > 128 {
            return Err(invalid("A routed session end reason is required"));
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let current = get_by_id(&tx, session_id)?;
        if matches!(current.state.as_str(), "ended" | "rejected" | "cancelled") {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(current);
        }
        let members = members(&tx, &current.conversation_id)?;
        if !capacity(&tx, members.len() as u64, 0, self)? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable event capacity exceeded",
            ));
        }
        let timestamp = now();
        tx.execute(
            "UPDATE session_bindings SET state='ended',updated_at=?1 WHERE session_id=?2 AND state='ready'",
            params![timestamp, session_id],
        )
        .map_err(|_| Error::storage())?;
        tx.execute(
            "UPDATE sessions SET state='ended',updated_at=?1,ended_reason=?2 WHERE id=?3",
            params![timestamp, reason, session_id],
        )
        .map_err(|_| Error::storage())?;
        let ended = get_by_id(&tx, session_id)?;
        emit(&tx, &members, &ended, "session_ended")?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ended)
    }

    /// Revalidate the authenticated credential before exposing a durable
    /// Session as a process-wide UCTP media namespace.
    pub fn authorize_session_binding(
        &self,
        credential_id: &str,
        session_id: &str,
    ) -> Result<Session> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
        authorize_binding(&tx, &actor, session_id)
    }

    /// Resolve the one enabled Vapi assistant that may answer an initiator's
    /// accepted Session. The durable route is bound to the invited assistant
    /// identity, so merely knowing a Session ID cannot trigger a provider call.
    pub fn vapi_session_route(
        &self,
        credential_id: &str,
        session_id: &str,
    ) -> Result<Option<VapiSessionRoute>> {
        let c = self.connection()?;
        let actor = crate::identity::actor_from_connection(&c, credential_id)?;
        actor.require("session_get")?;
        let session = get_by_id(&c, session_id)?;
        if actor.identity_id != session.initiator_id
            || !matches!(session.state.as_str(), "accepted" | "ready")
        {
            return Ok(None);
        }
        crate::conversations::authorized_conversation(&c, &actor, &session.conversation_id)?;
        Ok(crate::session_routes::vapi_assistant(
            &c,
            &session.conversation_id,
            &session.participant_id,
        )?
        .map(|assistant_id| VapiSessionRoute {
            conversation_id: session.conversation_id,
            session_id: session.id,
            assistant_identity_id: session.participant_id,
            assistant_id,
        }))
    }

    /// Record one authenticated live connection. The socket remains rvoip
    /// state; this row is a restart-safe observation and never resurrects it.
    pub fn bind_session_connection(
        &self,
        credential_id: &str,
        session_id: &str,
        connection_id: &str,
    ) -> Result<Session> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
        actor.require("session_get")?;
        let current = get_by_id(&tx, session_id)?;
        if actor.identity_id != current.initiator_id && actor.identity_id != current.participant_id
        {
            return Err(Error::forbidden());
        }
        if !matches!(current.state.as_str(), "accepted" | "ready") {
            return Err(Error::forbidden());
        }
        let members =
            crate::conversations::authorized_conversation(&tx, &actor, &current.conversation_id)?
                .participants;
        tx.execute(
            "INSERT INTO session_bindings(session_id,participant_id,credential_id,connection_id,state,updated_at) VALUES(?1,?2,?3,?4,'ready',?5) ON CONFLICT(session_id,participant_id,connection_id) DO UPDATE SET credential_id=excluded.credential_id,state='ready',updated_at=excluded.updated_at",
            params![session_id, actor.identity_id, credential_id, connection_id, now()],
        )
        .map_err(|_| Error::storage())?;
        if current.state != "ready" {
            if !capacity(&tx, members.len() as u64, 0, self)? {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Durable event capacity exceeded",
                ));
            }
            tx.execute(
                "UPDATE sessions SET state='ready',updated_at=?1 WHERE id=?2",
                params![now(), session_id],
            )
            .map_err(|_| Error::storage())?;
            let ready = get_by_id(&tx, session_id)?;
            emit(&tx, &members, &ready, "session_ready")?;
        }
        let result = get_by_id(&tx, session_id)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }

    /// Record a transient native connection interruption without ending its
    /// reconnectable durable Session.
    pub fn unbind_session_connection(&self, connection_id: &str) -> Result<Option<Session>> {
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let session_id: Option<String> = tx
            .query_row(
                "SELECT session_id FROM session_bindings WHERE connection_id=?1 AND state='ready' LIMIT 1",
                [connection_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        let Some(session_id) = session_id else {
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(None);
        };
        tx.execute(
            "UPDATE session_bindings SET state='ended',updated_at=?1 WHERE connection_id=?2",
            params![now(), connection_id],
        )
        .map_err(|_| Error::storage())?;
        let remaining: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM session_bindings WHERE session_id=?1 AND state='ready')",
                [&session_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let mut session = get_by_id(&tx, &session_id)?;
        if !remaining && session.state == "ready" {
            tx.execute(
                "UPDATE sessions SET state='accepted',updated_at=?1 WHERE id=?2",
                params![now(), session_id],
            )
            .map_err(|_| Error::storage())?;
            session = get_by_id(&tx, &session_id)?;
            let members = members(&tx, &session.conversation_id)?;
            if capacity(&tx, members.len() as u64, 0, self)? {
                emit(&tx, &members, &session, "session_interrupted")?;
            }
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(session))
    }
}

fn members(c: &Connection, conversation_id: &str) -> Result<Vec<String>> {
    let mut statement = c
        .prepare(
            "SELECT identity_id FROM participants WHERE conversation_id=?1 ORDER BY identity_id",
        )
        .map_err(|_| Error::storage())?;
    let members = statement
        .query_map([conversation_id], |row| row.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(members)
}

pub(crate) fn ensure_conversation_closable(c: &Connection, conversation_id: &str) -> Result<()> {
    let live: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE conversation_id=?1 AND state IN ('invited','accepted','ready'))",
            [conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if live {
        Err(conflict(
            "End or reject live sessions before closing the conversation",
        ))
    } else {
        Ok(())
    }
}

fn get_by_id(c: &Connection, session_id: &str) -> Result<Session> {
    c.query_row(
        "SELECT id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at,ended_reason FROM sessions WHERE id=?1",
        [session_id],
        row,
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}

fn emit(c: &Connection, recipients: &[String], session: &Session, event: &str) -> Result<()> {
    for recipient in recipients {
        insert_session_event(c, recipient, session, event)?;
    }
    Ok(())
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let media: String = r.get(5)?;
    Ok(Session {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        initiator_id: r.get(2)?,
        participant_id: r.get(3)?,
        state: r.get(4)?,
        media: serde_json::from_str(&media).map_err(|_| rusqlite::Error::InvalidQuery)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        ended_reason: r.get(8)?,
    })
}

fn validate_reason(reason: Option<&str>) -> Result<()> {
    if reason.is_some_and(|value| value.len() > 256) {
        Err(invalid("Session reason exceeds 256 bytes"))
    } else {
        Ok(())
    }
}
fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
fn conflict(message: &str) -> Error {
    Error::new("conflict", message)
}

/// Common current participant and accepted-session boundary, used inside the
/// caller's authority transaction for owner and task-specific composition.
pub(crate) fn authorize_binding(
    c: &Connection,
    actor: &Actor,
    session_id: &str,
) -> Result<Session> {
    actor.require("session_get")?;
    let session = get_by_id(c, session_id)?;
    let conversation =
        crate::conversations::authorized_conversation(c, actor, &session.conversation_id)?;
    if conversation.closed
        || (actor.identity_id != session.initiator_id
            && actor.identity_id != session.participant_id)
        || !matches!(session.state.as_str(), "accepted" | "ready")
    {
        return Err(Error::forbidden());
    }
    Ok(session)
}
