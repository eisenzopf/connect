//! Ordinary mutations retain typed intent and origin in the shared operation ledger.
use crate::{artifact_links::TaskLinkContext, Actor, AuthorityMode, AuthorityScope};
use connect_protocol::{membership::ContentKind, Error, Operation, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension};
pub(super) fn valid_id(id: &str) -> Result<()> {
    connect_protocol::transitions::id(id)
}
pub(crate) struct Saved {
    pub digest: String,
    pub operation: Operation,
    pub resources: Vec<String>,
    pub result: Result<Outcome>,
    pub task_id: Option<String>,
    pub grant_id: Option<String>,
    pub conversation_id: Option<String>,
    pub participant: Option<(String, u64)>,
    pub authority: String,
}
pub(crate) fn receipt(c: &Connection, actor: &str, id: &str) -> Result<Option<Saved>> {
    let record:Option<String>=c.query_row("SELECT o.record_id FROM operations o JOIN operation_control_receipts x ON x.operation_record_id=o.record_id WHERE o.owner_id=?1 AND o.operation_id=?2",params![actor,id],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    record.map(|record| by_record(c, &record)).transpose()
}
pub(crate) fn by_record(c: &Connection, record: &str) -> Result<Saved> {
    let row=c.query_row("SELECT o.digest,o.intent_json,o.result_json,o.problem_json,o.state,o.conversation_id,x.resources_json,x.task_id,x.grant_id,x.actor_participant_id,x.actor_membership_epoch,x.actor_authority FROM operations o JOIN operation_control_receipts x ON x.operation_record_id=o.record_id WHERE o.record_id=?1",[record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,String>(6)?,r.get::<_,Option<String>>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,Option<String>>(9)?,r.get::<_,Option<u64>>(10)?,r.get::<_,String>(11)?))).map_err(|_|Error::storage())?;
    let (
        digest,
        intent,
        result,
        problem,
        state,
        conversation_id,
        resources,
        task_id,
        grant_id,
        pid,
        epoch,
        authority,
    ) = row;
    let outcome = match state.as_str() {
        "completed" if problem.is_none() => {
            Ok(serde_json::from_str(&result.ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?)
        }
        "failed" if result.is_none() => {
            Err(serde_json::from_str(&problem.ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?)
        }
        _ => return Err(Error::storage()),
    };
    let participant = match (pid, epoch) {
        (Some(id), Some(epoch)) => Some((id, epoch)),
        (None, None) => None,
        _ => return Err(Error::storage()),
    };
    Ok(Saved {
        digest,
        operation: serde_json::from_str(&intent).map_err(|_| Error::storage())?,
        resources: serde_json::from_str(&resources).map_err(|_| Error::storage())?,
        result: outcome,
        task_id,
        grant_id,
        conversation_id,
        participant,
        authority,
    })
}
pub(crate) fn check_receipt_authority(
    c: &Connection,
    actor: &Actor,
    saved: &Saved,
    context: Option<&TaskLinkContext>,
) -> Result<()> {
    if saved.task_id.as_deref() != context.map(|v| v.task_id.as_str())
        || saved.grant_id.as_deref() != context.and_then(|v| v.grant_id.as_deref())
    {
        return Err(Error::forbidden());
    }
    actor.require(saved.operation.required_scope())?;
    if saved.resources.iter().any(|p| !actor.permits_recipient(p)) {
        return Err(Error::forbidden());
    }
    if saved.authority == "member" {
        let cid = saved
            .conversation_id
            .as_deref()
            .ok_or_else(Error::storage)?;
        let cleanup = match &saved.operation {
            Operation::Membership { request } => {
                crate::membership::control::cleanup_receipt(c, actor, request)?
            }
            _ => false,
        };
        let member = crate::audience::member(c, &actor.identity_id, cid, !cleanup)?;
        if saved.participant.as_ref() != Some(&(member.participant_id, member.membership_epoch)) {
            return Err(Error::forbidden());
        }
    }
    if let Operation::Membership { request } = &saved.operation {
        crate::membership::control::check_origin(c, actor, request, &saved.authority)?;
    } else if saved.conversation_id.is_some() && saved.authority != "member" {
        return Err(Error::forbidden());
    }
    if let Ok(value) = &saved.result {
        crate::audience::outcome(c, actor, value)?;
        match value {
            Outcome::Accepted {
                message_id,
                conversation_id,
                ..
            } => crate::audience::require_original(
                c,
                actor,
                conversation_id,
                ContentKind::Message,
                message_id,
            )?,
            Outcome::Artifact(a) => crate::audience::require_original(
                c,
                actor,
                &a.conversation_id,
                ContentKind::Artifact,
                &a.id,
            )?,
            _ => (),
        }
    }
    Ok(())
}
/// Receipt identity is durable; conversation state is projected under current authority.
pub(crate) fn view_result(c: &Connection, actor: &Actor, saved: &Saved) -> Result<Result<Outcome>> {
    if let (Operation::Membership { request }, Ok(Outcome::Membership(value))) =
        (&saved.operation, &saved.result)
    {
        return crate::membership::authorize_replay(c, actor, request, value.clone())
            .map(|v| Ok(Outcome::Membership(v)));
    }
    if let Ok(Outcome::Conversation(old)) = &saved.result {
        let mut current = crate::conversation_control::view(c, actor, &old.id)?;
        current.receipt = old.receipt.clone();
        Ok(Ok(Outcome::Conversation(current)))
    } else {
        Ok(saved.result.clone())
    }
}
pub(crate) fn projection_scopes(c: &Connection, record: &str) -> Result<Vec<AuthorityScope>> {
    let saved = by_record(c, record)?;
    let mut result = vec![];
    for id in &saved.resources {
        result.push(AuthorityScope::identity(id, AuthorityMode::Read)?);
    }
    // Conversation receipts project current appearances, including members added
    // after acceptance. Retain those current Identity readers through delivery.
    if let Ok(Outcome::Conversation(value)) = &saved.result {
        result.extend(crate::authority_admission::conversation_scopes(
            c,
            &value.id,
            AuthorityMode::Read,
        )?);
    }
    if let Operation::Membership { request } = &saved.operation {
        // The frozen resource identities include source owners and target epochs;
        // current Conversation member readers are already added by the caller.
        if let connect_protocol::membership::MembershipAction::Join { invitation_id }
        | connect_protocol::membership::MembershipAction::Decline { invitation_id }
        | connect_protocol::membership::MembershipAction::CancelInvitation { invitation_id } =
            &request.change
        {
            let issuer:String=c.query_row("SELECT inviter_id FROM membership_invitations WHERE id=?1 AND conversation_id=?2",params![invitation_id,request.conversation_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
            result.push(AuthorityScope::identity(&issuer, AuthorityMode::Read)?);
        }
    }
    // Parent/attachment owners may be outside the explicit message audience.
    if let Operation::Send {
        conversation_id,
        message,
        ..
    } = &saved.operation
    {
        let sources = message
            .attachments
            .iter()
            .map(|a| (ContentKind::Artifact, a.id.as_str()))
            .chain(
                message
                    .in_reply_to_msg
                    .iter()
                    .map(|id| (ContentKind::Message, id.as_str())),
            );
        for (kind, id) in sources {
            if let Some(owner) = crate::audience::source_owner(c, conversation_id, kind, id)? {
                result.push(AuthorityScope::identity(&owner, AuthorityMode::Read)?);
            }
        }
    }
    Ok(result)
}
pub(super) fn save(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    digest: &str,
    resources: &[String],
    result: &Result<Outcome>,
    context: Option<&TaskLinkContext>,
    origin: Option<&crate::membership::control::Origin>,
) -> Result<()> {
    let record = format!("opr_{}", uuid::Uuid::new_v4());
    let mut intent = crate::artifact_links::canonical_intent(
        serde_json::to_value(operation).map_err(|_| Error::storage())?,
    );
    // A proof of possession is not replay execution intent. Retain its digest,
    // rather than another plaintext copy of the one-time pairing challenge.
    if matches!(operation, Operation::ContactPairingComplete { .. }) {
        if let Some(challenge) = intent.get_mut("challenge") {
            use sha2::{Digest, Sha256};
            *challenge = serde_json::Value::String(hex::encode(Sha256::digest(
                challenge.as_str().ok_or_else(Error::storage)?.as_bytes(),
            )));
        }
    }
    let cid = operation
        .conversation_id()
        .map(str::to_owned)
        .or_else(|| match result {
            Ok(Outcome::Conversation(v)) => Some(v.id.clone()),
            _ => None,
        });
    let sid = match operation {
        Operation::ConnectionPrepare { request } => Some(request.intent.session_id.clone()),
        Operation::SessionControl { request } => Some(request.session_id.clone()),
        Operation::SessionDecision { request } => Some(request.session_id.clone()),
        _ => None,
    }
    .or_else(|| match result {
        Ok(Outcome::SessionControl(v)) => Some(v.session.id.clone()),
        Ok(Outcome::Session(v)) => Some(v.id.clone()),
        Ok(Outcome::SessionDecision(v)) => Some(v.session_id.clone()),
        _ => None,
    });
    // A failed creation has an asserted target in its immutable intent, but
    // cannot manufacture a foreign-key reference to a Session that never existed.
    let sid = if let Some(id) = sid {
        c.query_row(
            "SELECT id FROM sessions WHERE id=?1 AND conversation_id=?2",
            params![id, cid],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
    } else {
        None
    };
    let expected_revision = match operation {
        Operation::ConnectionPrepare { request } => request.intent.expected_revision,
        Operation::SessionControl { request } => request.expected_revision,
        Operation::SessionDecision { request } => request.expected_revision,
        _ => None,
    };
    let participant = if let Some(origin) = origin {
        if origin.authority == "invitation"
            && matches!(
                result,
                Ok(Outcome::Membership(
                    connect_protocol::membership::MembershipOutcome::Participant { .. }
                ))
            )
        {
            let own = crate::audience::member(
                c,
                &actor.identity_id,
                cid.as_deref().ok_or_else(Error::storage)?,
                true,
            )?;
            Some((own.participant_id, own.membership_epoch))
        } else {
            origin.participant.clone()
        }
    } else {
        cid.as_deref()
            .map(|cid| crate::audience::member(c, &actor.identity_id, cid, true))
            .transpose()?
            .map(|m| (m.participant_id, m.membership_epoch))
    };
    let authority = if participant.is_some() {
        "member"
    } else {
        origin.map(|o| o.authority).unwrap_or("unbound")
    };
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let (state, value, problem) = match result {
        Ok(v) => (
            "completed",
            Some(serde_json::to_string(v).map_err(|_| Error::storage())?),
            None,
        ),
        Err(e) => (
            "failed",
            None,
            Some(serde_json::to_string(e).map_err(|_| Error::storage())?),
        ),
    };
    let connid = if let Operation::ConnectionPrepare { request } = operation {
        c.query_row(
            "SELECT id FROM connections WHERE id=?1 AND session_id=?2",
            params![request.intent.connection_id, sid],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
    } else {
        None
    };
    let command_type = match operation {
        Operation::ConnectionPrepare { .. } => "connection.prepare",
        _ => operation.required_scope(),
    };
    c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,connection_id,expected_revision,intent_json,state,result_json,problem_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?15)",params![record,tenant,actor.identity_id,operation.request_id().ok_or_else(Error::storage)?,digest,command_type,cid,sid,connid,expected_revision,crate::provisioning::canonical_json(&intent)?,state,value,problem,crate::now()]).map_err(|_|Error::storage())?;
    c.execute(
        "INSERT INTO operation_control_receipts VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            record,
            serde_json::to_string(resources).map_err(|_| Error::storage())?,
            context.map(|v| &v.task_id),
            context.and_then(|v| v.grant_id.as_deref()),
            participant.as_ref().map(|v| &v.0),
            participant.as_ref().map(|v| v.1),
            authority
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
