//! Invitation decisions use the ordinary receipt transaction and its held grant.
use crate::{Actor, Store};
use connect_protocol::{session_decisions::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

impl Store {
    pub fn execute_session_decision_with_peer(
        &self,
        credential: &str,
        request: Request,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        self.execute_runtime_composed(
            credential,
            &connect_protocol::Operation::SessionDecision {
                request: request.clone(),
            },
            &crate::DiscoveryRuntime::default(),
            Some(Self::peer_scopes(peer)),
            Some(peer),
        )?;
        let c = self.connection()?;
        self.require_peer_source(&c, peer, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let record = crate::transitions::current_control_record(&c, &actor, &request.operation_id)?;
        self.require_peer_source(&c, peer, credential)?;
        Ok(record)
    }
}

pub(crate) fn resources(c: &Connection, actor: &Actor, r: &Request) -> Result<Vec<String>> {
    r.validate()?;
    actor.require(r.decision.required_scope())?;
    crate::conversations::authorized_conversation(c, actor, &r.conversation_id)?;
    let member = crate::audience::member(c, &actor.identity_id, &r.conversation_id, true)?;
    if r.by_participant
        .as_ref()
        .is_some_and(|by| by != &member.participant_id)
    {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        actor,
        &r.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        &r.session_id,
    )?;
    let session = crate::sessions::get(c, &r.conversation_id, &r.session_id)?;
    let owner = match r.decision {
        Decision::Reject => &session.participant_id,
        Decision::Cancel => &session.initiator_id,
    };
    if owner != &actor.identity_id {
        return Err(Error::forbidden());
    }
    crate::audience::source_members(
        c,
        &r.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        &r.session_id,
    )
}

pub(crate) fn apply(
    c: &Connection,
    actor: &Actor,
    r: &Request,
    recipients: &[String],
    store: &Store,
) -> Result<DecisionOutcome> {
    resources(c, actor, r)?;
    let (state, revision): (String, u64) = c
        .query_row(
            "SELECT state,topology_revision FROM sessions WHERE id=?1 AND conversation_id=?2",
            params![r.session_id, r.conversation_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    // Exact replay is handled by the ordinary receipt engine before this point.
    // A new logical decision cannot rewrite a previously decided invitation.
    if state != "invited"
        || r.expected_revision.is_some_and(|v| v != revision)
        || revision >= 9_007_199_254_740_991
    {
        return Err(Error::new(
            "conflict",
            "Invitation state or revision conflicts",
        ));
    }
    c.execute(
        "UPDATE sessions SET topology_revision=topology_revision+1,terminal_reason_code=?2 WHERE id=?1",
        params![r.session_id,r.reason_code],
    )
    .map_err(|_| Error::storage())?;
    let transition = match r.decision {
        Decision::Reject => crate::sessions::Transition::Reject(Some(r.reason.clone())),
        Decision::Cancel => crate::sessions::Transition::Cancel(Some(r.reason.clone())),
    };
    let session = crate::sessions::transition(
        c,
        actor,
        &r.conversation_id,
        &r.session_id,
        transition,
        recipients,
        store,
    )?;
    Ok(DecisionOutcome {
        operation_id: r.operation_id.clone(),
        conversation_id: r.conversation_id.clone(),
        session_id: r.session_id.clone(),
        state: session.state,
        revision: revision + 1,
        reason_code: r.reason_code,
        reason: r.reason.clone(),
    })
}
