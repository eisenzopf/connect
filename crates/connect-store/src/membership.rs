//! Serialized membership mutations, invitations and explicit sharing receipts.
pub(crate) mod control;
pub(crate) mod mutations;
mod queries;
use crate::{audience, identity::actor_from_connection, now, Actor, Store};
use connect_protocol::{membership::*, Error, Result};
use rusqlite::{params, Connection, OptionalExtension};

fn conflict() -> Error {
    Error::new(
        "conflict",
        "Membership intent or revision conflicts with current state",
    )
}
fn tenant(c: &Connection, actor: &Actor, cid: &str) -> Result<String> {
    c.query_row("SELECT v.tenant_id FROM conversations v JOIN identities i ON i.tenant_id=v.tenant_id WHERE v.id=?1 AND i.id=?2 AND i.revoked=0",params![cid,actor.identity_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
fn revision(c: &Connection, cid: &str) -> Result<u64> {
    c.query_row(
        "SELECT revision FROM conversations WHERE id=?1",
        [cid],
        |r| r.get(0),
    )
    .map_err(|_| Error::storage())
}
fn manage(c: &Connection, actor: &Actor, cid: &str) -> Result<()> {
    tenant(c, actor, cid)?;
    if !actor.permits("admin") {
        audience::member(c, &actor.identity_id, cid, true)?;
    }
    Ok(())
}
fn invitation(c: &Connection, id: &str, cid: &str) -> Result<InvitationView> {
    c.query_row("SELECT x.id,x.conversation_id,x.identity_id,x.expires_at,x.state,v.revision FROM membership_invitations x JOIN conversations v ON v.id=x.conversation_id WHERE x.id=?1 AND x.conversation_id=?2",params![id,cid],|r|Ok(InvitationView{invitation_id:r.get(0)?,conversation_id:r.get(1)?,identity_id:r.get(2)?,expires_at:r.get(3)?,state:r.get(4)?,conversation_revision:r.get(5)?})).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
pub(crate) fn authorize(c: &Connection, actor: &Actor, request: &MembershipRequest) -> Result<()> {
    actor.require(request.required_scope())?;
    tenant(c, actor, &request.conversation_id)?;
    match &request.change {
        MembershipAction::Join { invitation_id } | MembershipAction::Decline { invitation_id } => {
            let invite = invitation(c, invitation_id, &request.conversation_id)?;
            if invite.identity_id != actor.identity_id {
                return Err(Error::forbidden());
            }
        }
        MembershipAction::Leave {} => {
            audience::member(
                c,
                &actor.identity_id,
                &request.conversation_id,
                !control::cleanup_receipt(c, actor, request)?,
            )?;
        }
        MembershipAction::ShareHistory {
            resources,
            participants,
            ..
        } => {
            audience::member(c, &actor.identity_id, &request.conversation_id, true)?;
            for source in resources {
                actor.require(audience::read_scope(source.kind))?;
                audience::require(c, actor, &request.conversation_id, source.kind, &source.id)?;
            }
            for id in participants {
                let identity:String=c.query_row("SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2 AND active=1",params![id,request.conversation_id],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
                audience::member(c, &identity, &request.conversation_id, true)?;
                if !actor.permits_recipient(&identity) {
                    return Err(Error::forbidden());
                }
            }
        }
        _ if control::cleanup_receipt(c, actor, request)? => (),
        _ => manage(c, actor, &request.conversation_id)?,
    }
    Ok(())
}
pub(crate) fn authorize_replay(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
    value: MembershipOutcome,
) -> Result<MembershipOutcome> {
    match value {
        MembershipOutcome::Participant {
            participant,
            receipt,
            ..
        } => {
            let current =
                audience::member(c, &participant.identity_id, &request.conversation_id, false)?;
            if current.membership_epoch != participant.membership_epoch
                || (!current.active
                    && !matches!(
                        request.change,
                        MembershipAction::Leave {} | MembershipAction::Remove { .. }
                    ))
                || !actor.permits_recipient(&participant.identity_id)
            {
                return Err(Error::forbidden());
            }
            Ok(MembershipOutcome::Participant {
                participant: current,
                revision: revision(c, &request.conversation_id)?,
                receipt,
            })
        }
        MembershipOutcome::Invitation {
            invitation: old,
            receipt,
        } => {
            let current = invitation(c, &old.invitation_id, &request.conversation_id)?;
            if !actor.permits_recipient(&current.identity_id) {
                return Err(Error::forbidden());
            }
            Ok(MembershipOutcome::Invitation {
                invitation: current,
                receipt,
            })
        }
        MembershipOutcome::Sharing {
            sharing_id,
            conversation_id,
            receipt,
            ..
        } => {
            let active: bool = c
                .query_row(
                    "SELECT active FROM history_shares WHERE id=?1 AND conversation_id=?2",
                    params![sharing_id, conversation_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            Ok(MembershipOutcome::Sharing {
                sharing_id,
                revision: revision(c, &conversation_id)?,
                conversation_id,
                active,
                receipt,
            })
        }
    }
}

impl Store {
    pub fn execute_membership(
        &self,
        credential: &str,
        request: MembershipRequest,
    ) -> Result<MembershipOutcome> {
        match self.execute(
            credential,
            &connect_protocol::Operation::Membership { request },
        )? {
            connect_protocol::Outcome::Membership(value) => Ok(value),
            _ => Err(Error::storage()),
        }
    }
    pub fn execute_membership_with_authority(
        &self,
        credential: &str,
        request: MembershipRequest,
        held: &std::sync::Arc<crate::AuthorityGuardSet>,
    ) -> Result<MembershipOutcome> {
        match self.execute_authority_mutation(
            credential,
            &connect_protocol::Operation::Membership { request },
            held,
        )? {
            connect_protocol::Outcome::Membership(value) => Ok(value),
            _ => Err(Error::storage()),
        }
    }
}
