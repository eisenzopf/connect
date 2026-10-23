//! Durable Session activation from verified public Connection observations.
use crate::{Actor, AuthorityMode, AuthorityScope, PeerAuthorityGuard, PeerCommand, Store};
use connect_protocol::{
    events::{SessionActiveConnection, SessionStarted},
    membership::{ContentKind, MemberView},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
mod readiness;
pub(crate) use readiness::{ReadyConnection, ReadyOrigin};

fn pending_snapshot(
    c: &Connection,
    store: &Store,
    cid: &str,
    sid: &str,
) -> Result<Option<(Vec<MemberView>, Vec<ReadyConnection>)>> {
    let current = crate::session_control::view(c, cid, sid)?;
    if current.state == "active" {
        return Ok(None);
    }
    if !matches!(current.state.as_str(), "accepted" | "ready") {
        return Ok(None);
    }
    let already: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_started_facts WHERE session_id=?1)",
            [sid],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if already {
        return Ok(None);
    }
    if !readiness::observed_coverage(c, store, sid)? {
        return Ok(None);
    }
    let mut q=c.prepare("SELECT p.identity_id FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE sp.session_id=?1 AND sp.state='present' ORDER BY p.participant_id").map_err(|_|Error::storage())?;
    let ids = q
        .query_map([sid], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    let audience = ids
        .iter()
        .map(|id| crate::audience::member(c, id, cid, true))
        .collect::<Result<Vec<_>>>()?;
    if audience.is_empty() {
        return Ok(None);
    }
    let ready = readiness::current(c, store, sid)?;
    if audience.iter().any(|member| {
        !ready.iter().any(|value| {
            value.connection.participant_id == member.participant_id
                && value.epoch == member.membership_epoch
        })
    }) {
        // Accepted presence and provisional ready JSON cannot stand in for an
        // observed current public Connection for every accepted Participant.
        return Ok(None);
    }
    if ready.iter().any(|value| value.expires_at <= crate::now()) {
        return Ok(None);
    }
    Ok(Some((audience, ready)))
}
pub(crate) fn can_activate(c: &Connection, store: &Store, cid: &str, sid: &str) -> Result<bool> {
    Ok(pending_snapshot(c, store, cid, sid)?.is_some())
}
pub(crate) fn activate(c: &Connection, store: &Store, cid: &str, sid: &str) -> Result<bool> {
    let Some((audience, ready)) = pending_snapshot(c, store, cid, sid)? else {
        return Ok(false);
    };
    // This is the durable activation observation. Accepted-set changes can
    // make already observed Connections sufficient without another packet.
    let started_at = crate::now();
    let negotiated_capabilities = readiness::capabilities(&ready);
    if ready.len() > 256 || audience.len() > 256 {
        return Err(Error::new(
            "capacity-exceeded",
            "Session activation snapshot exceeds the wire bound",
        ));
    }
    if !crate::conversations::capacity(c, audience.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Session activation journal capacity exhausted",
        ));
    }
    // Active is an observational live phase, with the same media permission
    // requirements as accepted/ready. Existing source readers fence affected
    // authority; no write grant is reentered while that reader is held.
    c.execute("UPDATE sessions SET state='active',updated_at=?2 WHERE id=?1 AND state IN ('accepted','ready')",params![sid,crate::now()]).map_err(|_|Error::storage())?;
    let fact = SessionStarted {
        session: crate::session_control::view(c, cid, sid)?,
        started_at,
        participants_present: audience.iter().map(|m| m.participant_id.clone()).collect(),
        active_connections: ready.iter().map(|value| value.connection.clone()).collect(),
        negotiated_capabilities,
    };
    let raw = serde_json::to_string(&fact).map_err(|_| Error::storage())?;
    c.execute(
        "INSERT INTO session_started_facts VALUES(?1,?2,?3,?4)",
        params![
            sid,
            cid,
            raw,
            serde_json::to_string(&audience).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    let data = serde_json::json!({"source_kind":"session","source_id":sid,"session_id":sid,"session_started":fact});
    for member in audience {
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,resource_revision,data_json,source_json) VALUES(?1,?2,?3,'session_started',?4,?5,?6)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),member.identity_id,cid,fact.session.revision,data.to_string(),serde_json::json!({"kind":"connect-session-lifecycle","correlation_id":sid}).to_string()]).map_err(|_|Error::storage())?;
    }
    for value in ready {
        crate::session_participant_lifecycle::observed_join(
            c,
            store,
            cid,
            sid,
            &value.connection.participant_id,
            value.epoch,
            &value.connection.connid,
            value.observed_at_millis,
        )?;
    }
    Ok(true)
}
pub(crate) fn stored(
    c: &Connection,
    cid: &str,
    sid: &str,
) -> Result<(SessionStarted, Vec<MemberView>)> {
    let row:Option<(String,String)>=c.query_row("SELECT fact_json,audience_json FROM session_started_facts WHERE session_id=?1 AND conversation_id=?2",params![sid,cid],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    let (fact, audience) = row.ok_or_else(Error::forbidden)?;
    Ok((
        serde_json::from_str(&fact).map_err(|_| Error::storage())?,
        serde_json::from_str(&audience).map_err(|_| Error::storage())?,
    ))
}
pub(crate) fn read(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    sid: &str,
    live: bool,
) -> Result<SessionStarted> {
    actor.require("session_get")?;
    crate::audience::require_original(c, actor, cid, ContentKind::Session, sid)?;
    let (fact, audience) = stored(c, cid, sid)?;
    let own = crate::audience::member(c, &actor.identity_id, cid, true)?;
    if !audience.iter().any(|m| {
        m.participant_id == own.participant_id && m.membership_epoch == own.membership_epoch
    }) || audience
        .iter()
        .any(|m| !actor.permits_recipient(&m.identity_id))
        || (live && crate::session_control::view(c, cid, sid)?.state != "active")
    {
        return Err(Error::forbidden());
    }
    for n in &fact.active_connections {
        if !crate::connection_control::private_consultation::visible(c, actor, &n.connid)? {
            return Err(Error::forbidden());
        }
    }
    Ok(fact)
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    sid: &str,
) -> Result<Vec<AuthorityScope>> {
    read(c, actor, cid, sid, true)?;
    let (_, audience) = stored(c, cid, sid)?;
    let mut out = crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Read)?;
    out.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    for m in audience {
        out.push(AuthorityScope::identity(
            &m.identity_id,
            AuthorityMode::Read,
        )?);
    }
    AuthorityScope::merged(&out)
}
impl Store {
    /// Trusted hint template only; actual route delivery authorizes independently.
    pub fn session_started_template(
        &self,
        cid: &str,
        sid: &str,
    ) -> Result<(SessionStarted, Vec<String>)> {
        self.require_runtime_owner()?;
        let c = self.connection()?;
        let (fact, audience) = stored(&c, cid, sid)?;
        Ok((fact, audience.into_iter().map(|m| m.identity_id).collect()))
    }
    pub fn session_started_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<SessionStarted> {
        let PeerCommand::SessionStartedRead {
            conversation_id,
            session_id,
        } = &proof.request.command
        else {
            return Err(Error::forbidden());
        };
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let needed = scopes(&c, &actor, conversation_id, session_id)?;
        if !self.authority_covers(Self::peer_scopes(proof), &needed) {
            return Err(Error::forbidden());
        }
        read(&c, &actor, conversation_id, session_id, true)
    }
}
