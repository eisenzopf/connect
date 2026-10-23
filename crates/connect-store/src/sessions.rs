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
    Cancel(Option<String>),
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
    // Presence is a conversation Participant at its current membership epoch,
    // independent of the two identity fields retained by routed invitations.
    for (identity, state) in [
        (actor.identity_id.as_str(), "present"),
        (participant_id, "invited"),
    ] {
        let member = crate::audience::member(c, identity, conversation_id, true)?;
        c.execute(
            "INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) SELECT ?1,?2,p.tenant_id,p.participant_id,p.membership_epoch,'participant',?3 FROM participants p WHERE p.participant_id=?4 AND p.membership_epoch=?5 AND p.active=1",
            params![session.id, conversation_id, state, member.participant_id, member.membership_epoch],
        ).map_err(|_| Error::storage())?;
    }
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
            if matches!(current.state.as_str(), "accepted" | "ready" | "active") {
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
        Transition::Cancel(reason) => {
            if actor.identity_id != current.initiator_id {
                return Err(Error::forbidden());
            }
            if current.state == "cancelled" {
                return Ok(current);
            }
            if current.state != "invited" {
                return Err(conflict("Only an invited session can be cancelled"));
            }
            validate_reason(reason.as_deref())?;
            ("cancelled", reason, "session_cancelled")
        }
        Transition::End(reason) => {
            if current.state == "ended" {
                return Ok(current);
            }
            if !matches!(current.state.as_str(), "accepted" | "ready" | "active") {
                return Err(conflict("Only a live session can be ended"));
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
    if next == "accepted" {
        let member = crate::audience::member(c, &actor.identity_id, conversation_id, true)?;
        let changed = c.execute(
            "UPDATE session_participants SET state='present' WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='invited'",
            params![session_id, member.participant_id, member.membership_epoch],
        ).map_err(|_| Error::storage())?;
        if changed != 1 {
            return Err(Error::forbidden());
        }
    } else {
        c.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND state NOT IN ('left','removed')", [session_id])
            .map_err(|_| Error::storage())?;
    }
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
            || !matches!(session.state.as_str(), "accepted" | "ready" | "active")
        {
            return Err(Error::forbidden());
        }
        if session.media != ["audio"] {
            return Err(Error::new(
                "unsupported-media-profile",
                "The configured SIP or legacy browser route supports audio only",
            ));
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
        let _authority = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = crate::identity::actor_from_connection(&c, route_credential_id)?;
                actor.require("session_invite")?;
                crate::conversations::authorized_conversation(&c, &actor, conversation_id)?;
                crate::authority_admission::conversation_scopes(
                    &c,
                    conversation_id,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
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
        let _authority = self.runtime_conversation_authority(|c| {
            Ok(vec![get_by_id(c, session_id)?.conversation_id])
        })?;
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
        tx.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND state NOT IN ('left','removed')", [session_id])
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

    /// Classify an existing media binding for trusted peer lifetime supervision.
    /// Current credential, membership and participant authority are checked in
    /// the same transaction as state. An ended Session permits exact transport
    /// retirement only; this never authorizes another attachment or media write.
    pub fn authorize_session_lifetime(
        &self,
        credential_id: &str,
        session_id: &str,
    ) -> Result<Session> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
        authorize_binding_lifetime(&tx, &actor, session_id)
    }

    /// Return terminal durable media Sessions among the supplied runtime projections.
    /// This operational lookup also reconciles idle projections with no live socket.
    /// Callers own the process namespace; credential admission remains a separate check.
    pub fn terminal_media_session_ids(&self, session_ids: &[String]) -> Result<Vec<String>> {
        if session_ids.len() > 10_000 {
            return Err(Error::new(
                "capacity-exceeded",
                "Media projection capacity exceeded",
            ));
        }
        let connection = self.connection()?;
        let mut terminal = Vec::new();
        for chunk in session_ids.chunks(256) {
            let placeholders = std::iter::repeat_n("(?)", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!("WITH requested(id) AS (VALUES {placeholders}) SELECT requested.id FROM requested LEFT JOIN sessions s ON s.id=requested.id LEFT JOIN conversations c ON c.id=s.conversation_id WHERE s.id IS NULL OR s.state NOT IN ('accepted','ready','active') OR c.closed=1");
            let mut query = connection.prepare(&sql).map_err(|_| Error::storage())?;
            let rows = query
                .query_map(rusqlite::params_from_iter(chunk), |row| {
                    row.get::<_, String>(0)
                })
                .map_err(|_| Error::storage())?;
            for row in rows {
                terminal.push(row.map_err(|_| Error::storage())?);
            }
        }
        Ok(terminal)
    }

    /// Resolve the one enabled Vapi assistant that may answer an initiator's
    /// accepted Session. The durable route is bound to the invited assistant
    /// identity, so merely knowing a Session ID cannot trigger a provider call.
    pub fn vapi_session_route(
        &self,
        credential_id: &str,
        session_id: &str,
    ) -> Result<Option<VapiSessionRoute>> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let native = credential_id.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX);
        let (actor, session) = if native {
            let authority = Self::native_session_authority(&tx, credential_id, session_id)?;
            (authority.actor, authority.session)
        } else {
            let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
            actor.require("session_get")?;
            let session = authorize_binding(&tx, &actor, session_id)?;
            (actor, session)
        };
        if actor.identity_id != session.initiator_id
            || !matches!(session.state.as_str(), "accepted" | "ready" | "active")
        {
            return Ok(None);
        }
        crate::conversations::authorized_conversation(&tx, &actor, &session.conversation_id)?;
        let route = crate::session_routes::vapi_assistant(
            &tx,
            &session.conversation_id,
            &session.participant_id,
        )?
        .map(|assistant_id| VapiSessionRoute {
            conversation_id: session.conversation_id,
            session_id: session.id,
            assistant_identity_id: session.participant_id,
            assistant_id,
        });
        if route.is_some() && session.media.iter().any(|kind| kind == "video") {
            return Err(Error::new(
                "unsupported-media-profile",
                "The configured Vapi route supports audio only",
            ));
        }
        Ok(route)
    }

    /// Record one authenticated live connection. The socket remains rvoip
    /// state; this row is a restart-safe observation and never resurrects it.
    pub fn bind_session_connection(
        &self,
        credential_id: &str,
        session_id: &str,
        connection_id: &str,
    ) -> Result<Session> {
        let _authority = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let current =
                    if credential_id.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX) {
                        Self::native_session_authority(&c, credential_id, session_id)?.session
                    } else {
                        let actor = crate::identity::actor_from_connection(&c, credential_id)?;
                        authorize_binding(&c, &actor, session_id)?
                    };
                crate::authority_admission::conversation_scopes(
                    &c,
                    &current.conversation_id,
                    crate::AuthorityMode::Write,
                )
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (actor, current, grant_id) =
            if credential_id.starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX) {
                let authority = Self::native_session_authority(&tx, credential_id, session_id)?;
                (authority.actor, authority.session, Some(authority.grant_id))
            } else {
                let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
                let session = authorize_binding(&tx, &actor, session_id)?;
                (actor, session, None)
            };
        let members =
            crate::conversations::authorized_conversation(&tx, &actor, &current.conversation_id)?
                .participants;
        if current.media.iter().any(|kind| kind == "video") {
            let connected: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM session_bindings WHERE session_id=?1 AND participant_id=?2 AND state='ready' AND connection_id<>?3)",
                params![session_id, actor.identity_id, connection_id],
                |row| row.get(0),
            ).map_err(|_| Error::storage())?;
            if connected {
                return Err(Error::new(
                    "video-participant-already-connected",
                    "The video participant already has an active connection",
                ));
            }
        }
        tx.execute(
            "INSERT INTO session_bindings(session_id,participant_id,credential_id,connection_id,state,updated_at,task_grant_id) VALUES(?1,?2,?3,?4,'ready',?5,?6) ON CONFLICT(session_id,participant_id,connection_id) DO UPDATE SET credential_id=excluded.credential_id,task_grant_id=excluded.task_grant_id,state='ready',updated_at=excluded.updated_at",
            params![session_id, actor.identity_id, actor.credential_id, connection_id, now(), grant_id],
        )
        .map_err(|_| Error::storage())?;
        if !matches!(current.state.as_str(), "ready" | "active") {
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

    /// Reconcile live AV observations after acquiring the installation lock at
    /// process startup. Physical sockets do not survive a process restart; their
    /// durable Sessions remain accepted and require fresh authorized connections.
    pub fn reconcile_interrupted_video_connections(&self) -> Result<usize> {
        let _runtime = self.require_runtime_owner()?;
        let mut total = 0;
        loop {
            let count = self.reconcile_video_batch()?;
            total += count;
            if count == 0 {
                return Ok(total);
            }
        }
    }
    fn reconcile_video_batch(&self) -> Result<usize> {
        let _runtime = self.require_runtime_owner()?;
        let kind = crate::startup_authority::Kind::VideoBindings;
        let (ids, guard) = self.startup_batch(kind, 1000)?;
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let candidates = self.startup_rows(&tx, kind, &ids, &guard)?;
        let mut reconciled = 0;
        for candidate in candidates {
            let id = candidate.id;
            let session = get_by_id(&tx, &id)?;
            if !session.media.iter().any(|kind| kind == "video") {
                continue;
            }
            tx.execute(
                "UPDATE session_bindings SET state='ended',updated_at=?1 WHERE session_id=?2 AND state='ready'",
                params![now(), id],
            ).map_err(|_| Error::storage())?;
            if session.state == "ready" {
                tx.execute(
                    "UPDATE sessions SET state='accepted',updated_at=?1 WHERE id=?2",
                    params![now(), id],
                )
                .map_err(|_| Error::storage())?;
                let session = get_by_id(&tx, &id)?;
                let members = members(&tx, &session.conversation_id)?;
                if !capacity(&tx, members.len() as u64, 0, self)? {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Durable event capacity exceeded",
                    ));
                }
                emit(&tx, &members, &session, "session_interrupted")?;
            }
            reconciled += 1;
        }
        self.require_runtime_owner()?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(reconciled)
    }

    /// Record a transient native connection interruption without ending its
    /// reconnectable durable Session.
    pub fn unbind_session_connection(&self, connection_id: &str) -> Result<Option<Session>> {
        let _authority = self.runtime_conversation_authority(|c| {
            let mut st = c.prepare("SELECT DISTINCT s.conversation_id FROM session_bindings b JOIN sessions s ON s.id=b.session_id WHERE b.connection_id=?1 LIMIT 1025").map_err(|_| Error::storage())?;
            let cids = st.query_map([connection_id], |r| r.get::<_, String>(0)).map_err(|_| Error::storage())?.collect::<std::result::Result<Vec<_>, _>>().map_err(|_| Error::storage())?;
            if cids.len()>1024 {return Err(Error::new("capacity-exceeded", "Authority scope capacity exceeded"));}
            Ok(cids)
        })?;
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
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE conversation_id=?1 AND state NOT IN ('ended','rejected','cancelled','failed')) OR EXISTS(SELECT 1 FROM voice_calls WHERE conversation_id=?1 AND state NOT IN ('ended','cancelled','failed','forbidden')) OR EXISTS(SELECT 1 FROM sip_legs WHERE conversation_id=?1 AND state NOT IN ('ended','cancelled','failed','forbidden')) OR EXISTS(SELECT 1 FROM operations WHERE conversation_id=?1 AND ledger_version=1 AND state NOT IN ('completed','cancelled','failed'))",
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
    let session = authorize_binding_lifetime(c, actor, session_id)?;
    if !matches!(session.state.as_str(), "accepted" | "ready" | "active") {
        return Err(Error::forbidden());
    }
    Ok(session)
}

/// Current authority for either a live binding or its explicit durable end.
pub(crate) fn authorize_binding_lifetime(
    c: &Connection,
    actor: &Actor,
    session_id: &str,
) -> Result<Session> {
    actor.require("session_get")?;
    let session = get_by_id(c, session_id)?;
    crate::audience::require(
        c,
        actor,
        &session.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        session_id,
    )?;
    // Sharing historical Session metadata never revives old live presence.
    require_original_epoch(c, &actor.identity_id, &session)?;
    let conversation =
        crate::conversations::authorized_conversation(c, actor, &session.conversation_id)?;
    if conversation.closed
        || !matches!(
            session.state.as_str(),
            "accepted" | "ready" | "active" | "ended"
        )
    {
        return Err(Error::forbidden());
    }
    let state: Option<String> = c.query_row(
        "SELECT sp.state FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.conversation_id=sp.conversation_id AND p.tenant_id=sp.tenant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 WHERE sp.session_id=?1 AND p.identity_id=?2",
        params![session.id, actor.identity_id], |r| r.get(0),
    ).optional().map_err(|_| Error::storage())?;
    if !matches!(
        (session.state.as_str(), state.as_deref()),
        ("accepted" | "ready" | "active", Some("present")) | ("ended", Some("present" | "left"))
    ) {
        return Err(Error::forbidden());
    }
    Ok(session)
}

impl Store {
    /// Current, recipient-filtered Session appearances. A historical audience or
    /// a readmitted Participant never revives presence from an earlier epoch.
    pub fn session_presence(
        &self,
        credential_id: &str,
        conversation_id: &str,
        session_id: &str,
    ) -> Result<Vec<connect_protocol::events::SessionAppearance>> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential_id)?;
        actor.require("session_get")?;
        crate::conversations::authorized_conversation(&tx, &actor, conversation_id)?;
        crate::audience::require(
            &tx,
            &actor,
            conversation_id,
            connect_protocol::membership::ContentKind::Session,
            session_id,
        )?;
        get(&tx, conversation_id, session_id)?;
        let mut statement = tx.prepare("SELECT sp.participant_id,sp.membership_epoch,sp.role,sp.state,p.identity_id FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.conversation_id=sp.conversation_id AND p.tenant_id=sp.tenant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0 WHERE sp.session_id=?1 AND sp.conversation_id=?2 ORDER BY sp.participant_id").map_err(|_| Error::storage())?;
        let rows = statement
            .query_map(params![session_id, conversation_id], |r| {
                Ok((
                    connect_protocol::events::SessionAppearance {
                        session_id: session_id.into(),
                        participant_id: r.get(0)?,
                        membership_epoch: r.get(1)?,
                        role: r.get(2)?,
                        state: r.get(3)?,
                    },
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| Error::storage())?;
        let mut appearances = Vec::new();
        for row in rows {
            let (appearance, identity) = row.map_err(|_| Error::storage())?;
            if actor.permits_recipient(&identity) {
                appearances.push(appearance);
            }
        }
        Ok(appearances)
    }
}

pub(crate) fn require_original_epoch(
    c: &Connection,
    identity: &str,
    session: &Session,
) -> Result<()> {
    crate::audience::member(c, identity, &session.conversation_id, true)?;
    let bound: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM resource_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.membership_epoch=a.membership_epoch AND p.active=1 WHERE a.kind='session' AND a.id=?1 AND a.conversation_id=?2 AND p.identity_id=?3)",params![session.id,session.conversation_id,identity],|r|r.get(0)).map_err(|_|Error::storage())?;
    if bound {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}

#[cfg(test)]
#[path = "session_lifetime_tests.rs"]
mod lifetime_tests;

#[cfg(test)]
#[path = "session_presence_tests.rs"]
mod presence_tests;
