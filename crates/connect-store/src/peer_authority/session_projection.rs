//! Current disposable Core inputs, resolved under the exact Session command fence.
use super::*;
use connect_protocol::{membership::MemberView, session_control::SessionView};

/// Checked facts, not a reusable grant. The caller must keep the originating
/// PeerAuthorityGuard alive through the synchronous Core projection/attachment.
#[derive(Clone)]
pub struct PeerSessionProjection {
    pub session: SessionView,
    pub actor: MemberView,
    /// The durable Conversation revision governs this membership view; Core
    /// must never mutate/persist a competing membership registry.
    pub conversation_revision: u64,
    pub present: bool,
}
impl Store {
    pub fn project_session_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<PeerSessionProjection> {
        let PeerCommand::SessionControl(request) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        {
            let c = self.connection()?;
            self.require_peer_source(&c, proof, credential)?;
        }
        let projection = self.session_projection_guarded(credential, request, &proof._scopes)?;
        {
            let c = self.connection()?;
            self.require_peer_source(&c, proof, credential)?;
        }
        Ok(projection)
    }
    pub(crate) fn session_projection_guarded(
        &self,
        credential: &str,
        request: &connect_protocol::session_control::Request,
        authority: &Arc<AuthorityGuardSet>,
    ) -> Result<PeerSessionProjection> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        let record =
            crate::transitions::current_control_record(&tx, &actor, &request.operation_id)?;
        if record.state != connect_protocol::transitions::OperationState::Completed {
            return Err(Error::forbidden());
        }
        let saved =
            crate::operations::receipts::receipt(&tx, &actor.identity_id, &request.operation_id)?
                .ok_or_else(Error::forbidden)?;
        if !matches!(&saved.operation, connect_protocol::Operation::SessionControl {request: original} if original == request)
        {
            return Err(Error::forbidden());
        }
        crate::operations::receipts::check_receipt_authority(&tx, &actor, &saved, None)?;
        let scopes =
            crate::authority_admission::operation_scopes(&tx, &actor, &saved.operation, self)?;
        if !self.authority_covers(authority, &scopes) {
            return Err(Error::forbidden());
        }
        crate::conversations::authorized_conversation(&tx, &actor, &request.conversation_id)?;
        crate::audience::require(
            &tx,
            &actor,
            &request.conversation_id,
            connect_protocol::membership::ContentKind::Session,
            &request.session_id,
        )?;
        let member =
            crate::audience::member(&tx, &actor.identity_id, &request.conversation_id, true)?;
        let session =
            crate::session_control::view(&tx, &request.conversation_id, &request.session_id)?;
        let conversation_revision = tx
            .query_row(
                "SELECT revision FROM conversations WHERE id=?1",
                [&request.conversation_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        let present = tx.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')", params![request.session_id,member.participant_id,member.membership_epoch], |r| r.get(0)).map_err(|_| Error::storage())?;
        crate::identity::actor_from_connection(&tx, credential)?;
        Ok(PeerSessionProjection {
            session,
            actor: member,
            conversation_revision,
            present,
        })
    }
}
