//! Immutable cancellation observations for the exact pending target, with fresh
//! current recipient reads only during actual notification IO.
use super::*;
#[derive(Clone, PartialEq, Eq)]
pub struct SessionInvitationCancellation {
    pub conversation_id: String,
    pub session_id: String,
    pub by_participant_id: String,
    pub by_membership_epoch: u64,
    pub reason_code: u64,
    pub reason: String,
    pub cancelled_at: i64,
}
impl std::fmt::Debug for SessionInvitationCancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationCancellation(redacted)")
    }
}
pub(crate) fn freeze(c: &Connection, r: &Request, by: &MemberView) -> Result<()> {
    let connect_protocol::session_control::Action::Cancel {
        reason_code,
        reason,
    } = &r.action
    else {
        return Err(Error::storage());
    };
    c.execute("INSERT INTO session_invitation_cancellations(source_id,participant_id,membership_epoch,by_participant_id,by_membership_epoch,reason_code,reason,cancelled_at) SELECT slot.source_id,slot.participant_id,slot.membership_epoch,?2,?3,?4,?5,?6 FROM session_invitation_slots slot JOIN session_participants presence ON presence.session_id=slot.session_id AND presence.participant_id=slot.participant_id AND presence.membership_epoch=slot.membership_epoch AND presence.state='invited' WHERE slot.session_id=?1",params![r.session_id,by.participant_id,by.membership_epoch,reason_code,reason,crate::now()]).map_err(|_|Error::storage())?;
    Ok(())
}
fn load(
    c: &Connection,
    source: &SessionInvitationSource,
) -> Result<Option<SessionInvitationCancellation>> {
    c.query_row("SELECT s.conversation_id,s.session_id,c.by_participant_id,c.by_membership_epoch,c.reason_code,c.reason,c.cancelled_at FROM session_invitation_cancellations c JOIN session_invitation_sources s ON s.id=c.source_id WHERE c.source_id=?1 AND c.participant_id=?2 AND c.membership_epoch=?3",params![source.source,source.participant,source.epoch],|r|Ok(SessionInvitationCancellation{conversation_id:r.get(0)?,session_id:r.get(1)?,by_participant_id:r.get(2)?,by_membership_epoch:r.get(3)?,reason_code:r.get(4)?,reason:r.get(5)?,cancelled_at:r.get(6)?})).optional().map_err(|_|Error::storage())
}
pub(crate) fn scopes(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    source: &SessionInvitationSource,
) -> Result<Vec<AuthorityScope>> {
    if !source.owner.current()
        || source.runtime != store.runtime_epoch
        || load(c, source)?.is_none()
    {
        return Err(Error::forbidden());
    }
    // Resolved metadata still requires both original and current source access,
    // current inviter/target epochs and the recipient's current ceiling. Only
    // the pending-state predicate is replaced by the immutable cancellation.
    metadata_state(c, actor, source, true).map(|(_, scopes)| scopes)
}
impl Store {
    /// Protected runtime metadata only; no recipient authorization is retained.
    pub fn session_invitation_cancellation_template(
        &self,
        source: &SessionInvitationSource,
    ) -> Result<Option<SessionInvitationCancellation>> {
        self.require_runtime_owner()?;
        if !source.owner.current() || source.runtime != self.runtime_epoch {
            return Err(Error::forbidden());
        }
        load(&*self.connection()?, source)
    }
    pub fn session_invitation_cancellation_with_peer(
        &self,
        source: &SessionInvitationSource,
        credential: &str,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<SessionInvitationCancellation> {
        if !matches!(&peer.request.command,crate::PeerCommand::SessionInvitationCancellationRead(exact) if exact==source)
        {
            return Err(Error::forbidden());
        }
        let c = self.connection()?;
        self.require_peer_source(&c, peer, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let needed = scopes(self, &c, &actor, source)?;
        if !self.authority_covers(Self::peer_scopes(peer), &needed) {
            return Err(Error::forbidden());
        }
        load(&c, source)?.ok_or_else(Error::forbidden)
    }
}
