//! Original outcome readiness, captured Session epochs and current Record policy.
use super::*;
use crate::{Actor, PeerAuthorityGuard, PeerCommand};
use connect_protocol::events::RecordAvailability;

pub(super) fn capture(c: &Connection, scope: &RecordScope, uuid: &str) -> Result<()> {
    c.execute("INSERT INTO record_ready_facts(uuid,conversation_id,session_id,revision) VALUES(?1,?2,?3,1)",params![uuid,scope.conversation_id,scope.session_id]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn stored(
    c: &Connection,
    cid: &str,
    sid: &str,
    uuid: &str,
) -> Result<(RecordAvailability, Vec<(String, String, u64)>)> {
    let revision:Option<u64>=c.query_row("SELECT revision FROM record_ready_facts WHERE uuid=?1 AND conversation_id=?2 AND session_id=?3",params![uuid,cid,sid],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let revision = revision.ok_or_else(Error::forbidden)?;
    let mut q=c.prepare("SELECT identity_id,participant_id,membership_epoch FROM record_ready_audiences WHERE uuid=?1 ORDER BY participant_id,membership_epoch").map_err(|_|Error::storage())?;
    let audience = q
        .query_map([uuid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|_| Error::storage())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::storage())?;
    Ok((
        RecordAvailability::Ready {
            session_id: sid.into(),
            uuid: uuid.into(),
            revision,
        },
        audience,
    ))
}
/// The document metadata cutoff bounds this hint, including actual queued IO.
/// Outcome policies use whole seconds; flooring a fractional custom cutoff can
/// only refuse an advisory hint early, never extend its authority.
pub(crate) fn expiry(c: &Connection, intent: &RecordFetchIntent) -> Result<Option<i64>> {
    let uuid = intent.uuid.as_deref().ok_or_else(Error::forbidden)?;
    let raw:Option<Vec<u8>>=c.query_row("SELECT manifest FROM session_records WHERE uuid=?1 AND conversation_id=?2 AND session_id=?3 AND deleted=0",params![uuid,intent.conversation_id,intent.session_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?.flatten();
    let value: serde_json::Value =
        serde_json::from_slice(&raw.ok_or_else(Error::forbidden)?).map_err(|_| Error::storage())?;
    let documents = value["sources"]
        .as_array()
        .ok_or_else(Error::storage)?
        .iter()
        .filter(|s| s["id"] == "document")
        .collect::<Vec<_>>();
    if documents.len() != 1 || documents[0]["state"] == "metadata-unavailable" {
        return Err(Error::forbidden());
    }
    let retention = &documents[0]["metadata_retention"];
    match retention["kind"].as_str() {
        Some("until-deleted") => Ok(None),
        Some("until") => {
            let until = retention["until"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .ok_or_else(Error::storage)?
                .timestamp();
            if until <= crate::now() {
                return Err(Error::forbidden());
            }
            Ok(Some(until))
        }
        _ => Err(Error::storage()),
    }
}
fn read(c: &Connection, actor: &Actor, intent: &RecordFetchIntent) -> Result<RecordAvailability> {
    let uuid = intent
        .uuid
        .as_deref()
        .filter(|_| intent.url.is_none())
        .ok_or_else(Error::forbidden)?;
    let (fact, audience) = stored(c, &intent.conversation_id, &intent.session_id, uuid)?;
    let own = crate::audience::member(c, &actor.identity_id, &intent.conversation_id, true)?;
    if !audience.iter().any(|(id, pid, epoch)| {
        id == &actor.identity_id && pid == &own.participant_id && *epoch == own.membership_epoch
    }) {
        return Err(Error::forbidden());
    }
    expiry(c, intent)?;
    let policy = super::read::policy(c, actor, intent)?.ok_or_else(Error::forbidden)?;
    let RecordAvailability::Ready { revision, .. } = &fact else {
        return Err(Error::storage());
    };
    if policy.binding.revision != *revision || !policy.sources.contains("document") {
        return Err(Error::forbidden());
    }
    Ok(fact)
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &Actor,
    intent: &RecordFetchIntent,
) -> Result<Vec<AuthorityScope>> {
    read(c, actor, intent)?;
    super::read::scopes(c, actor, intent)
}
impl Store {
    pub fn record_ready_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<RecordAvailability> {
        let PeerCommand::RecordReadyRead(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        if !self.authority_covers(Self::peer_scopes(proof), &scopes(&c, &actor, intent)?) {
            return Err(Error::forbidden());
        }
        read(&c, &actor, intent)
    }
}
