//! Current original-caller reads of immutable decisions. Wire correlation and
//! physical selection remain the trusted transport's separate obligation.
use super::decisions::SessionInvitationDecisionOutcome;
use super::*;
fn original(
    c: &Connection,
    source: &SessionInvitationSource,
) -> Result<(String, String, String, String)> {
    c.query_row("SELECT owner_id,operation_id,conversation_id,session_id FROM session_invitation_sources WHERE id=?1",[&source.source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
pub(crate) fn outcome(
    c: &Connection,
    source: &SessionInvitationSource,
) -> Result<Option<SessionInvitationDecisionOutcome>> {
    let raw:Option<String>=c.query_row("SELECT outcome_json FROM session_invitation_decisions WHERE source_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![source.source,source.participant,source.epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(|_| Error::storage()))
        .transpose()
}
pub(crate) fn scopes(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    source: &SessionInvitationSource,
) -> Result<Vec<AuthorityScope>> {
    if !source.owner.current() || source.runtime != store.runtime_epoch {
        return Err(Error::forbidden());
    }
    let (owner, operation, cid, sid) = original(c, source)?;
    if actor.identity_id != owner || outcome(c, source)?.is_none() {
        return Err(Error::forbidden());
    }
    let receipt = crate::operations::receipts::receipt(c, &owner, &operation)?
        .ok_or_else(Error::forbidden)?;
    crate::operations::receipts::check_receipt_authority(c, actor, &receipt, None)?;
    if !matches!(&receipt.operation,connect_protocol::Operation::SessionControl{request} if request.conversation_id==cid && request.session_id==sid && matches!(request.action,connect_protocol::session_control::Action::Invite{..}))
    {
        return Err(Error::forbidden());
    }
    let target = crate::audience::member(c, &source.identity, &cid, true)?;
    if target.participant_id != source.participant || target.membership_epoch != source.epoch {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(c, actor, &cid, ContentKind::Session, &sid)?;
    let mut scopes = vec![AuthorityScope::conversation(&cid, AuthorityMode::Read)?];
    for id in receipt.resources {
        scopes.push(AuthorityScope::identity(&id, AuthorityMode::Read)?);
    }
    AuthorityScope::merged(&scopes)
}
impl Store {
    /// Protected selection by the actual mapped caller, original operation and
    /// decision Participant. This supplies metadata only, never read authority.
    pub fn session_invitation_caller_source(
        &self,
        caller: &str,
        operation: &str,
        participant: &str,
    ) -> Result<SessionInvitationSource> {
        let owner = self.require_runtime_owner()?;
        let c = self.connection()?;
        let (source,identity,epoch)=c.query_row("SELECT s.id,t.identity_id,t.membership_epoch FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id WHERE s.owner_id=?1 AND s.operation_id=?2 AND t.participant_id=?3",params![caller,operation,participant],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        Ok(SessionInvitationSource {
            owner,
            runtime: self.runtime_epoch.clone(),
            source,
            participant: participant.into(),
            identity,
            epoch,
        })
    }
    /// Immutable runtime reconstruction, not a grant to publish the contents.
    pub fn session_invitation_decision_template(
        &self,
        source: &SessionInvitationSource,
    ) -> Result<Option<SessionInvitationDecisionOutcome>> {
        self.require_runtime_owner()?;
        if !source.owner.current() || source.runtime != self.runtime_epoch {
            return Err(Error::forbidden());
        }
        outcome(&*self.connection()?, source)
    }
    pub fn session_invitation_relay_with_peer(
        &self,
        source: &SessionInvitationSource,
        credential: &str,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<(String, SessionInvitationDecisionOutcome)> {
        if !matches!(&peer.request.command,crate::PeerCommand::SessionInvitationRelayRead(exact) if exact==source)
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
        let (_, operation, _, _) = original(&c, source)?;
        Ok((
            operation,
            outcome(&c, source)?.ok_or_else(Error::forbidden)?,
        ))
    }
}
