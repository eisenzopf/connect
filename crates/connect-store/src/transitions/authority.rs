use super::*;
use connect_protocol::{membership::ContentKind, membership::MemberView};
use std::collections::BTreeMap;
pub(super) struct Sources {
    members: BTreeMap<String, (String, u64)>,
    connections: BTreeMap<String, u64>,
    endpoints: BTreeMap<String, u64>,
    additional_readers: Vec<crate::AuthorityScope>,
}
pub(super) fn admission_readers(
    c: &Connection,
    actor: &Actor,
    request: &TransitionRequest,
    store: &Store,
) -> Result<Vec<crate::AuthorityScope>> {
    let sources = admission(c, actor, request, store)?;
    let mut readers = sources
        .members
        .values()
        .map(|(identity, _)| crate::AuthorityScope::identity(identity, crate::AuthorityMode::Read))
        .collect::<Result<Vec<_>>>()?;
    readers.extend(sources.additional_readers);
    crate::AuthorityScope::merged(&readers)
}
pub(crate) fn tenant(c: &Connection, actor: &Actor) -> Result<String> {
    c.query_row(
        "SELECT tenant_id FROM identities WHERE id=?1 AND revoked=0",
        [&actor.identity_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(|_| Error::storage())?
    .ok_or_else(Error::forbidden)
}
fn member(c: &Connection, actor: &Actor, cid: &str, pid: &str) -> Result<MemberView> {
    let identity: Option<String> = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",
            params![pid, cid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let value = crate::audience::member(c, &identity.ok_or_else(Error::forbidden)?, cid, true)?;
    if !actor.permits_recipient(&value.identity_id) {
        return Err(Error::forbidden());
    }
    Ok(value)
}
fn add_member(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    pid: &str,
    sources: &mut Sources,
) -> Result<MemberView> {
    let m = member(c, actor, cid, pid)?;
    sources.members.insert(
        m.participant_id.clone(),
        (m.identity_id.clone(), m.membership_epoch),
    );
    Ok(m)
}
fn add_endpoint(c: &Connection, id: &str, owner: &str, sources: &mut Sources) -> Result<()> {
    let revision:Option<u64>=c.query_row("SELECT e.revision FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.tenant_id=e.tenant_id WHERE e.id=?1 AND e.identity_id=?2 AND e.verification_state='verified' AND i.revoked=0",params![id,owner],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    sources
        .endpoints
        .insert(id.into(), revision.ok_or_else(Error::forbidden)?);
    Ok(())
}
/// A current native activation or captured telephone compound move records `ready`.
/// A declared ready row alone is not a settled retained route. This recognizes
/// the original still-live local observation, without granting media authority.
pub(super) fn retained_ready(c: &Connection, id: &str, state: &str, store: &Store) -> Result<bool> {
    if ["active", "held"].contains(&state) {
        return Ok(true);
    }
    if state != "ready" {
        return Ok(false);
    }
    c.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id JOIN native_admission_sources o ON o.record_id=l.operation_record_id AND o.connection_id=v.id JOIN native_admission_sources n ON n.operation_record_id=o.record_id AND n.generation=v.generation AND n.attempt_runtime=l.runtime_epoch JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE v.id=?1 AND v.state='ready' AND v.observed_runtime_id=?2 AND v.readiness_json IS NOT NULL AND l.retired=0 AND o.state='completed' AND n.effect_started=1 AND e.state='confirmed') OR EXISTS(SELECT 1 FROM connections v JOIN telephone_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id AND t.runtime_epoch=l.runtime_epoch JOIN native_replacements n ON n.operation_record_id=t.operation_record_id AND n.candidate_connection_id=v.id AND n.generation=v.generation JOIN connection_offers o ON o.connection_id=v.id AND o.runtime_connection_id=n.candidate_runtime_id JOIN operations r ON r.record_id=t.operation_record_id AND r.state='completed' JOIN operation_effects e ON e.id=t.effect_id AND e.state='confirmed' WHERE v.id=?1 AND v.state='ready' AND v.observed_runtime_id=?2 AND v.readiness_json IS NOT NULL) OR EXISTS(SELECT 1 FROM connections v JOIN operations o ON o.connection_id=v.id JOIN initial_telephone_operations n ON n.operation_record_id=o.record_id JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE v.id=?1 AND v.state='ready' AND v.generation=1 AND v.observed_runtime_id=?2 AND v.readiness_json IS NOT NULL AND n.attempt_runtime=?2 AND n.signaling_started=1 AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL AND n.runtime_connection_id=json_extract(a.observation_json,'$.runtime_connection_id') AND n.sip_call_id=json_extract(a.observation_json,'$.call_id') AND json_extract(a.connected_json,'$.connection_id')=v.id AND json_extract(a.connected_json,'$.generation')=1 AND json_extract(a.connected_json,'$.participant_id')=v.participant_id AND json_extract(a.connected_json,'$.membership_epoch')=v.membership_epoch AND json_extract(a.connected_json,'$.endpoint_id')=v.endpoint_id)", params![id,store.runtime_epoch()], |r|r.get(0)).map_err(|_|Error::storage())
}
fn add_connection(
    c: &Connection,
    actor: &Actor,
    r: &TransitionRequest,
    id: &str,
    candidate: bool,
    sources: &mut Sources,
    store: &Store,
) -> Result<String> {
    let (pid,owner,epoch,gen,endpoint,state,runtime):(String,String,u64,u64,String,String,Option<String>)=c.query_row("SELECT participant_id,identity_id,membership_epoch,generation,endpoint_id,state,observed_runtime_id FROM connections WHERE id=?1 AND session_id=?2",params![id,r.session_id],|v|Ok((v.get(0)?,v.get(1)?,v.get(2)?,v.get(3)?,v.get(4)?,v.get(5)?,v.get(6)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let m = add_member(c, actor, &r.conversation_id, &pid, sources)?;
    if m.identity_id != owner
        || m.membership_epoch != epoch
        || runtime.as_deref() != Some(store.runtime_epoch())
        || if candidate {
            !["preparing", "ready", "active"].contains(&state.as_str())
        } else {
            !retained_ready(c, id, &state, store)?
        }
    {
        return Err(Error::forbidden());
    }
    add_endpoint(c, &endpoint, &owner, sources)?;
    sources.connections.insert(id.into(), gen);
    Ok(pid)
}
pub(super) fn admission(
    c: &Connection,
    actor: &Actor,
    r: &TransitionRequest,
    store: &Store,
) -> Result<Sources> {
    crate::audience::require_original(
        c,
        actor,
        &r.conversation_id,
        ContentKind::Session,
        &r.session_id,
    )?;
    let (revision,state,owner):(u64,String,String)=c.query_row("SELECT topology_revision,state,initiator_id FROM sessions WHERE id=?1 AND conversation_id=?2",params![r.session_id,r.conversation_id],|v|Ok((v.get(0)?,v.get(1)?,v.get(2)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let closed: bool = c
        .query_row(
            "SELECT closed FROM conversations WHERE id=?1",
            [&r.conversation_id],
            |v| v.get(0),
        )
        .map_err(|_| Error::storage())?;
    if revision != r.expected_revision
        && revision != super::consultation_source::admission_revision(c, actor, r, store)?
    {
        return Err(conflict());
    }
    if closed || !["active", "accepted", "ready"].contains(&state.as_str()) {
        return Err(Error::forbidden());
    }
    if matches!(r.intent, TransitionIntent::ConsultReturn { .. }) {
        return return_admission(c, actor, r, store);
    }
    let mut sources = Sources {
        members: BTreeMap::new(),
        connections: BTreeMap::new(),
        endpoints: BTreeMap::new(),
        additional_readers: vec![],
    };
    for id in [&actor.identity_id, &owner] {
        let m = crate::audience::member(c, id, &r.conversation_id, true)?;
        add_member(
            c,
            actor,
            &r.conversation_id,
            &m.participant_id,
            &mut sources,
        )?;
    }
    let retained = add_connection(c, actor, r, r.intent.retained(), false, &mut sources, store)?;
    let source = r
        .connection_id
        .as_deref()
        .map(|id| add_connection(c, actor, r, id, false, &mut sources, store))
        .transpose()?;
    match &r.intent {
        TransitionIntent::ConsultReturn { .. } => return Err(Error::forbidden()),
        TransitionIntent::Consult {
            private_participants,
            ..
        } => {
            // Completion preserves the private context after generic operation
            // reservations are released. Its explicit receivers, including the
            // held remote, cannot be borrowed by another consultation. Disjoint
            // native contexts remain independent. The floor is released only
            // by a definitive unsubmitted settlement or actual authorized return.
            let conflict: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_private_content_floors f
                 JOIN operations o ON o.record_id=f.operation_record_id
                 WHERE NOT(o.operation_id=?3 AND o.owner_id=?4)
                   AND (f.connection_id IN (?1,?2)
                     OR (f.is_candidate=0 AND EXISTS(
                       SELECT 1 FROM native_consultation_attempts n
                       WHERE n.operation_record_id=f.operation_record_id
                         AND json_extract(n.selection_json,'$.remote.connection_id') IN (?1,?2)))))",
                params![r.connection_id.as_deref(),r.intent.retained(),r.operation_id,actor.identity_id],
                |row|row.get(0),
            ).map_err(|_|Error::storage())?;
            if conflict {
                return Err(Error::new(
                    "conflict",
                    "An existing private context requires an authorized return or decision",
                ));
            }
            // The held remote can never become a member of its own excluded
            // consultation audience. An explicit primary must be private.
            if private_participants.contains(&retained)
                || source
                    .as_ref()
                    .is_some_and(|pid| !private_participants.contains(pid))
            {
                return Err(invalid());
            }
            for id in private_participants {
                add_member(c, actor, &r.conversation_id, id, &mut sources)?;
            }
        }
        TransitionIntent::Handoff {
            source_participant,
            target_participant,
            role_scope,
            expected_conversation_revision,
            ..
        } => {
            add_member(
                c,
                actor,
                &r.conversation_id,
                source_participant,
                &mut sources,
            )?;
            add_member(
                c,
                actor,
                &r.conversation_id,
                target_participant,
                &mut sources,
            )?;
            if source.as_ref().is_some_and(|pid| pid != source_participant) {
                return Err(Error::forbidden());
            }
            let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 WHERE sp.session_id=?1 AND sp.participant_id=?2 AND sp.state='present')",params![r.session_id,source_participant],|v|v.get(0)).map_err(|_|Error::storage())?;
            if !present {
                return Err(Error::forbidden());
            }
            if *role_scope == RoleScope::Conversation {
                let revision: u64 = c
                    .query_row(
                        "SELECT revision FROM conversations WHERE id=?1",
                        [&r.conversation_id],
                        |v| v.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if Some(revision) != *expected_conversation_revision {
                    return Err(conflict());
                }
            }
        }
        TransitionIntent::Replace { endpoint_id, .. } => {
            let pid = source.as_ref().ok_or_else(invalid)?;
            let m = member(c, actor, &r.conversation_id, pid)?;
            if m.identity_id != actor.identity_id {
                actor.require("connection.replace.any")?;
            }
            if let Some(ep) = endpoint_id {
                add_endpoint(c, ep, &m.identity_id, &mut sources)?;
            }
        }
    }
    if let Some(candidate) = r.intent.candidate() {
        let pid = add_connection(c, actor, r, candidate, true, &mut sources, store)?;
        match &r.intent {
            TransitionIntent::Handoff {
                target_participant, ..
            } if &pid != target_participant => return Err(Error::forbidden()),
            TransitionIntent::Replace { .. } if Some(pid) != source => {
                return Err(Error::forbidden())
            }
            _ => (),
        }
    }
    Ok(sources)
}
pub(super) fn freeze(c: &Connection, record: &str, sources: &Sources) -> Result<()> {
    for (pid, (identity, epoch)) in &sources.members {
        c.execute(
            "INSERT INTO operation_members VALUES(?1,?2,?3,?4)",
            params![record, pid, identity, epoch],
        )
        .map_err(|_| Error::storage())?;
    }
    for (id, gen) in &sources.connections {
        c.execute(
            "INSERT INTO operation_connections VALUES(?1,?2,?3)",
            params![record, id, gen],
        )
        .map_err(|_| Error::storage())?;
    }
    for (id, rev) in &sources.endpoints {
        c.execute(
            "INSERT INTO operation_endpoints VALUES(?1,?2,?3)",
            params![record, id, rev],
        )
        .map_err(|_| Error::storage())?;
    }
    Ok(())
}
pub(super) fn read(c: &Connection, actor: &Actor, row: &rows::Saved) -> Result<()> {
    if actor.identity_id != row.owner || tenant(c, actor)? != row.tenant {
        return Err(Error::forbidden());
    }
    if row.initial_telephone {
        return super::initial_telephone::read(c, actor, row);
    }
    if row.session_codecs {
        return super::session_codecs::read(c, actor, row);
    }
    if row.received_dtmf {
        return super::received_dtmf::read(c, actor, row);
    }
    if row.native_stream {
        return super::native_streams::read(c, actor, row);
    }
    if row.native {
        return super::native_offers::read(c, actor, row);
    }
    if row.control {
        let receipt = crate::operations::receipts::by_record(c, &row.record.record_id)?;
        return crate::operations::receipts::check_receipt_authority(c, actor, &receipt, None);
    }
    if !row.ledger {
        let target: Option<String> = c
            .query_row(
                "SELECT target_record_id FROM operation_decisions WHERE operation_record_id=?1",
                [&row.record.record_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if let Some(id) = target {
            actor.require(&row.record.command_type)?;
            let target = rows::by_record(c, &id)?;
            return if row.record.command_type == "operation.cleanup" {
                super::cleanup::authorize(c, actor, &target)
            } else {
                read(c, actor, &target)
            };
        }
        if row.record.command_type.starts_with("conversation.") {
            return Err(Error::forbidden());
        }
        actor.require("admin")?;
        if !row.tombstoned {
            local_result(c, actor, row)?;
        }
        return Ok(());
    }
    actor.require(&row.record.command_type)?;
    let cid = row
        .record
        .conversation_id
        .as_deref()
        .ok_or_else(Error::storage)?;
    let own = crate::audience::member(c, &actor.identity_id, cid, true)?;
    let original:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND identity_id=?4)",params![row.record.record_id,own.participant_id,own.membership_epoch,actor.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !original {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        actor,
        cid,
        ContentKind::Session,
        row.record
            .session_id
            .as_deref()
            .ok_or_else(Error::storage)?,
    )?;
    let mut st=c.prepare("SELECT m.identity_id,i.tenant_id FROM operation_members m JOIN identities i ON i.id=m.identity_id WHERE m.operation_record_id=?1").map_err(|_|Error::storage())?;
    for value in st
        .query_map([&row.record.record_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| Error::storage())?
    {
        let (id, t) = value.map_err(|_| Error::storage())?;
        if t != row.tenant || !actor.permits_recipient(&id) {
            return Err(Error::forbidden());
        }
    }
    Ok(())
}
pub(super) fn execution(
    c: &Connection,
    actor: &Actor,
    row: &rows::Saved,
    store: &Store,
) -> Result<()> {
    read(c, actor, row)?;
    if !row.ledger || row.tombstoned {
        return Err(Error::forbidden());
    }
    if row.record.expires_at.is_none_or(|v| v <= crate::now()) || row.record.cancel_requested {
        return Err(Error::forbidden());
    }
    if row.initial_telephone {
        return Err(Error::forbidden()); // Only the owned telephone attempt may execute.
    }
    let request = rows::request(row)?;
    let current = admission(c, actor, &request, store)?;
    let mut st=c.prepare("SELECT participant_id,identity_id,membership_epoch FROM operation_members WHERE operation_record_id=?1").map_err(|_|Error::storage())?;
    for value in st
        .query_map([&row.record.record_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u64>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
    {
        let (id, identity, epoch) = value.map_err(|_| Error::storage())?;
        if current.members.get(&id) != Some(&(identity, epoch)) {
            return Err(Error::forbidden());
        }
    }
    let mismatch:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operation_connections s JOIN connections v ON v.id=s.connection_id WHERE s.operation_record_id=?1 AND s.generation!=v.generation AND ?2=0) OR EXISTS(SELECT 1 FROM operation_endpoints s JOIN endpoints e ON e.id=s.endpoint_id WHERE s.operation_record_id=?1 AND (s.revision!=e.revision OR e.verification_state!='verified'))",params![row.record.record_id,row.native_return],|r|r.get(0)).map_err(|_|Error::storage())?;
    if mismatch {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn local_result(
    c: &Connection,
    actor: &Actor,
    row: &rows::Saved,
) -> Result<Option<serde_json::Value>> {
    if row.session_codecs {
        super::session_codecs::read(c, actor, row)?;
        return Ok(row.record.result.clone());
    }
    if row.received_dtmf {
        super::received_dtmf::read(c, actor, row)?;
        return Ok(row.record.result.clone());
    }
    if row.native_stream {
        super::native_streams::read(c, actor, row)?;
        return Ok(row.record.result.clone());
    }
    if row.native {
        super::native_offers::read(c, actor, row)?;
        return Ok(row.record.result.clone());
    }
    if row.control {
        let receipt = crate::operations::receipts::by_record(c, &row.record.record_id)?;
        crate::operations::receipts::check_receipt_authority(c, actor, &receipt, None)?;
        return crate::operations::receipts::view_result(c, actor, &receipt)?
            .ok()
            .map(|v| serde_json::to_value(v).map_err(|_| Error::storage()))
            .transpose();
    }
    let decision: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM operation_decisions WHERE operation_record_id=?1)",
            [&row.record.record_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if decision {
        return Ok(row.record.result.clone());
    }
    let Some(result) = row.record.result.clone() else {
        return Ok(None);
    };
    crate::provisioning::ledger_result(c, actor, &row.tenant, result).map(Some)
}

/// Known departed slots are metadata, never substitute media grants.
fn return_admission(
    c: &Connection,
    actor: &Actor,
    r: &TransitionRequest,
    store: &Store,
) -> Result<Sources> {
    let TransitionIntent::ConsultReturn {
        consultation_record_id,
        retained_connid,
    } = &r.intent
    else {
        return Err(Error::forbidden());
    };
    let target = super::native_returns::target(c, store, consultation_record_id)?;
    if target.conversation != r.conversation_id
        || target.session != r.session_id
        || r.connection_id.as_deref() != Some(target.primary.as_str())
        || retained_connid != &target.remote
    {
        return Err(Error::forbidden());
    }
    let mut sources = Sources {
        members: BTreeMap::new(),
        connections: BTreeMap::new(),
        endpoints: BTreeMap::new(),
        additional_readers: vec![],
    };
    let own = crate::audience::member(c, &actor.identity_id, &r.conversation_id, true)?;
    let mut st=c.prepare("SELECT participant_id,identity_id,membership_epoch FROM operation_members WHERE operation_record_id=?1").map_err(|_|Error::storage())?;
    for row in st
        .query_map([consultation_record_id], |v| {
            Ok((
                v.get::<_, String>(0)?,
                v.get::<_, String>(1)?,
                v.get::<_, u64>(2)?,
            ))
        })
        .map_err(|_| Error::storage())?
    {
        let (pid, identity, epoch) = row.map_err(|_| Error::storage())?;
        if !actor.permits_recipient(&identity) {
            return Err(Error::forbidden());
        }
        sources.members.insert(pid, (identity, epoch));
    }
    if sources.members.get(&own.participant_id)
        != Some(&(actor.identity_id.clone(), own.membership_epoch))
    {
        return Err(Error::forbidden());
    }
    let primary_owner: String = c
        .query_row(
            "SELECT identity_id FROM connections WHERE id=?1",
            [&target.primary],
            |v| v.get(0),
        )
        .map_err(|_| Error::storage())?;
    if actor.identity_id != primary_owner && actor.identity_id != target.owner {
        return Err(Error::forbidden());
    }
    let original = target.selection["private"]
        .as_array()
        .ok_or_else(Error::storage)?;
    if original.len() != 2
        || !original
            .iter()
            .any(|v| v["connection_id"] == target.primary)
        || !original
            .iter()
            .any(|v| v["connection_id"] == target.candidate)
        || target.selection["remote"]["connection_id"] != target.remote
    {
        return Err(Error::forbidden());
    }
    for slot in original
        .iter()
        .chain(std::iter::once(&target.selection["remote"]))
    {
        let id = slot["connection_id"].as_str().ok_or_else(Error::storage)?;
        let generation = slot["generation"].as_u64().ok_or_else(Error::storage)?;
        let current:Option<(String,String,u64,String,u64,String,Option<String>)>=c.query_row("SELECT participant_id,identity_id,membership_epoch,endpoint_id,generation,state,observed_runtime_id FROM connections WHERE id=?1 AND session_id=?2",params![id,r.session_id],|v|Ok((v.get(0)?,v.get(1)?,v.get(2)?,v.get(3)?,v.get(4)?,v.get(5)?,v.get(6)?))).optional().map_err(|_|Error::storage())?;
        let (pid, identity, epoch, endpoint, current_generation, state, runtime) =
            current.ok_or_else(Error::forbidden)?;
        if slot["participant_id"] != pid
            || slot["membership_epoch"] != epoch
            || sources.members.get(&pid) != Some(&(identity.clone(), epoch))
        {
            return Err(Error::forbidden());
        }
        let live = matches!(state.as_str(), "ready" | "active")
            && current_generation == generation
            && runtime.as_deref() == Some(store.runtime_epoch());
        if live {
            if slot.get("source_kind").is_some() {
                // Returning this captured private candidate closes its own
                // call; it does not restore any of its media programmes.
                let purpose =
                    if id == target.candidate && slot["source_kind"] == "private-initial-sip" {
                        crate::connection_control::private_consultation::Purpose::Presence
                    } else {
                        crate::connection_control::private_consultation::Purpose::Media
                    };
                let (readers, _) = store.sip_audience_snapshot_facts_for(
                    c,
                    slot,
                    &r.conversation_id,
                    &r.session_id,
                    purpose,
                )?;
                sources.additional_readers.extend(readers);
            } else {
                let mapping:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connection_offers o JOIN native_connection_lifetimes l ON l.connection_id=o.connection_id AND l.generation=?3 AND l.runtime_epoch=?4 AND l.retired=0 WHERE o.connection_id=?1 AND o.runtime_connection_id=?2)",params![id,slot["runtime_connection_id"].as_str().ok_or_else(Error::storage)?,generation,store.runtime_epoch()],|v|v.get(0)).map_err(|_|Error::storage())?;
                if !mapping {
                    return Err(Error::forbidden());
                }
            }
            let member = crate::audience::member(c, &identity, &r.conversation_id, true)?;
            if member.participant_id != pid || member.membership_epoch != epoch {
                return Err(Error::forbidden());
            }
            add_endpoint(c, &endpoint, &identity, &mut sources)?;
        } else {
            let ended = if slot.get("source_kind").is_some() {
                Store::recorded_sip_departure(c, slot, store.runtime_epoch())?
            } else {
                c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes WHERE connection_id=?1 AND generation=?2 AND runtime_epoch=?3 AND retired=1)",params![id,generation,store.runtime_epoch()],|v|v.get(0)).map_err(|_|Error::storage())?
            };
            if !ended || !matches!(state.as_str(), "retired" | "failed") {
                return Err(Error::new(
                    "not-ready",
                    "Original private context requires a current native lifetime observation",
                ));
            }
        }
        sources.connections.insert(id.into(), current_generation);
    }
    Ok(sources)
}
