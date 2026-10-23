//! Shared control admission and local effects; the ordinary engine owns commit.
use super::*;
use crate::{AuthorityMode, AuthorityScope};
#[derive(Clone)]
pub(crate) struct Origin {
    pub authority: &'static str,
    pub participant: Option<(String, u64)>,
}
pub(crate) fn cleanup_receipt(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
) -> Result<bool> {
    let own = match audience::member(c, &actor.identity_id, &request.conversation_id, false) {
        Ok(member) => Some(member),
        Err(error) if error.code == "forbidden" => None,
        Err(error) => return Err(error),
    };
    let Some(own) = own else {
        return Ok(false);
    };
    if !matches!(&request.change, MembershipAction::Leave {})
        && !matches!(&request.change,MembershipAction::Remove{participant_id,..} if participant_id==&own.participant_id)
    {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM operations o JOIN operation_control_receipts x ON x.operation_record_id=o.record_id WHERE o.owner_id=?1 AND o.operation_id=?2 AND o.conversation_id=?3 AND o.state='completed' AND json_extract(o.intent_json,'$.op')='membership' AND x.actor_participant_id=?4 AND x.actor_membership_epoch=?5)",params![actor.identity_id,request.operation_id,request.conversation_id,own.participant_id,own.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())
}
pub(crate) fn origin(c: &Connection, actor: &Actor, request: &MembershipRequest) -> Result<Origin> {
    let current = match audience::member(c, &actor.identity_id, &request.conversation_id, true) {
        Ok(member) => Some(member),
        Err(error) if error.code == "forbidden" => None,
        Err(error) => return Err(error),
    };
    if let Some(member) = current {
        return Ok(Origin {
            authority: "member",
            participant: Some((member.participant_id, member.membership_epoch)),
        });
    }
    if cleanup_receipt(c, actor, request)? {
        let member = audience::member(c, &actor.identity_id, &request.conversation_id, false)?;
        return Ok(Origin {
            authority: "member",
            participant: Some((member.participant_id, member.membership_epoch)),
        });
    }
    if matches!(
        request.change,
        MembershipAction::Join { .. } | MembershipAction::Decline { .. }
    ) {
        return Ok(Origin {
            authority: "invitation",
            participant: None,
        });
    }
    if actor.permits("admin") {
        return Ok(Origin {
            authority: "administrator",
            participant: None,
        });
    }
    Err(Error::forbidden())
}
fn participant(c: &Connection, request: &MembershipRequest, id: &str) -> Result<String> {
    c.query_row("SELECT p.identity_id FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=?1 AND p.participant_id=?2 AND i.revoked=0",params![request.conversation_id,id],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
pub(crate) fn resources(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
) -> Result<Vec<String>> {
    authorize(c, actor, request)?;
    let mut result = vec![actor.identity_id.clone()];
    match &request.change {
        MembershipAction::Add {
            identity_id, kind, ..
        }
        | MembershipAction::Invite {
            identity_id, kind, ..
        } => {
            crate::provisioning::require_identity(
                c,
                actor,
                &tenant(c, actor, &request.conversation_id)?,
                identity_id,
                true,
            )?;
            let actual: String = c
                .query_row(
                    "SELECT kind FROM identities WHERE id=?1",
                    [identity_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if actual != kind.as_str() {
                return Err(Error::forbidden());
            }
            result.push(identity_id.clone());
        }
        MembershipAction::Join { invitation_id }
        | MembershipAction::Decline { invitation_id }
        | MembershipAction::CancelInvitation { invitation_id } => {
            let invite = invitation(c, invitation_id, &request.conversation_id)?;
            if !actor.permits_recipient(&invite.identity_id) {
                return Err(Error::forbidden());
            }
            result.push(invite.identity_id);
            let live:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM membership_invitations x JOIN identities i ON i.id=x.inviter_id AND i.tenant_id=x.tenant_id AND i.revoked=0 WHERE x.id=?1 AND x.conversation_id=?2)",params![invitation_id,request.conversation_id],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !live {
                return Err(Error::forbidden());
            }
        }
        MembershipAction::Remove { participant_id, .. }
        | MembershipAction::UpdateRole { participant_id, .. } => {
            result.push(participant(c, request, participant_id)?)
        }
        MembershipAction::ShareHistory {
            resources,
            participants,
            ..
        } => {
            for id in participants {
                result.push(participant(c, request, id)?);
            }
            for resource in resources {
                if let Some(id) = audience::source_owner(
                    c,
                    &request.conversation_id,
                    resource.kind,
                    &resource.id,
                )? {
                    result.push(id);
                }
            }
        }
        MembershipAction::RevokeHistoryShare { sharing_id } => {
            let owner: Option<String> = c
                .query_row(
                    "SELECT owner_id FROM history_shares WHERE id=?1 AND conversation_id=?2",
                    params![sharing_id, request.conversation_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| Error::storage())?;
            let owner = owner.ok_or_else(Error::forbidden)?;
            if owner != actor.identity_id && !actor.permits("admin") {
                return Err(Error::forbidden());
            }
            result.push(owner);
            let mut st=c.prepare("SELECT DISTINCT p.identity_id FROM history_share_items x JOIN participants p ON p.participant_id=x.participant_id AND p.conversation_id=x.conversation_id WHERE x.sharing_id=?1").map_err(|_|Error::storage())?;
            result.extend(
                st.query_map([sharing_id], |r| r.get::<_, String>(0))
                    .map_err(|_| Error::storage())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|_| Error::storage())?,
            );
        }
        MembershipAction::Leave {} => (),
    }
    result.sort();
    result.dedup();
    if result.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    Ok(result)
}
pub(crate) fn scopes(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
) -> Result<Vec<AuthorityScope>> {
    let mut normalized = request.clone();
    normalized.normalize()?;
    let resources = resources(c, actor, &normalized)?;
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &request.conversation_id,
        AuthorityMode::Write,
    )?;
    scopes.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Write,
    )?);
    for identity in resources {
        scopes.push(AuthorityScope::identity(&identity, AuthorityMode::Write)?);
    }
    // Invitation source identity remains fenced even while its target is not a member.
    if let MembershipAction::Join { invitation_id }
    | MembershipAction::Decline { invitation_id }
    | MembershipAction::CancelInvitation { invitation_id } = &request.change
    {
        let issuer: String = c
            .query_row(
                "SELECT inviter_id FROM membership_invitations WHERE id=?1 AND conversation_id=?2",
                params![invitation_id, request.conversation_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        scopes.push(AuthorityScope::identity(&issuer, AuthorityMode::Read)?);
    }
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operations WHERE owner_id=?1 AND operation_id=?2)",
            params![actor.identity_id, request.operation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if exists {
        scopes.extend(crate::transitions::operation_source_scopes(
            c,
            actor,
            &crate::PeerOperationClaim {
                action: connect_protocol::transitions::OperationAction::Get {
                    operation_id: request.operation_id.clone(),
                },
                conversation_id: None,
                session_id: None,
                connection_id: None,
            },
            false,
        )?);
    }
    AuthorityScope::merged(&scopes)
}
pub(crate) fn apply(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
    store: &Store,
) -> Result<MembershipOutcome> {
    if revision(c, &request.conversation_id)? != request.expected_revision {
        return Err(conflict());
    }
    let closed: bool = c
        .query_row(
            "SELECT closed FROM conversations WHERE id=?1",
            [&request.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if closed
        && !matches!(
            request.change,
            MembershipAction::Leave {}
                | MembershipAction::Remove { .. }
                | MembershipAction::RevokeHistoryShare { .. }
        )
    {
        return Err(Error::new("conversation-closed", "Conversation is closed"));
    }
    let extra = match &request.change {
        MembershipAction::ShareHistory {
            resources,
            participants,
            ..
        } => resources.len().saturating_mul(participants.len()),
        _ => 0,
    };
    if !crate::conversations::capacity(c, (store.limits.participants + extra) as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Durable membership event capacity exceeded",
        ));
    }
    let mut result = mutations::apply(c, actor, request, store)?;
    c.execute(
        "UPDATE conversations SET revision=?2 WHERE id=?1 AND revision=?3",
        params![
            request.conversation_id,
            request.expected_revision + 1,
            request.expected_revision
        ],
    )
    .map_err(|_| Error::storage())?;
    let current = revision(c, &request.conversation_id)?;
    match &mut result {
        MembershipOutcome::Invitation { invitation, .. } => {
            invitation.conversation_revision = current
        }
        MembershipOutcome::Participant { revision, .. }
        | MembershipOutcome::Sharing { revision, .. } => *revision = current,
    }
    mutations::emit(c, actor, request, &result, None, current)?;
    Ok(result)
}
pub(crate) fn check_origin(
    c: &Connection,
    actor: &Actor,
    request: &MembershipRequest,
    authority: &str,
) -> Result<()> {
    match authority {
        "member" => (),
        "administrator" => actor.require("admin")?,
        "invitation" => match &request.change {
            MembershipAction::Join { invitation_id }
            | MembershipAction::Decline { invitation_id } => {
                if invitation(c, invitation_id, &request.conversation_id)?.identity_id
                    != actor.identity_id
                {
                    return Err(Error::forbidden());
                }
            }
            _ => return Err(Error::forbidden()),
        },
        _ => return Err(Error::forbidden()),
    }
    authorize(c, actor, request)
}
