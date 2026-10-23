//! Server-owned observations resolve original invitation slots. No caller
//! operation ID, ordinary command dispatch, media proof or response IO is used.
use super::*;
use connect_protocol::session_control::{validate_capabilities, SessionView};
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionInvitationDecision {
    Accept {
        by_participant: Option<String>,
        capabilities: Value,
    },
    Reject {
        by_participant: Option<String>,
        reason_code: u64,
        reason: String,
    },
}
impl std::fmt::Debug for SessionInvitationDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationDecision(redacted)")
    }
}
impl SessionInvitationDecision {
    fn validate(&self, source: &SessionInvitationSource) -> Result<()> {
        let by = match self {
            Self::Accept {
                by_participant,
                capabilities,
            } => {
                validate_capabilities(capabilities)?;
                by_participant
            }
            Self::Reject {
                by_participant,
                reason_code,
                reason,
            } => {
                if *reason_code > 65535
                    || reason.len() > 256
                    || reason.chars().any(char::is_control)
                {
                    return Err(Error::new(
                        "invalid-request",
                        "Invalid Session invitation decision",
                    ));
                }
                by_participant
            }
        };
        if by.as_ref().is_some_and(|p| p != &source.participant) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    fn permission(&self) -> &'static str {
        match self {
            Self::Accept { .. } => "session.accept",
            Self::Reject { .. } => "session.reject",
        }
    }
}
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionInvitationDecisionClaim {
    pub source: SessionInvitationSource,
    pub decision: SessionInvitationDecision,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionInvitationDecisionOutcome {
    pub invitation_id: String,
    pub participant_id: String,
    pub membership_epoch: u64,
    pub decision: SessionInvitationDecision,
    pub session: SessionView,
    pub resolved_at: i64,
}
impl std::fmt::Debug for SessionInvitationDecisionOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationDecisionOutcome(redacted)")
    }
}
fn saved(
    c: &Connection,
    claim: &SessionInvitationDecisionClaim,
) -> Result<Option<SessionInvitationDecisionOutcome>> {
    let row:Option<(String,String)>=c.query_row("SELECT decision_json,outcome_json FROM session_invitation_decisions WHERE source_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![claim.source.source,claim.source.participant,claim.source.epoch],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    row.map(|(input, outcome)| {
        let original: SessionInvitationDecision =
            serde_json::from_str(&input).map_err(|_| Error::storage())?;
        if original != claim.decision {
            return Err(Error::new(
                "conflict",
                "Original invitation decision differs",
            ));
        }
        serde_json::from_str(&outcome).map_err(|_| Error::storage())
    })
    .transpose()
}
pub(crate) fn scopes(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    claim: &SessionInvitationDecisionClaim,
) -> Result<Vec<AuthorityScope>> {
    let source = &claim.source;
    if !source.owner.current() || source.runtime != store.runtime_epoch {
        return Err(Error::forbidden());
    }
    claim.decision.validate(source)?;
    actor.require(claim.decision.permission())?;
    let resolved = saved(c, claim)?.is_some();
    let (view, mut scopes) = metadata_state(c, actor, source, resolved)?;
    scopes.push(AuthorityScope::conversation(
        &view.conversation_id,
        AuthorityMode::Write,
    )?);
    AuthorityScope::merged(&scopes)
}
impl Store {
    /// Apply a verified owned exchange observation under fresh exact source,
    /// named action, membership and physical credential/Device/key authority.
    /// Only the dedicated peer claim is accepted; Binding and command proofs
    /// cannot be substituted. The transport must verify its scoped waiter first.
    pub fn apply_session_invitation_decision(
        &self,
        credential: &str,
        claim: SessionInvitationDecisionClaim,
        peer: &crate::PeerAuthorityGuard,
        physical_current: &(dyn Fn() -> bool + Send + Sync),
    ) -> Result<SessionInvitationDecisionOutcome> {
        self.require_runtime_owner()?;
        if !physical_current() {
            return Err(Error::forbidden());
        }
        if !matches!(&peer.request.command,crate::PeerCommand::SessionInvitationDecision(exact) if exact==&claim)
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        if !physical_current() {
            return Err(Error::forbidden());
        }
        self.require_peer_source(&tx, peer, credential)?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let needed = scopes(self, &tx, &actor, &claim)?;
        if !self.authority_covers(Self::peer_scopes(peer), &needed) {
            return Err(Error::forbidden());
        }
        if let Some(original) = saved(&tx, &claim)? {
            if !physical_current() {
                return Err(Error::forbidden());
            }
            self.require_peer_source(&tx, peer, credential)?;
            return Ok(original);
        }
        let (metadata, _) = metadata(&tx, &actor, &claim.source)?;
        let cid = &metadata.conversation_id;
        let sid = &metadata.session_id;
        let current = crate::session_control::view(&tx, cid, sid)?;
        if current.topology_revision >= 9_007_199_254_740_991 {
            return Err(Error::new("conflict", "Session revision exhausted"));
        }
        let action = match &claim.decision {
            SessionInvitationDecision::Accept { capabilities, .. } => {
                let offer = current
                    .capabilities_offer
                    .as_ref()
                    .ok_or_else(Error::forbidden)?;
                if !crate::session_control::supports(offer, capabilities, &current.media) {
                    return Err(Error::new(
                        "unsupported-feature",
                        "Session media profile is unavailable",
                    ));
                }
                tx.execute("UPDATE session_participants SET state='present' WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='invited'",params![sid,claim.source.participant,claim.source.epoch]).map_err(|_|Error::storage())?;
                tx.execute("INSERT INTO session_presence_capabilities(session_id,participant_id,membership_epoch,capabilities_json) VALUES(?1,?2,?3,?4) ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET capabilities_json=excluded.capabilities_json",params![sid,claim.source.participant,claim.source.epoch,crate::provisioning::canonical_json(capabilities)?]).map_err(|_|Error::storage())?;
                tx.execute("UPDATE sessions SET state=CASE WHEN state='invited' THEN 'accepted' ELSE state END WHERE id=?1",[sid]).map_err(|_|Error::storage())?;
                "accept"
            }
            SessionInvitationDecision::Reject {
                reason_code,
                reason,
                ..
            } => {
                tx.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='invited'",params![sid,claim.source.participant,claim.source.epoch]).map_err(|_|Error::storage())?;
                tx.execute("UPDATE connections SET state='retiring',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state NOT IN ('retired','failed','retiring')",params![sid,claim.source.participant,claim.source.epoch]).map_err(|_|Error::storage())?;
                let remains:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 WHERE sp.session_id=?1 AND sp.state IN ('invited','joining','present') AND p.identity_id<>(SELECT owner_id FROM audience_resources WHERE kind='session' AND id=?1))",[sid],|r|r.get(0)).map_err(|_|Error::storage())?;
                if !remains {
                    tx.execute("UPDATE sessions SET state='rejected',ended_reason=?2,terminal_reason_code=?3 WHERE id=?1",params![sid,reason,reason_code]).map_err(|_|Error::storage())?;
                    tx.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND state NOT IN ('left','removed')",[sid]).map_err(|_|Error::storage())?;
                    tx.execute("UPDATE connections SET state='retiring',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE session_id=?1 AND state NOT IN ('retired','failed','retiring')",[sid]).map_err(|_|Error::storage())?;
                    tx.execute("UPDATE session_bindings SET state='ended',updated_at=?2 WHERE session_id=?1 AND state='ready'",params![sid,crate::now()]).map_err(|_|Error::storage())?;
                }
                "reject"
            }
        };
        let resolved_at = crate::now();
        tx.execute(
            "UPDATE sessions SET topology_revision=topology_revision+1,updated_at=?2 WHERE id=?1",
            params![sid, resolved_at],
        )
        .map_err(|_| Error::storage())?;
        let session = crate::session_control::view(&tx, cid, sid)?;
        crate::session_control::emit_invitation_decision(&tx, cid, sid, action, &session, self)?;
        let outcome = SessionInvitationDecisionOutcome {
            invitation_id: claim.source.source.clone(),
            participant_id: claim.source.participant.clone(),
            membership_epoch: claim.source.epoch,
            decision: claim.decision.clone(),
            session,
            resolved_at,
        };
        tx.execute(
            "INSERT INTO session_invitation_decisions VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                claim.source.source,
                claim.source.participant,
                claim.source.epoch,
                crate::provisioning::canonical_json(
                    &serde_json::to_value(&claim.decision).map_err(|_| Error::storage())?
                )?,
                serde_json::to_string(&outcome).map_err(|_| Error::storage())?,
                resolved_at
            ],
        )
        .map_err(|_| Error::storage())?;
        if !physical_current() {
            return Err(Error::forbidden());
        }
        self.require_peer_source(&tx, peer, credential)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(outcome)
    }
}
