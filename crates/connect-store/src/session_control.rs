//! General Session commands run in the ordinary mutation/receipt transaction.
use crate::{Actor, Store};
use connect_protocol::{membership::ContentKind, session_control::*, Error, Outcome, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn conflict() -> Error {
    Error::new("conflict", "Session intent or topology revision conflicts")
}
fn unsupported() -> Error {
    Error::new(
        "unsupported-feature",
        "Session media profile is unavailable",
    )
}
fn member(
    c: &Connection,
    cid: &str,
    pid: &str,
) -> Result<connect_protocol::membership::MemberView> {
    let identity: Option<String> = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",
            params![pid, cid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    crate::audience::member(c, &identity.ok_or_else(Error::forbidden)?, cid, true)
}
// Resolve only current members of this explicit Conversation. Preserve the
// original requested references in the receipt/digest; resolution grants no join.
fn invite_targets(
    c: &Connection,
    store: &Store,
    cid: &str,
    references: &[String],
) -> Result<Vec<connect_protocol::membership::MemberView>> {
    let mut targets = Vec::new();
    let mut seen = BTreeSet::new();
    for reference in references {
        let target = crate::session_gateways::resolve(c, store, cid, reference)?;
        if !seen.insert(target.participant_id.clone()) {
            return Err(Error::new(
                "invalid-request",
                "Repeated Session invitation target",
            ));
        }
        targets.push(target);
    }
    Ok(targets)
}
fn owner(c: &Connection, cid: &str, sid: &str) -> Result<String> {
    crate::audience::source_owner(c, cid, ContentKind::Session, sid)?.ok_or_else(Error::forbidden)
}
pub(crate) fn manager(c: &Connection, actor: &Actor, r: &Request) -> Result<()> {
    if owner(c, &r.conversation_id, &r.session_id)? == actor.identity_id
        || actor.permits("session.manage")
        || actor.permits("admin")
    {
        Ok(())
    } else {
        Err(Error::forbidden())
    }
}
fn own(
    c: &Connection,
    actor: &Actor,
    r: &Request,
) -> Result<connect_protocol::membership::MemberView> {
    let own = crate::audience::member(c, &actor.identity_id, &r.conversation_id, true)?;
    if r.by_participant
        .as_ref()
        .is_some_and(|by| by != &own.participant_id)
    {
        return Err(Error::forbidden());
    }
    Ok(own)
}
fn appearance(c: &Connection, r: &Request, pid: &str, epoch: u64) -> Result<Option<String>> {
    c.query_row("SELECT state FROM session_participants WHERE session_id=?1 AND conversation_id=?2 AND participant_id=?3 AND membership_epoch=?4",params![r.session_id,r.conversation_id,pid,epoch],|row|row.get(0)).optional().map_err(|_|Error::storage())
}
fn provider_role_supported(c: &Connection, r: &Request) -> Result<()> {
    if let Action::Role { participant_id, .. } = &r.action {
        let controlled:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM provider_session_voice WHERE closed_at IS NULL AND json_extract(source_json,'$.request.conversation_id')=?1 AND json_extract(source_json,'$.request.session_id')=?2 AND (target_participant_id=?3 OR json_extract(source_json,'$.caller.participant_id')=?3))",params![r.conversation_id,r.session_id,participant_id],|row|row.get(0)).map_err(|_|Error::storage())?;
        if controlled {
            return Err(Error::new("unsupported-feature","Provider telephone control has no live media role projection; end the external call first"));
        }
    }
    Ok(())
}
pub(crate) fn resources(
    c: &Connection,
    store: &Store,
    actor: &Actor,
    r: &Request,
) -> Result<Vec<String>> {
    let mut normalized = r.clone();
    normalized.normalize()?;
    actor.require(r.action.required_scope())?;
    let own = own(c, actor, r)?;
    let conversation = crate::conversations::authorized_conversation(c, actor, &r.conversation_id)?;
    // Recover the original local receipt before revalidating a new topology.
    // The common engine subsequently checks the normalized intent digest.
    if let Some(saved) =
        crate::operations::receipts::receipt(c, &actor.identity_id, &r.operation_id)?
    {
        crate::operations::receipts::check_receipt_authority(c, actor, &saved, None)?;
        if saved.conversation_id.as_deref() != Some(r.conversation_id.as_str()) {
            return Err(Error::forbidden());
        }
        return Ok(saved.resources);
    }
    if conversation.closed {
        return Err(Error::forbidden());
    }
    provider_role_supported(c, r)?;
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            [&r.session_id],
            |row| row.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut identities = BTreeSet::from([actor.identity_id.clone()]);
    if exists {
        crate::sessions::get(c, &r.conversation_id, &r.session_id)?;
        crate::audience::require_original(
            c,
            actor,
            &r.conversation_id,
            ContentKind::Session,
            &r.session_id,
        )?;
        identities.insert(owner(c, &r.conversation_id, &r.session_id)?);
    } else if !matches!(r.action, Action::Invite { .. }) {
        return Err(Error::forbidden());
    }
    match &r.action {
        Action::Invite { participants, .. } => {
            if exists {
                manager(c, actor, r)?;
                if participants.is_empty() {
                    return Err(Error::new(
                        "invalid-request",
                        "An existing Session invitation requires a target",
                    ));
                }
            }
            for target in invite_targets(c, store, &r.conversation_id, participants)? {
                if target.participant_id == own.participant_id {
                    return Err(Error::forbidden());
                }
                identities.insert(target.identity_id);
            }
        }
        // A current Session Participant with the named session.end permission
        // can clear the whole Session. Presence supplies no management rights.
        Action::End { .. } => {
            if appearance(c, r, &own.participant_id, own.membership_epoch)?.as_deref()
                != Some("present")
            {
                manager(c, actor, r)?;
            }
        }
        Action::Cancel { .. } | Action::Role { .. } | Action::Renegotiate { .. } => {
            manager(c, actor, r)?
        }
        Action::Leave { participant_id } if participant_id != &own.participant_id => {
            manager(c, actor, r)?
        }
        _ => (),
    }
    if let Action::Leave { participant_id } | Action::Role { participant_id, .. } = &r.action {
        identities.insert(member(c, &r.conversation_id, participant_id)?.identity_id);
    }
    if matches!(
        r.action,
        Action::End { .. } | Action::Cancel { .. } | Action::Renegotiate { .. }
    ) {
        let mut query=c.prepare("SELECT DISTINCT p.identity_id FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities i ON i.id=p.identity_id AND i.revoked=0 WHERE sp.session_id=?1 AND sp.state NOT IN ('left','removed')").map_err(|_|Error::storage())?;
        for id in query
            .query_map([&r.session_id], |row| row.get::<_, String>(0))
            .map_err(|_| Error::storage())?
        {
            identities.insert(id.map_err(|_| Error::storage())?);
        }
    }
    if identities.iter().any(|id| !actor.permits_recipient(id)) {
        return Err(Error::forbidden());
    }
    Ok(identities.into_iter().collect())
}
fn caps(c: &Connection, r: &Request) -> Result<Value> {
    let raw:Option<String>=c.query_row("SELECT capabilities_json FROM session_control WHERE session_id=?1 AND conversation_id=?2",params![r.session_id,r.conversation_id],|row|row.get(0)).optional().map_err(|_|Error::storage())?;
    serde_json::from_str(&raw.ok_or_else(unsupported)?).map_err(|_| Error::storage())
}
fn codecs<'a>(value: &'a Value, kind: &str) -> Vec<&'a str> {
    value
        .get(kind)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}
fn supported_codec(kind: &str, codec: &str) -> bool {
    match kind {
        "audio" => matches!(codec, "opus" | "g.711-mu" | "g.711-a"),
        "video" | "screen" => codec == "vp8",
        "data" => codec == "reliable-ordered",
        _ => false,
    }
}
pub(crate) fn supports(offer: &Value, answer: &Value, kinds: &[String]) -> bool {
    kinds.iter().any(|kind| {
        codecs(answer, kind)
            .iter()
            .any(|codec| supported_codec(kind, codec) && codecs(offer, kind).contains(codec))
    })
}
fn media_json(c: &Connection, sid: &str) -> Result<Vec<String>> {
    let raw: String = c
        .query_row("SELECT media FROM sessions WHERE id=?1", [sid], |row| {
            row.get(0)
        })
        .map_err(|_| Error::storage())?;
    serde_json::from_str(&raw).map_err(|_| Error::storage())
}
pub(crate) fn view(c: &Connection, cid: &str, sid: &str) -> Result<SessionView> {
    c.query_row("SELECT s.id,s.conversation_id,s.state,s.media,sc.intent,EXISTS(SELECT 1 FROM main_sessions m WHERE m.conversation_id=s.conversation_id AND m.session_id=s.id),s.state_revision,s.topology_revision,s.created_at,s.updated_at,s.ended_reason,sc.capabilities_json,sc.codec_preferences_json FROM sessions s LEFT JOIN session_control sc ON sc.session_id=s.id AND sc.conversation_id=s.conversation_id WHERE s.id=?1 AND s.conversation_id=?2",params![sid,cid],|row| {
        let media:String=row.get(3)?;
        Ok(SessionView{id:row.get(0)?,conversation_id:row.get(1)?,state:row.get(2)?,media:serde_json::from_str(&media).map_err(|_|rusqlite::Error::InvalidQuery)?,intent:row.get(4)?,is_main:row.get(5)?,revision:row.get(6)?,topology_revision:row.get(7)?,created_at:row.get(8)?,updated_at:row.get(9)?,ended_reason:row.get(10)?,capabilities_offer:row.get::<_,Option<String>>(11)?.map(|raw|serde_json::from_str(&raw).map_err(|_|rusqlite::Error::InvalidQuery)).transpose()?,codec_preferences:row.get::<_,Option<String>>(12)?.map(|raw|serde_json::from_str(&raw).map_err(|_|rusqlite::Error::InvalidQuery)).transpose()?.unwrap_or_default()})
    }).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
fn set_caps(c: &Connection, r: &Request, pid: &str, epoch: u64, value: &Value) -> Result<()> {
    c.execute("INSERT INTO session_presence_capabilities VALUES(?1,?2,?3,?4) ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET capabilities_json=excluded.capabilities_json",params![r.session_id,pid,epoch,crate::provisioning::canonical_json(value)?]).map_err(|_|Error::storage())?;
    Ok(())
}
fn retire_presence(c: &Connection, r: &Request, pid: &str, epoch: u64) -> Result<()> {
    c.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state!='removed'",params![r.session_id,pid,epoch]).map_err(|_|Error::storage())?;
    // Generation fencing is immediate; physical retirement is a coordinator duty.
    c.execute("UPDATE connections SET state='retiring',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state NOT IN ('retired','failed','retiring')",params![r.session_id,pid,epoch]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn terminal(c: &Connection, r: &Request, state: &str, reason: &str) -> Result<()> {
    c.execute(
        "UPDATE sessions SET state=?2,ended_reason=?3,terminal_reason_code=?4 WHERE id=?1",
        params![
            r.session_id,
            state,
            reason,
            match &r.action {
                Action::End { reason_code, .. }
                | Action::Cancel { reason_code, .. }
                | Action::Reject { reason_code, .. } => Some(*reason_code),
                _ => None,
            }
        ],
    )
    .map_err(|_| Error::storage())?;
    c.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND state NOT IN ('left','removed')",[&r.session_id]).map_err(|_|Error::storage())?;
    c.execute("UPDATE connections SET state='retiring',generation=generation+1,revision=revision+1,observed_runtime_id=NULL,observed_at=NULL,readiness_json=NULL WHERE session_id=?1 AND state NOT IN ('retired','failed','retiring')",[&r.session_id]).map_err(|_|Error::storage())?;
    c.execute("UPDATE session_bindings SET state='ended',updated_at=?2 WHERE session_id=?1 AND state='ready'",params![r.session_id,crate::now()]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn apply(c: &Connection, actor: &Actor, r: &Request, store: &Store) -> Result<Outcome> {
    resources(c, store, actor, r)?;
    let own = own(c, actor, r)?;
    let current = view(c, &r.conversation_id, &r.session_id);
    let new = matches!(&current,Err(error) if error.code=="forbidden")
        && matches!(r.action, Action::Invite { .. });
    let old = if new { None } else { Some(current?) };
    if let Some(old) = &old {
        if matches!(
            old.state.as_str(),
            "ended" | "rejected" | "cancelled" | "failed"
        ) {
            return Err(conflict());
        }
        let optional = matches!(r.action, Action::Reject { .. } | Action::Cancel { .. })
            && old.state == "invited";
        if (!optional && r.expected_revision.is_none())
            || r.expected_revision
                .is_some_and(|v| v != old.topology_revision)
            || old.topology_revision >= 9_007_199_254_740_991
        {
            return Err(conflict());
        }
    } else if r.expected_revision.is_some() {
        return Err(conflict());
    }
    match &r.action {
        Action::Invite {
            participants,
            medium,
            intent,
            capabilities,
        } => {
            let kinds = media(medium)?;
            if !supports(capabilities, capabilities, &kinds) {
                return Err(unsupported());
            }
            let targets = invite_targets(c, store, &r.conversation_id, participants)?;
            for target in &targets {
                if appearance(c, r, &target.participant_id, target.membership_epoch)?
                    .is_some_and(|s| !matches!(s.as_str(), "left" | "removed"))
                {
                    return Err(conflict());
                }
            }
            let count:u64=c.query_row("SELECT COUNT(*) FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 WHERE sp.session_id=?1 AND sp.state NOT IN ('left','removed')",[&r.session_id],|row|row.get(0)).map_err(|_|Error::storage())?;
            if count + targets.len() as u64 + u64::from(new) > store.limits.participants as u64 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Session Participant capacity exceeded",
                ));
            }
            if new {
                let sessions: u64 = c
                    .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
                    .map_err(|_| Error::storage())?;
                if sessions >= store.limits.sessions {
                    return Err(Error::new("capacity-exceeded", "Session capacity exceeded"));
                }
                // A solo intent has no pending remote acceptance. Accepted
                // capability intent remains distinct from actual media flow.
                let participant = targets
                    .first()
                    .map_or(actor.identity_id.as_str(), |t| t.identity_id.as_str());
                let state = if targets.is_empty() {
                    "accepted"
                } else {
                    "invited"
                };
                c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at,audience_mode) VALUES(?1,?2,?3,?4,?7,?5,?6,?6,'explicit')",params![r.session_id,r.conversation_id,actor.identity_id,participant,serde_json::to_string(&kinds).map_err(|_|Error::storage())?,crate::now(),state]).map_err(|error|if matches!(error,rusqlite::Error::SqliteFailure(ref code,_) if code.code==rusqlite::ErrorCode::ConstraintViolation){conflict()}else{Error::storage()})?;
                insert_presence(c, r, &own, "present")?;
                set_caps(
                    c,
                    r,
                    &own.participant_id,
                    own.membership_epoch,
                    capabilities,
                )?;
                c.execute("INSERT INTO session_control(session_id,conversation_id,creator_participant_id,creator_membership_epoch,intent,capabilities_json) VALUES(?1,?2,?3,?4,?5,?6)",params![r.session_id,r.conversation_id,own.participant_id,own.membership_epoch,intent,crate::provisioning::canonical_json(capabilities)?]).map_err(|_|Error::storage())?;
                c.execute("INSERT INTO main_sessions VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET session_id=excluded.session_id WHERE EXISTS(SELECT 1 FROM sessions s WHERE s.id=main_sessions.session_id AND s.state IN ('ended','rejected','cancelled','failed'))",params![r.conversation_id,r.session_id]).map_err(|_|Error::storage())?;
            } else {
                let (old_intent, old_caps): (String, String) = c
                    .query_row(
                        "SELECT intent,capabilities_json FROM session_control WHERE session_id=?1",
                        [&r.session_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|_| Error::storage())?
                    .ok_or_else(unsupported)?;
                if old_intent != *intent
                    || media_json(c, &r.session_id)? != kinds
                    || serde_json::from_str::<Value>(&old_caps).map_err(|_| Error::storage())?
                        != *capabilities
                {
                    return Err(conflict());
                }
            }
            for target in &targets {
                insert_presence(c, r, target, "invited")?;
            }
            if !targets.is_empty() {
                crate::session_invitations::freeze(
                    c,
                    store,
                    actor,
                    r,
                    &own,
                    &targets,
                    medium,
                    intent,
                    capabilities,
                )?;
            }
        }
        Action::Accept { capabilities } => {
            if appearance(c, r, &own.participant_id, own.membership_epoch)?.as_deref()
                != Some("invited")
            {
                return Err(conflict());
            }
            if !supports(&caps(c, r)?, capabilities, &media_json(c, &r.session_id)?) {
                return Err(unsupported());
            }
            c.execute("UPDATE session_participants SET state='present' WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![r.session_id,own.participant_id,own.membership_epoch]).map_err(|_|Error::storage())?;
            set_caps(
                c,
                r,
                &own.participant_id,
                own.membership_epoch,
                capabilities,
            )?;
            c.execute("UPDATE sessions SET state=CASE WHEN state='invited' THEN 'accepted' ELSE state END WHERE id=?1",[&r.session_id]).map_err(|_|Error::storage())?;
        }
        Action::Reject { reason, .. } => {
            if appearance(c, r, &own.participant_id, own.membership_epoch)?.as_deref()
                != Some("invited")
            {
                return Err(conflict());
            }
            retire_presence(c, r, &own.participant_id, own.membership_epoch)?;
            let remaining:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 WHERE sp.session_id=?1 AND sp.state IN ('invited','joining','present') AND p.identity_id<>(SELECT owner_id FROM audience_resources WHERE kind='session' AND id=?1))",[&r.session_id],|row|row.get(0)).map_err(|_|Error::storage())?;
            if !remaining {
                terminal(c, r, "rejected", reason)?;
            }
        }
        Action::Cancel { reason, .. } => {
            // Cancellation belongs to the unaccepted invitation. Once any
            // invitee accepts, clearing the Session requires session.end;
            // pending invitees do not reopen the cancellation boundary.
            if old.as_ref().is_none_or(|s| s.state != "invited") {
                return Err(conflict());
            }
            crate::session_invitations::cancellations::freeze(c, r, &own)?;
            let changed=c.execute("UPDATE session_participants SET state='left' WHERE session_id=?1 AND state='invited'",[&r.session_id]).map_err(|_|Error::storage())?;
            if changed == 0 {
                return Err(conflict());
            }
            if old.as_ref().is_some_and(|s| s.state == "invited") {
                terminal(c, r, "cancelled", reason)?;
            }
        }
        Action::Leave { participant_id } => {
            let target = member(c, &r.conversation_id, participant_id)?;
            if appearance(c, r, participant_id, target.membership_epoch)?.as_deref()
                != Some("present")
            {
                return Err(conflict());
            }
            retire_presence(c, r, participant_id, target.membership_epoch)?;
            let remains:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 WHERE sp.session_id=?1 AND sp.state IN ('invited','joining','present'))",[&r.session_id],|row|row.get(0)).map_err(|_|Error::storage())?;
            if !remains {
                terminal(c, r, "ended", "last-participant-left")?;
            }
        }
        Action::Role {
            participant_id,
            role,
        } => {
            let target = member(c, &r.conversation_id, participant_id)?;
            if !matches!(
                appearance(c, r, participant_id, target.membership_epoch)?.as_deref(),
                Some("present" | "invited")
            ) {
                return Err(conflict());
            }
            c.execute("UPDATE session_participants SET role=?4 WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![r.session_id,participant_id,target.membership_epoch,role]).map_err(|_|Error::storage())?;
        }
        Action::Upgrade { media } => {
            if appearance(c, r, &own.participant_id, own.membership_epoch)?.as_deref()
                != Some("present")
            {
                return Err(Error::forbidden());
            }
            let answer:String=c.query_row("SELECT capabilities_json FROM session_presence_capabilities WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![r.session_id,own.participant_id,own.membership_epoch],|row|row.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(unsupported)?;
            let answer: Value = serde_json::from_str(&answer).map_err(|_| Error::storage())?;
            let offer = caps(c, r)?;
            if media
                .iter()
                .any(|kind| !supports(&offer, &answer, std::slice::from_ref(kind)))
            {
                return Err(unsupported());
            }
            require_additive_native_media(c, r, store)?;
            let mut kinds: BTreeSet<_> = media_json(c, &r.session_id)?.into_iter().collect();
            kinds.extend(media.iter().cloned());
            c.execute(
                "UPDATE sessions SET media=?2 WHERE id=?1",
                params![
                    r.session_id,
                    serde_json::to_string(&kinds).map_err(|_| Error::storage())?
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        Action::Renegotiate { codec_preferences } => {
            let offer = caps(c, r)?;
            if codec_preferences.iter().any(|codec| {
                !["audio", "video", "screen", "data"].iter().any(|kind| {
                    supported_codec(kind, codec) && codecs(&offer, kind).contains(&codec.as_str())
                })
            }) {
                return Err(unsupported());
            }
            if store
                .session_codec_coordinator
                .load(std::sync::atomic::Ordering::Acquire)
            {
                require_additive_native_media(c, r, store)?;
            } else {
                require_idle_media(c, r)?;
            }
            c.execute(
                "UPDATE session_control SET codec_preferences_json=?2 WHERE session_id=?1",
                params![
                    r.session_id,
                    serde_json::to_string(codec_preferences).map_err(|_| Error::storage())?
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        Action::End { reason, .. } => terminal(c, r, "ended", reason)?,
    }
    if !new {
        c.execute(
            "UPDATE sessions SET topology_revision=topology_revision+1,updated_at=?2 WHERE id=?1",
            params![r.session_id, crate::now()],
        )
        .map_err(|_| Error::storage())?;
    }
    let departure = match &r.action {
        Action::Leave { participant_id } => Some((participant_id.as_str(), "explicit-leave")),
        _ => None,
    };
    crate::session_participant_lifecycle::reconcile(
        c,
        store,
        &r.conversation_id,
        &r.session_id,
        departure,
    )?;
    let session = view(c, &r.conversation_id, &r.session_id)?;
    emit(c, actor, r, &session, store)?;
    if matches!(r.action, Action::Reject { .. } | Action::Cancel { .. }) {
        let (reason_code, reason) = r.action.reason();
        Ok(Outcome::SessionDecision(
            connect_protocol::session_decisions::DecisionOutcome {
                operation_id: r.operation_id.clone(),
                conversation_id: r.conversation_id.clone(),
                session_id: r.session_id.clone(),
                state: if matches!(r.action, Action::Reject { .. }) {
                    "rejected"
                } else {
                    "cancelled"
                }
                .into(),
                revision: session.topology_revision,
                reason_code,
                reason: reason.into(),
            },
        ))
    } else {
        let codec_reconciliation =
            crate::transitions::session_codecs::enqueue(c, actor, r, &session, store)?;
        Ok(Outcome::SessionControl(ControlOutcome {
            codec_reconciliation,
            operation_id: r.operation_id.clone(),
            action: r.action.name().into(),
            session,
        }))
    }
}
/// An additive accepted Session intent does not renegotiate existing ports.
/// New media still needs its own current signed Connection offer. Until the
/// corresponding coordinators are installed, refuse legacy or unsettled slots.
fn require_additive_native_media(c: &Connection, r: &Request, store: &Store) -> Result<()> {
    let unsupported: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_bindings WHERE session_id=?1 AND state='ready') OR EXISTS(SELECT 1 FROM connections slot WHERE slot.session_id=?1 AND slot.state NOT IN ('retired','failed','interrupted') AND NOT EXISTS(SELECT 1 FROM native_connection_lifetimes life JOIN native_admission_sources n ON n.operation_record_id=life.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE life.connection_id=slot.id AND life.retired=0 AND life.generation=slot.generation AND life.runtime_epoch=?2 AND n.attempt_runtime=?2 AND n.generation=slot.generation AND n.source_expires_at>?3 AND o.state='completed' AND e.state='confirmed' AND slot.state IN ('ready','active') AND slot.observed_runtime_id=?2 AND slot.readiness_json IS NOT NULL))",params![r.session_id,store.runtime_epoch(),crate::now()],|row|row.get(0)).map_err(|_|Error::storage())?;
    if unsupported {
        return Err(Error::new(
            "unsupported-feature",
            "Session upgrade requires settled current native Connections",
        ));
    }
    Ok(())
}
fn require_idle_media(c: &Connection, r: &Request) -> Result<()> {
    let live:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE session_id=?1 AND state NOT IN ('retired','failed','interrupted')) OR EXISTS(SELECT 1 FROM session_bindings WHERE session_id=?1 AND state='ready')",[&r.session_id],|row|row.get(0)).map_err(|_|Error::storage())?;
    if live {
        return Err(Error::new(
            "unsupported-feature",
            "Live capability changes require the media coordinator",
        ));
    }
    Ok(())
}
fn insert_presence(
    c: &Connection,
    r: &Request,
    member: &connect_protocol::membership::MemberView,
    state: &str,
) -> Result<()> {
    c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) SELECT ?1,p.conversation_id,p.tenant_id,p.participant_id,p.membership_epoch,'participant',?2 FROM participants p WHERE p.participant_id=?3 AND p.membership_epoch=?4 AND p.active=1 ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET state=excluded.state,role=excluded.role",params![r.session_id,state,member.participant_id,member.membership_epoch]).map_err(|_|Error::storage())?;
    c.execute(
        "INSERT OR IGNORE INTO resource_audiences VALUES('session',?1,?2,?3,?4)",
        params![
            r.session_id,
            r.conversation_id,
            member.participant_id,
            member.membership_epoch
        ],
    )
    .map_err(|_| Error::storage())?;
    c.execute("DELETE FROM session_presence_capabilities WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3",params![r.session_id,member.participant_id,member.membership_epoch]).map_err(|_|Error::storage())?;
    Ok(())
}
fn emit(
    c: &Connection,
    actor: &Actor,
    r: &Request,
    session: &SessionView,
    store: &Store,
) -> Result<()> {
    emit_transition(
        c,
        &actor.identity_id,
        &r.operation_id,
        &r.conversation_id,
        &r.session_id,
        r.action.name(),
        session,
        store,
    )
}

/// Publish the shared Session revision after a coordinator's atomic local
/// graph decision. Private Connection/content details stay in their own floors.
pub(crate) fn emit_transition(
    c: &Connection,
    owner: &str,
    operation: &str,
    conversation: &str,
    session_id: &str,
    action: &str,
    session: &SessionView,
    store: &Store,
) -> Result<()> {
    emit_transition_inner(
        c,
        owner,
        Some(operation),
        conversation,
        session_id,
        action,
        session,
        store,
    )
}
/// A server-owned invitation observation has an existing source identity, not
/// a caller command/operation ID. Its journal fact omits ordinary correlation.
pub(crate) fn emit_invitation_decision(
    c: &Connection,
    conversation: &str,
    session_id: &str,
    action: &str,
    session: &SessionView,
    store: &Store,
) -> Result<()> {
    emit_transition_inner(
        c,
        "",
        None,
        conversation,
        session_id,
        action,
        session,
        store,
    )
}
#[allow(clippy::too_many_arguments)]
fn emit_transition_inner(
    c: &Connection,
    owner: &str,
    operation: Option<&str>,
    conversation: &str,
    session_id: &str,
    action: &str,
    session: &SessionView,
    store: &Store,
) -> Result<()> {
    let recipients =
        crate::audience::source_members(c, conversation, ContentKind::Session, session_id)?;
    if !crate::conversations::capacity(c, recipients.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Session journal capacity exceeded",
        ));
    }
    let data =
        json!({"source_kind":"session","source_id":session_id,"session":session,"action":action});
    for recipient in recipients {
        // Correlation must be final at INSERT; immutable facts cannot be
        // rewritten by the journal context trigger afterward.
        let mut data = data.clone();
        if let Some(operation) = operation.filter(|_| recipient == owner) {
            let original:bool=c.query_row("SELECT NOT EXISTS(SELECT 1 FROM operations WHERE owner_id=?1 AND operation_id=?2 AND ledger_version=1) OR EXISTS(SELECT 1 FROM operations o JOIN operation_members m ON m.operation_record_id=o.record_id AND m.identity_id=o.owner_id JOIN participants p ON p.participant_id=m.participant_id AND p.membership_epoch=m.membership_epoch AND p.identity_id=m.identity_id AND p.active=1 WHERE o.owner_id=?1 AND o.operation_id=?2)",params![owner,operation],|r|r.get(0)).map_err(|_|Error::storage())?;
            if original {
                data["operation_id"] = json!(operation);
            }
        }
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,data_json,resource_revision,source_json) VALUES(?1,?2,?3,'session_control',?4,?5,?6)",params![format!("evt_{}",uuid::Uuid::new_v4().simple()),recipient,conversation,serde_json::to_string(&data).map_err(|_|Error::storage())?,session.revision,serde_json::to_string(&json!({"kind":"connect-session-control","correlation_id":session_id})).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
impl Store {
    pub fn session_view(&self, credential: &str, cid: &str, sid: &str) -> Result<SessionView> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = crate::identity::actor_from_connection(&tx, credential)?;
        actor.require("session_get")?;
        crate::audience::require(&tx, &actor, cid, ContentKind::Session, sid)?;
        view(&tx, cid, sid)
    }
    pub fn execute_session_control_with_peer(
        &self,
        credential: &str,
        request: Request,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<connect_protocol::transitions::OperationRecord> {
        self.execute_runtime_composed(
            credential,
            &connect_protocol::Operation::SessionControl {
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

#[cfg(test)]
#[path = "session_control_tests.rs"]
mod tests;
