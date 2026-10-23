//! Actual public readiness enters an appearance; last terminal route or explicit
//! departure leaves it. Pending/replacement Connections preserve the appearance.
use crate::{
    Actor, AuthorityMode, AuthorityScope, NativeConnectionFlowFact, NativeMediaOrigin,
    PeerAuthorityGuard, PeerCommand, Store,
};
use connect_protocol::{
    events::{
        JournalData, SessionJoinedParticipant, SessionParticipantJoined, SessionParticipantLeft,
    },
    membership::{ContentKind, MemberView},
    session_control::SessionView,
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn fields(data: &JournalData) -> Result<(&str, &SessionView, &str, u64, &str)> {
    match data {
        JournalData::SessionParticipantJoined(v) => Ok((
            &v.fact_id,
            &v.session,
            &v.participant.participant_id,
            v.membership_epoch,
            "session_participant_joined",
        )),
        JournalData::SessionParticipantLeft(v) => Ok((
            &v.fact_id,
            &v.session,
            &v.participant_id,
            v.membership_epoch,
            "session_participant_left",
        )),
        _ => Err(Error::storage()),
    }
}
fn audience(c: &Connection, cid: &str, sid: &str, subject: &MemberView) -> Result<Vec<MemberView>> {
    let mut q=c.prepare("SELECT p.identity_id FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE sp.session_id=?1 AND sp.state IN ('present','left') ORDER BY p.participant_id").map_err(|_|Error::storage())?;
    let ids = q
        .query_map([sid], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    let mut members = ids
        .iter()
        .map(|id| crate::audience::member(c, id, cid, true))
        .collect::<Result<Vec<_>>>()?;
    if !members.iter().any(|m| {
        m.participant_id == subject.participant_id && m.membership_epoch == subject.membership_epoch
    }) {
        members.push(subject.clone());
    }
    Ok(members)
}
fn append(c: &Connection, store: &Store, data: JournalData, subject: &MemberView) -> Result<()> {
    let (id, session, pid, epoch, kind) = fields(&data)?;
    let members = audience(c, &session.conversation_id, &session.id, subject)?;
    if !crate::conversations::capacity(c, members.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Participant lifecycle journal capacity exhausted",
        ));
    }
    let count: u64 = c
        .query_row("SELECT COUNT(*) FROM session_participant_facts", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    if count >= store.limits().operation_receipts {
        return Err(Error::new(
            "capacity-exceeded",
            "Participant lifecycle fact capacity exhausted",
        ));
    }
    c.execute(
        "INSERT INTO session_participant_facts VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            id,
            session.conversation_id,
            session.id,
            pid,
            epoch,
            kind,
            serde_json::to_string(&data).map_err(|_| Error::storage())?,
            serde_json::to_string(&members).map_err(|_| Error::storage())?
        ],
    )
    .map_err(|_| Error::storage())?;
    c.execute("INSERT INTO session_connected_presence VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET state=excluded.state,fact_id=excluded.fact_id",params![session.id,pid,epoch,if kind=="session_participant_joined"{"joined"}else{"left"},id]).map_err(|_|Error::storage())?;
    let correlation: Option<(String, String)> = c
        .query_row(
            "SELECT actor,operation_id FROM journal_context WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    for member in members {
        let mut body = serde_json::json!({"source_kind":"session","source_id":session.id,"session_id":session.id,"participant_id":pid,"participant_fact_id":id,"participant_fact":data});
        if let Some((owner, operation)) = &correlation {
            if owner == &member.identity_id {
                body["operation_id"] = serde_json::json!(operation);
            }
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,resource_revision,data_json,source_json,recipient_participant_id,recipient_epoch) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),member.identity_id,session.conversation_id,kind,session.revision,body.to_string(),serde_json::json!({"kind":"connect-session-lifecycle","correlation_id":id}).to_string(),member.participant_id,member.membership_epoch]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
pub(crate) fn joined(
    c: &Connection,
    store: &Store,
    origin: &NativeMediaOrigin,
    flow: &NativeConnectionFlowFact,
) -> Result<()> {
    let joined:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_connected_presence WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='joined')",params![origin.session_id(),origin.participant_id(),origin.membership_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if joined {
        return Ok(());
    }
    let identity: String = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1",
            [origin.participant_id()],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let subject = crate::audience::member(c, &identity, origin.conversation_id(), true)?;
    let (kind,role,name):(String,String,String)=c.query_row("SELECT p.kind,sp.role,COALESCE(i.display_name,i.alias) FROM participants p JOIN identities i ON i.id=p.identity_id JOIN session_participants sp ON sp.participant_id=p.participant_id AND sp.membership_epoch=p.membership_epoch WHERE sp.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND sp.state='present' AND p.active=1",params![origin.session_id(),origin.participant_id(),origin.membership_epoch()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    c.execute(
        "UPDATE sessions SET state_revision=state_revision+1,updated_at=?2 WHERE id=?1",
        params![origin.session_id(), crate::now()],
    )
    .map_err(|_| Error::storage())?;
    let data = JournalData::SessionParticipantJoined(SessionParticipantJoined {
        fact_id: format!("spf_{}", uuid::Uuid::new_v4().simple()),
        session: crate::session_control::view(c, origin.conversation_id(), origin.session_id())?,
        membership_epoch: origin.membership_epoch(),
        participant: SessionJoinedParticipant {
            participant_id: origin.participant_id().into(),
            identity_id: subject.identity_id.clone(),
            kind: serde_json::from_value(serde_json::json!(kind)).map_err(|_| Error::storage())?,
            role,
            display_name: name,
            joined_at: flow.observed_at_millis / 1000,
        },
        via_connection: origin.connection_id().into(),
    });
    append(c, store, data, &subject)
}
/// Runs inside the mutation/observed-retirement transaction, never from silence.
pub(crate) fn reconcile(
    c: &Connection,
    store: &Store,
    cid: &str,
    sid: &str,
    reason: Option<(&str, &str)>,
) -> Result<()> {
    crate::session_reconnect::observe_loss(c, store, sid, chrono::Utc::now().timestamp_millis())?;
    let mut q=c.prepare("SELECT participant_id,membership_epoch,fact_id FROM session_connected_presence WHERE session_id=?1 AND state='joined' ORDER BY participant_id").map_err(|_|Error::storage())?;
    let rows = q
        .query_map([sid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    for (pid, epoch, old) in rows {
        let admitted:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN sessions s ON s.id=sp.session_id WHERE sp.session_id=?1 AND sp.participant_id=?2 AND sp.membership_epoch=?3 AND sp.state='present' AND s.state IN ('accepted','ready','active'))",params![sid,pid,epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        let live:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state NOT IN ('retired','failed','interrupted'))",params![sid,pid,epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if admitted && live {
            continue;
        }
        let (original, members) = stored(c, cid, sid, &old)?;
        let subject = members
            .iter()
            .find(|m| m.participant_id == pid && m.membership_epoch == epoch)
            .ok_or_else(Error::storage)?;
        let why = reason
            .filter(|(target, _)| *target == pid || *target == "*")
            .map(|(_, r)| r)
            .unwrap_or("all-connections-ended");
        let _ = original;
        c.execute(
            "UPDATE sessions SET state_revision=state_revision+1,updated_at=?2 WHERE id=?1",
            params![sid, crate::now()],
        )
        .map_err(|_| Error::storage())?;
        append(
            c,
            store,
            JournalData::SessionParticipantLeft(SessionParticipantLeft {
                fact_id: format!("spf_{}", uuid::Uuid::new_v4().simple()),
                session: crate::session_control::view(c, cid, sid)?,
                participant_id: pid,
                membership_epoch: epoch,
                left_at: crate::now(),
                reason: why.into(),
            }),
            subject,
        )?;
    }
    Ok(())
}
pub(crate) fn participant_departed(
    c: &Connection,
    store: &Store,
    member: &MemberView,
    reason: &str,
) -> Result<()> {
    let mut sessions=c.prepare("SELECT session_id FROM session_connected_presence WHERE participant_id=?1 AND membership_epoch=?2 AND state='joined'").map_err(|_|Error::storage())?;
    let ids = sessions
        .query_map(
            params![member.participant_id, member.membership_epoch],
            |r| r.get::<_, String>(0),
        )
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    for sid in ids {
        reconcile(
            c,
            store,
            &member.conversation_id,
            &sid,
            Some((&member.participant_id, reason)),
        )?;
    }
    Ok(())
}
fn stored(
    c: &Connection,
    cid: &str,
    sid: &str,
    id: &str,
) -> Result<(JournalData, Vec<MemberView>)> {
    let row:Option<(String,String)>=c.query_row("SELECT fact_json,audience_json FROM session_participant_facts WHERE fact_id=?1 AND conversation_id=?2 AND session_id=?3",params![id,cid,sid],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    let (raw, audience) = row.ok_or_else(Error::forbidden)?;
    Ok((
        serde_json::from_str(&raw).map_err(|_| Error::storage())?,
        serde_json::from_str(&audience).map_err(|_| Error::storage())?,
    ))
}
pub(crate) fn read(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    sid: &str,
    id: &str,
    live: bool,
) -> Result<JournalData> {
    actor.require("session_get")?;
    crate::audience::require_original(c, actor, cid, ContentKind::Session, sid)?;
    let (data, members) = stored(c, cid, sid, id)?;
    let (_, _, pid, epoch, kind) = fields(&data)?;
    let own = crate::audience::member(c, &actor.identity_id, cid, true)?;
    if !members.iter().any(|m| {
        m.participant_id == own.participant_id && m.membership_epoch == own.membership_epoch
    }) || members
        .iter()
        .any(|m| !actor.permits_recipient(&m.identity_id))
    {
        return Err(Error::forbidden());
    }
    if let JournalData::SessionParticipantJoined(v) = &data {
        if !crate::connection_control::private_consultation::visible(c, actor, &v.via_connection)? {
            return Err(Error::forbidden());
        }
    }
    if live {
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_connected_presence WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND fact_id=?4 AND state=?5)",params![sid,pid,epoch,id,if kind=="session_participant_joined"{"joined"}else{"left"}],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current {
            return Err(Error::forbidden());
        }
        if kind == "session_participant_joined"
            && crate::session_control::view(c, cid, sid)?.state != "active"
        {
            return Err(Error::forbidden());
        }
    }
    Ok(data)
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    sid: &str,
    id: &str,
) -> Result<Vec<AuthorityScope>> {
    read(c, actor, cid, sid, id, true)?;
    let (_, members) = stored(c, cid, sid, id)?;
    let mut scopes = crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Read)?;
    for member in members {
        scopes.push(AuthorityScope::identity(
            &member.identity_id,
            AuthorityMode::Read,
        )?);
    }
    AuthorityScope::merged(&scopes)
}
impl Store {
    pub fn session_participant_fact_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<JournalData> {
        let PeerCommand::SessionParticipantFactRead {
            conversation_id,
            session_id,
            fact_id,
        } = &proof.request.command
        else {
            return Err(Error::forbidden());
        };
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        if !self.authority_covers(
            Self::peer_scopes(proof),
            &scopes(&c, &actor, conversation_id, session_id, fact_id)?,
        ) {
            return Err(Error::forbidden());
        }
        read(&c, &actor, conversation_id, session_id, fact_id, true)
    }
    pub fn session_lifecycle_hint_watermark(&self) -> Result<u64> {
        self.require_runtime_owner()?;
        self.connection()?
            .query_row(
                "SELECT COALESCE(MAX(sequence),0) FROM session_lifecycle_hints",
                [],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())
    }
    /// Trusted metadata templates, not delivery grants. Bounded source order.
    pub fn session_lifecycle_hint_page(
        &self,
        after: u64,
    ) -> Result<Vec<(u64, String, JournalData, Vec<String>)>> {
        self.require_runtime_owner()?;
        // Template polling is not an authorization boundary. Use a separate
        // bounded committed read snapshot so idle background work cannot make
        // an actual media/control try_connection fail on our foreground mutex.
        let mut snapshot = Connection::open_with_flags(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| Error::storage())?;
        snapshot
            .busy_timeout(std::time::Duration::from_millis(50))
            .map_err(|_| Error::storage())?;
        let transaction = snapshot
            .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
            .map_err(|_| Error::storage())?;
        let c = &transaction;
        let mut q=c.prepare("SELECT sequence,kind,fact_id,conversation_id,session_id FROM session_lifecycle_hints WHERE sequence>?1 ORDER BY sequence LIMIT 16").map_err(|_|Error::storage())?;
        let rows = q
            .query_map([after], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage())?;
        rows.into_iter()
            .map(|(seq, kind, id, cid, sid)| {
                if kind == "record_ready" {
                    let (fact, members) = crate::records::readiness::stored(&c, &cid, &sid, &id)?;
                    Ok((
                        seq,
                        cid,
                        JournalData::Record(fact),
                        members.into_iter().map(|(id, _, _)| id).collect(),
                    ))
                } else if kind == "session_ended" {
                    let (fact, members) =
                        crate::session_terminal_lifecycle::stored(&c, &cid, &sid)?;
                    Ok((
                        seq,
                        cid,
                        JournalData::SessionEnded(fact),
                        members.into_iter().map(|(id, _, _)| id).collect(),
                    ))
                } else if kind == "session_started" {
                    let (fact, members) = crate::session_lifecycle::stored(&c, &cid, &sid)?;
                    let targets = members.into_iter().map(|m| m.identity_id).collect();
                    Ok((seq, cid, JournalData::SessionStarted(fact), targets))
                } else {
                    let (data, members) = stored(&c, &cid, &sid, &id)?;
                    Ok((
                        seq,
                        cid,
                        data,
                        members.into_iter().map(|m| m.identity_id).collect(),
                    ))
                }
            })
            .collect()
    }
}
