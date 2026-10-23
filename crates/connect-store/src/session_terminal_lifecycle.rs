//! Original terminal metadata with captured epochs and fresh delivery authority.
use crate::{Actor, AuthorityMode, AuthorityScope, PeerAuthorityGuard, PeerCommand, Store};
use connect_protocol::{events::SessionEnded, membership::ContentKind, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

pub(crate) fn stored(
    c: &Connection,
    cid: &str,
    sid: &str,
) -> Result<(SessionEnded, Vec<(String, String, u64)>)> {
    let raw: Option<String> = c.query_row("SELECT fact_json FROM session_terminal_facts WHERE conversation_id=?1 AND session_id=?2", params![cid,sid], |r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let fact =
        serde_json::from_str(&raw.ok_or_else(Error::forbidden)?).map_err(|_| Error::storage())?;
    let mut q=c.prepare("SELECT identity_id,participant_id,membership_epoch FROM session_terminal_audiences WHERE session_id=?1 ORDER BY participant_id,membership_epoch").map_err(|_|Error::storage())?;
    let members = q
        .query_map([sid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    Ok((fact, members))
}
pub(crate) fn read(c: &Connection, actor: &Actor, cid: &str, sid: &str) -> Result<SessionEnded> {
    actor.require("session_get")?;
    crate::audience::require_original(c, actor, cid, ContentKind::Session, sid)?;
    let (fact, members) = stored(c, cid, sid)?;
    let own = crate::audience::member(c, &actor.identity_id, cid, true)?;
    if !members
        .iter()
        .any(|(_, pid, epoch)| pid == &own.participant_id && *epoch == own.membership_epoch)
        || members
            .iter()
            .any(|(id, _, _)| !actor.permits_recipient(id))
        || crate::session_control::view(c, cid, sid)?.state != "ended"
    {
        return Err(Error::forbidden());
    }
    Ok(fact)
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    sid: &str,
) -> Result<Vec<AuthorityScope>> {
    read(c, actor, cid, sid)?;
    let mut scopes = crate::authority_admission::conversation_scopes(c, cid, AuthorityMode::Read)?;
    scopes.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    for (identity, _, _) in stored(c, cid, sid)?.1 {
        scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Read)?);
    }
    AuthorityScope::merged(&scopes)
}
impl Store {
    pub fn session_ended_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<SessionEnded> {
        let PeerCommand::SessionEndedRead {
            conversation_id,
            session_id,
        } = &proof.request.command
        else {
            return Err(Error::forbidden());
        };
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        if !self.authority_covers(
            Self::peer_scopes(proof),
            &scopes(&c, &actor, conversation_id, session_id)?,
        ) {
            return Err(Error::forbidden());
        }
        read(&c, &actor, conversation_id, session_id)
    }
}
