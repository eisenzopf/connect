//! Durable Session activation from verified public Connection observations.
use crate::{
    Actor, AuthorityMode, AuthorityScope, NativeConnectionFlowFact, NativeMediaOrigin,
    PeerAuthorityGuard, PeerCommand, Store,
};
use connect_protocol::{
    events::{SessionActiveConnection, SessionStarted},
    membership::{ContentKind, MemberView},
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn start(
    c: &Connection,
    store: &Store,
    origin: &NativeMediaOrigin,
    flow: &NativeConnectionFlowFact,
) -> Result<()> {
    let current = crate::session_control::view(c, origin.conversation_id(), origin.session_id())?;
    if current.state == "active" {
        return Ok(());
    }
    if !matches!(current.state.as_str(), "accepted" | "ready") {
        return Err(Error::forbidden());
    }
    let already: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_started_facts WHERE session_id=?1)",
            [origin.session_id()],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if already {
        return Ok(());
    }
    let mut q=c.prepare("SELECT p.identity_id FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE sp.session_id=?1 AND sp.state='present' ORDER BY p.participant_id").map_err(|_|Error::storage())?;
    let ids = q
        .query_map([origin.session_id()], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    let audience = ids
        .iter()
        .map(|id| crate::audience::member(c, id, origin.conversation_id(), true))
        .collect::<Result<Vec<_>>>()?;
    if !audience.iter().any(|m| {
        m.participant_id == origin.participant_id()
            && m.membership_epoch == origin.membership_epoch()
    }) {
        return Err(Error::forbidden());
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
    c.execute("UPDATE sessions SET state='active',updated_at=?2 WHERE id=?1 AND state IN ('accepted','ready')",params![origin.session_id(),crate::now()]).map_err(|_|Error::storage())?;
    let transport: String = c.query_row("SELECT e.transport FROM connections n JOIN endpoints e ON e.id=n.endpoint_id WHERE n.id=?1",[origin.connection_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
    let fact = SessionStarted {
        session: crate::session_control::view(c, origin.conversation_id(), origin.session_id())?,
        started_at: flow.observed_at_millis / 1000,
        participants_present: audience.iter().map(|m| m.participant_id.clone()).collect(),
        active_connections: vec![SessionActiveConnection {
            connid: origin.connection_id().into(),
            participant_id: origin.participant_id().into(),
            transport,
        }],
        negotiated_capabilities: flow.negotiated_capabilities.clone(),
    };
    let raw = serde_json::to_string(&fact).map_err(|_| Error::storage())?;
    c.execute(
        "INSERT INTO session_started_facts VALUES(?1,?2,?3,?4)",
        params![
            origin.session_id(),
            origin.conversation_id(),
            raw,
            serde_json::to_string(&audience).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    let data = serde_json::json!({"source_kind":"session","source_id":origin.session_id(),"session_id":origin.session_id(),"session_started":fact});
    for member in audience {
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,resource_revision,data_json,source_json) VALUES(?1,?2,?3,'session_started',?4,?5,?6)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),member.identity_id,origin.conversation_id(),fact.session.revision,data.to_string(),serde_json::json!({"kind":"connect-session-lifecycle","correlation_id":origin.session_id()}).to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(())
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
