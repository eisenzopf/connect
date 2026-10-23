use super::*;
use connect_protocol::membership::ContentKind;
struct Row {
    sequence: i64,
    id: String,
    kind: String,
    message: Option<String>,
    call: Option<String>,
    session: Option<String>,
    artifact: Option<String>,
    raw: Option<String>,
    revision: u64,
    provenance: String,
    operation: Option<String>,
    observed_at: i64,
    source_kind: Option<String>,
    source_id: Option<String>,
    participant: String,
    epoch: u64,
}
pub(super) fn page(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    epoch: u64,
    after: i64,
    maximum: i64,
    size: usize,
) -> Result<(Vec<JournalEvent>, i64, bool)> {
    let mut statement=c.prepare(&format!("SELECT sequence,id,kind,message_id,call_json,session_json,artifact_json,data_json,resource_revision,operation_record_id,observed_at,source_kind,source_id,recipient_participant_id,recipient_epoch,source_json FROM visible_journal WHERE recipient=?1 AND sequence>?2 AND conversation_id=?3 AND sequence<=?5 AND (source_owner IS NULL OR source_owner=?1 OR EXISTS(SELECT 1 FROM json_each(?4) WHERE value IN(source_owner,'*'))) AND {} ORDER BY sequence",audience::journal_scope(4))).map_err(|_|Error::storage())?;
    let rows = statement
        .query_map(
            params![
                actor.identity_id,
                after,
                cid,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?,
                maximum
            ],
            |r| {
                Ok(Row {
                    sequence: r.get(0)?,
                    id: r.get(1)?,
                    kind: r.get(2)?,
                    message: r.get(3)?,
                    call: r.get(4)?,
                    session: r.get(5)?,
                    artifact: r.get(6)?,
                    raw: r.get(7)?,
                    revision: r.get(8)?,
                    operation: r.get(9)?,
                    observed_at: r.get(10)?,
                    source_kind: r.get(11)?,
                    source_id: r.get(12)?,
                    participant: r.get(13)?,
                    epoch: r.get(14)?,
                    provenance: r.get(15)?,
                })
            },
        )
        .map_err(|_| Error::storage())?;
    let mut events = vec![];
    let mut through = after;
    let mut bytes = 0;
    for row in rows {
        let row = row.map_err(|_| Error::storage())?;
        let sequence = row.sequence;
        // Projection applies every capability/source check before counting a
        // delivered row. Hidden compound/operation rows consume no public limit.
        let Some(event) = project(c, actor, cid, epoch, row)? else {
            through = sequence;
            continue;
        };
        let length = serde_json::to_vec(&event)
            .map_err(|_| Error::storage())?
            .len();
        if events.len() == size || bytes + length > connect_protocol::MAX_PAGE_BYTES - 8192 {
            if events.is_empty() {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Journal event exceeds encoded byte budget",
                ));
            }
            return Ok((events, through, true));
        }
        bytes += length;
        through = sequence;
        events.push(event);
    }
    Ok((events, maximum, false))
}
fn project(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    epoch: u64,
    row: Row,
) -> Result<Option<JournalEvent>> {
    let data: serde_json::Value = row
        .raw
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| Error::storage())?
        .unwrap_or(serde_json::Value::Null);
    let mut operation_id = data
        .get("operation_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let mut session_id = None;
    let mut participant_id = data
        .get("participant_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let connection_id = data
        .get("connection_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let operation_kind = row.kind.starts_with("operation_");
    if let Some(id) = &connection_id {
        if !crate::connection_control::private_consultation::visible(c, actor, id)? {
            return Ok(None);
        }
    }
    let payload = if row.kind == "conversation_closed" {
        if !actor.permits("conversation.get") {
            return Ok(None);
        }
        let fact = c.query_row("SELECT conversation_id,reason_code,reason,closed_at FROM conversation_closure_facts WHERE conversation_id=?1",[cid],|r| Ok(connect_protocol::events::ConversationClosed { conversation_id:r.get(0)?,reason_code:r.get(1)?,reason:r.get(2)?,closed_at:r.get(3)? })).map_err(|_|Error::storage())?;
        if fact.closed_at > row.observed_at {
            return Err(Error::storage());
        }
        JournalData::ConversationClosed(fact)
    } else if matches!(row.kind.as_str(), "record_ready" | "record_unavailable") {
        let value: RecordAvailability =
            serde_json::from_value(data["record"].clone()).map_err(|_| Error::storage())?;
        let source_bound = match &value {
            RecordAvailability::Ready { uuid, .. } => {
                row.source_kind.as_deref() == Some("record")
                    && row.source_id.as_deref() == Some(uuid.as_str())
            }
            RecordAvailability::Unavailable { session_id } => {
                row.source_kind.as_deref() == Some("session")
                    && row.source_id.as_deref() == Some(session_id.as_str())
            }
            RecordAvailability::Pending { .. } => false,
        };
        if !matches!(
            (row.kind.as_str(), &value),
            ("record_ready", RecordAvailability::Ready { .. })
                | ("record_unavailable", RecordAvailability::Unavailable { .. })
        ) || !source_bound
        {
            return Err(Error::storage());
        }
        if crate::records::read::availability(c, actor, cid, value.session_id())?.as_ref()
            != Some(&value)
        {
            return Ok(None);
        }
        session_id = Some(value.session_id().into());
        JournalData::Record(value)
    } else if row.kind == "provider_voice" {
        let value: connect_protocol::events::ProviderSessionVoiceView = serde_json::from_value(
            data.get("provider_call")
                .cloned()
                .ok_or_else(Error::storage)?,
        )
        .map_err(|_| Error::storage())?;
        if value.conversation_id != cid
            || row.source_kind.as_deref() != Some("session")
            || row.source_id.as_deref() != Some(value.session_id.as_str())
            || data.get("resource_id").and_then(|v| v.as_str()) != Some(value.id.as_str())
            || value.local_media
            || value.remote_identity_verified
        {
            return Err(Error::storage());
        }
        match crate::transitions::provider_sessions::view_for_actor(c, actor, &value.id) {
            Ok(now)
                if now.conversation_id == cid
                    && now.participant_id == value.participant_id
                    && now.membership_epoch == value.membership_epoch
                    && now.direction == value.direction =>
            {
                ()
            }
            Ok(_) => return Err(Error::storage()),
            Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized") => return Ok(None),
            Err(e) => return Err(e),
        }
        session_id = Some(value.session_id.clone());
        participant_id = Some(value.participant_id.clone());
        JournalData::ProviderCall(value)
    } else if row.kind == "connection_preparing" {
        let value: connect_protocol::connection_control::PreparedConnection =
            serde_json::from_value(data.get("connection").cloned().ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?;
        if value.conversation_id != cid
            || row.source_kind.as_deref() != Some("session")
            || row.source_id.as_deref() != Some(value.session_id.as_str())
            || connection_id.as_deref() != Some(value.connection_id.as_str())
            || participant_id.as_deref() != Some(value.participant_id.as_str())
            || value.state != "preparing"
            || value.generation != 1
            || value.revision != 0
        {
            return Err(Error::storage());
        }
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0 WHERE p.participant_id=?1 AND p.conversation_id=?2 AND p.membership_epoch=?3 AND p.active=1)",rusqlite::params![value.participant_id,cid,value.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current || !actor.permits("session_get") {
            return Ok(None);
        }
        session_id = Some(value.session_id.clone());
        JournalData::ConnectionPrepared(value)
    } else if matches!(row.kind.as_str(), "connection_retired" | "connection_ready") {
        let value: connect_protocol::events::ConnectionState =
            serde_json::from_value(data.get("connection").cloned().ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?;
        if row.source_kind.as_deref() != Some("session")
            || row.source_id.as_deref() != Some(value.session_id.as_str())
            || connection_id.as_deref() != Some(value.connection_id.as_str())
            || participant_id.as_deref() != Some(value.participant_id.as_str())
            || value.state
                != if row.kind == "connection_ready" {
                    "ready"
                } else {
                    "retired"
                }
            || value.generation < if row.kind == "connection_ready" { 1 } else { 2 }
            || value.revision == 0
        {
            return Err(Error::storage());
        }
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0 WHERE p.participant_id=?1 AND p.conversation_id=?2 AND p.membership_epoch=?3 AND p.active=1)",rusqlite::params![value.participant_id,cid,value.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current || !actor.permits("session_get") {
            return Ok(None);
        }
        session_id = Some(value.session_id.clone());
        if row.kind == "connection_ready" {
            JournalData::ConnectionReady(value)
        } else {
            JournalData::ConnectionRetired(value)
        }
    } else if row.kind == "native_data_write" {
        if !actor.permits("operation.get") || !actor.permits("history") {
            return Ok(None);
        }
        let record = match crate::transitions::snapshot_record(
            c,
            actor,
            row.operation.as_deref().ok_or_else(Error::storage)?,
        ) {
            Ok(value) => value,
            Err(e) if matches!(e.code.as_str(), "forbidden" | "gone") => return Ok(None),
            Err(e) => return Err(e),
        };
        let value: connect_protocol::native_data::WriteObservation =
            serde_json::from_value(data["native_data_write"].clone())
                .map_err(|_| Error::storage())?;
        if row.source_kind.as_deref() != Some("message")
            || row.source_id.as_deref() != Some(value.message_id.as_str())
            || connection_id.as_deref() != Some(value.target_connection_id.as_str())
            || participant_id.as_deref() != Some(value.participant_id.as_str())
            || record.connection_id.as_deref() != Some(value.source_connection_id.as_str())
            || row.observed_at != value.observed_at
        {
            return Err(Error::storage());
        }
        operation_id = Some(record.operation_id);
        session_id = record.session_id;
        JournalData::NativeDataWrite(value)
    } else if operation_kind {
        if !actor.permits("operation.get") {
            return Ok(None);
        }
        let Some(id) = &row.operation else {
            return Ok(None);
        };
        let record = match crate::transitions::snapshot_record(c, actor, id) {
            Ok(r) => r,
            Err(e) if matches!(e.code.as_str(), "forbidden" | "gone") => return Ok(None),
            Err(e) => return Err(e),
        };
        // A historical event carries its own revision and correlation, while the
        // queried operation is current. Do not relabel an old event as a new state.
        let state = data
            .get("state")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| Error::storage())?
            .ok_or_else(Error::storage)?;
        operation_id = Some(record.operation_id.clone());
        session_id = record.session_id;
        JournalData::Operation {
            operation_id: record.operation_id,
            record_id: record.record_id,
            state,
        }
    } else if matches!(
        row.kind.as_str(),
        "session_participant_joined" | "session_participant_left"
    ) {
        let sid = data["session_id"].as_str().ok_or_else(Error::storage)?;
        let id = data["participant_fact_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        let fact = match crate::session_participant_lifecycle::read(c, actor, cid, sid, id, false) {
            Ok(fact) => fact,
            Err(e) if e.code == "forbidden" => return Ok(None),
            Err(e) => return Err(e),
        };
        let (fact_id, session, pid, _, kind) = crate::session_participant_lifecycle::fields(&fact)?;
        if data["participant_fact"] != serde_json::to_value(&fact).map_err(|_| Error::storage())?
            || fact_id != id
            || kind != row.kind
            || session.revision != row.revision
            || session.id != sid
            || session.conversation_id != cid
            || participant_id.as_deref() != Some(pid)
            || row.source_kind.as_deref() != Some("session")
            || row.source_id.as_deref() != Some(sid)
            || connection_id.is_some()
        {
            return Err(Error::storage());
        }
        session_id = Some(sid.into());
        fact
    } else if row.kind == "session_started" {
        let sid = data
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(Error::storage)?;
        let fact = match crate::session_lifecycle::read(c, actor, cid, sid, false) {
            Ok(fact) => fact,
            Err(e) if e.code == "forbidden" => return Ok(None),
            Err(e) => return Err(e),
        };
        if data["session_started"] != serde_json::to_value(&fact).map_err(|_| Error::storage())?
            || fact.session.revision != row.revision
            || row.source_kind.as_deref() != Some("session")
            || row.source_id.as_deref() != Some(sid)
            || fact.session.conversation_id != cid
            || connection_id.is_some()
            || operation_id.is_some()
            || participant_id.is_some()
        {
            return Err(Error::storage());
        }
        session_id = Some(sid.into());
        JournalData::SessionStarted(fact)
    } else if row.kind == "session_control" {
        if !actor.permits("session_get") {
            return Ok(None);
        }
        let session: connect_protocol::session_control::SessionView =
            serde_json::from_value(data["session"].clone()).map_err(|_| Error::storage())?;
        if session.conversation_id != cid
            || row.source_kind.as_deref() != Some("session")
            || row.source_id.as_deref() != Some(session.id.as_str())
        {
            return Err(Error::storage());
        }
        session_id = Some(session.id.clone());
        match terminal_fact(
            c,
            cid,
            &session.id,
            &row.kind,
            row.revision,
            row.observed_at,
        )? {
            Some(fact) if session.state == "ended" => {
                if fact.session != session {
                    return Err(Error::storage());
                }
                JournalData::SessionEnded(fact)
            }
            _ => JournalData::SessionControl(session),
        }
    } else if let Some(receipt) = data.get("receipt") {
        if !actor.permits("history") {
            return Ok(None);
        }
        JournalData::Receipt(serde_json::from_value(receipt.clone()).map_err(|_| Error::storage())?)
    } else if let Some(id) = row.message {
        if !actor.permits("history") {
            return Ok(None);
        }
        JournalData::Message(crate::messages::project(c, actor, cid, &id, None, None)?)
    } else if let Some(raw) = row.session {
        if !actor.permits("session_get") {
            return Ok(None);
        }
        let session: connect_protocol::Session =
            serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        if !actor.permits_recipient(&session.initiator_id)
            || !actor.permits_recipient(&session.participant_id)
        {
            return Ok(None);
        }
        session_id = Some(session.id.clone());
        match terminal_fact(
            c,
            cid,
            &session.id,
            &row.kind,
            row.revision,
            row.observed_at,
        )? {
            Some(fact) if session.state == "ended" => JournalData::SessionEnded(fact),
            _ => JournalData::Session(session),
        }
    } else if let Some(raw) = row.artifact {
        if !actor.permits("artifact_get") {
            return Ok(None);
        }
        JournalData::Artifact(serde_json::from_str(&raw).map_err(|_| Error::storage())?)
    } else if let Some(raw) = row.call {
        if !actor.permits("call_status") {
            return Ok(None);
        }
        JournalData::Call(serde_json::from_str(&raw).map_err(|_| Error::storage())?)
    } else if row.kind.starts_with("conversation_participant_") {
        if !actor.permits("conversation.get") {
            return Ok(None);
        }
        JournalData::Membership {
            participant_id: participant_id.clone(),
            identity_id: data
                .get("identity_id")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            role: data.get("role").and_then(|v| v.as_str()).map(str::to_owned),
            active: data.get("active").and_then(|v| v.as_bool()),
            membership_epoch: data.get("membership_epoch").and_then(|v| v.as_u64()),
            removal_reason: data
                .get("removal_reason")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        }
    } else {
        let permission = row
            .source_kind
            .as_deref()
            .and_then(content_kind)
            .map(audience::read_scope)
            .unwrap_or("conversation.get");
        if !actor.permits(permission) {
            return Ok(None);
        }
        JournalData::Changed {
            resource_kind: row.source_kind.clone(),
            resource_id: row.source_id.clone(),
        }
    };
    if participant_id.is_none() {
        participant_id = match &payload {
            JournalData::Message(value) => value.sender_participant_id.clone(),
            JournalData::Receipt(value) => Some(value.participant_id.clone()),
            _ => None,
        };
    }
    // Visible_journal includes all compound audience intersections; this extra
    // source check keeps projection robust to future view extension mistakes.
    if let (Some(kind), Some(id)) = (row.source_kind.as_deref(), row.source_id.as_deref()) {
        let kind = content_kind(kind);
        if let Some(kind) = kind {
            if !audience::visible(c, actor, cid, kind, id)? {
                return Ok(None);
            }
        }
    }
    Ok(Some(JournalEvent {
        event_id: row.id.clone(),
        cursor: position(c, actor, cid, epoch, row.sequence)?,
        conversation_id: cid.into(),
        session_id,
        connection_id,
        participant_id,
        operation_id,
        kind: row.kind,
        revision: row.revision,
        source: serde_json::from_str(&row.provenance).map_err(|_| Error::storage())?,
        observed_at: row.observed_at,
        audience: vec![connect_protocol::messages::MessageRecipient {
            participant_id: row.participant,
            membership_epoch: row.epoch,
        }],
        data: payload,
    }))
}

fn content_kind(value: &str) -> Option<ContentKind> {
    serde_json::from_value(serde_json::Value::String(value.into())).ok()
}

fn terminal_fact(
    c: &Connection,
    cid: &str,
    sid: &str,
    kind: &str,
    revision: u64,
    observed_at: i64,
) -> Result<Option<connect_protocol::events::SessionEnded>> {
    use rusqlite::OptionalExtension;
    let value:Option<String>=c.query_row("SELECT fact_json FROM session_terminal_facts WHERE conversation_id=?1 AND session_id=?2 AND journal_kind=?3 AND resource_revision=?4",params![cid,sid,kind,revision],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    value
        .map(|json| {
            let fact: connect_protocol::events::SessionEnded =
                serde_json::from_str(&json).map_err(|_| Error::storage())?;
            if fact.session.id != sid
                || fact.session.conversation_id != cid
                || fact.session.state != "ended"
                || fact.session.revision != revision
                || fact.ended_at <= 0
                || fact.ended_at > observed_at
                || fact.reason_code > 65535
                || fact.reason.len() > 512
                || fact.reason.chars().any(char::is_control)
                || fact.by_participant.is_some() != fact.by_membership_epoch.is_some()
            {
                return Err(Error::storage());
            }
            Ok(fact)
        })
        .transpose()
}
