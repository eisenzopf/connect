use crate::{Actor, Store};
use connect_protocol::{ContextDelivery, ContextItem, ContextView, Error, Result};
use rusqlite::{params, Connection};

mod delivery;
mod session;

const MAX_CONTEXT_BYTES: usize = 32 * 1024;

// Default provider memory has no private purpose. A private destination must
// prove that every original participant of its current private audience can
// read the historical source; sharing a logical AI identity alone is insufficient.
fn private_export_refusal(kind: &str, id: &str, cid: &str, purpose: &str) -> String {
    format!("({purpose} IS NULL OR NOT EXISTS(SELECT 1 FROM native_consultation_legs target JOIN operations target_operation ON target_operation.record_id=target.operation_record_id WHERE target.operation_record_id={purpose} AND target_operation.conversation_id={cid} AND json_array_length(target.audience_json)=2 AND (target_operation.state IN ('accepted','preparing','ready','committing') OR (target_operation.state='completed' AND EXISTS(SELECT 1 FROM native_private_lifetimes life WHERE life.operation_record_id=target.operation_record_id AND life.restored_at IS NULL))) AND NOT EXISTS(SELECT 1 FROM json_each(target.audience_json) receiver WHERE NOT EXISTS(SELECT 1 FROM content_audiences allowed JOIN participants viewer ON viewer.participant_id=allowed.participant_id AND viewer.membership_epoch=allowed.membership_epoch AND viewer.active=1 JOIN identities identity ON identity.id=viewer.identity_id AND identity.revoked=0 WHERE allowed.kind={kind} AND allowed.id={id} AND allowed.conversation_id={cid} AND viewer.identity_id=json_extract(receiver.value,'$[0]') AND viewer.participant_id=json_extract(receiver.value,'$[1]') AND viewer.membership_epoch=json_extract(receiver.value,'$[2]')))))")
}

pub(crate) fn authorize_export_event(
    c: &Connection,
    actor: &Actor,
    peer: &str,
    delegate: Option<&Actor>,
    cid: &str,
    sequence: u64,
) -> Result<()> {
    authorize_private_export_event(c, actor, peer, delegate, cid, sequence, None)
}
pub(crate) fn authorize_private_export_event(
    c: &Connection,
    actor: &Actor,
    peer: &str,
    delegate: Option<&Actor>,
    cid: &str,
    sequence: u64,
    private_record: Option<&str>,
) -> Result<()> {
    authorize_private_export_events(c, actor, peer, delegate, cid, &[sequence], private_record)
}
/// The same current event/source checks in one snapshot. Journal entries can
/// reference the same resource many times; checking that resource once does not
/// turn historical projection JSON into a cache of authorization.
pub(crate) fn authorize_private_export_events(
    c: &Connection,
    actor: &Actor,
    peer: &str,
    delegate: Option<&Actor>,
    cid: &str,
    sequences: &[u64],
    private_record: Option<&str>,
) -> Result<()> {
    let selected = serde_json::to_string(sequences).map_err(|_| Error::storage())?;
    let private_floor = private_export_refusal("s.kind", "s.id", "s.conversation_id", "?2");
    let wrong:bool=c.query_row(&format!("SELECT EXISTS(SELECT 1 FROM journal_sources s LEFT JOIN private_content_sources p ON p.kind=s.kind AND p.source_id=s.id AND p.conversation_id=s.conversation_id WHERE s.sequence IN (SELECT value FROM json_each(?1)) AND (?2 IS NOT NULL OR p.kind IS NOT NULL) AND {private_floor})"),params![selected,private_record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if wrong {
        return Err(Error::forbidden());
    }
    let mut statement=c.prepare("WITH wanted AS (SELECT value AS sequence FROM json_each(?1)) SELECT s.kind,s.id FROM wanted LEFT JOIN visible_journal j ON j.sequence=wanted.sequence AND j.recipient=?2 AND j.conversation_id=?3 LEFT JOIN journal_sources s ON s.sequence=j.sequence").map_err(|_|Error::storage())?;
    let mut rows = std::collections::BTreeSet::new();
    let mut cursor = statement
        .query(params![selected, actor.identity_id, cid])
        .map_err(|_| Error::storage())?;
    while let Some(row) = cursor.next().map_err(|_| Error::storage())? {
        let kind: Option<String> = row.get(0).map_err(|_| Error::storage())?;
        let id: Option<String> = row.get(1).map_err(|_| Error::storage())?;
        rows.insert((
            kind.ok_or_else(Error::forbidden)?,
            id.ok_or_else(Error::forbidden)?,
        ));
    }
    for (kind, id) in rows {
        let kind = serde_json::from_value(serde_json::Value::String(kind))
            .map_err(|_| Error::storage())?;
        crate::audience::require(c, actor, cid, kind, &id)?;
        if !crate::audience::visible_to_identity(c, peer, cid, kind, &id)? {
            return Err(Error::forbidden());
        }
        if let Some(delegate) = delegate {
            delegate.require("context_view")?;
            crate::audience::require(c, delegate, cid, kind, &id)?;
        }
    }
    Ok(())
}

pub(crate) fn view(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
) -> Result<ContextView> {
    view_for(store, c, actor, conversation_id, limit, None)
}

pub(crate) fn view_for(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
    recipient: Option<&str>,
) -> Result<ContextView> {
    view_for_scope(store, c, actor, conversation_id, limit, recipient, None)
}

pub(crate) fn view_for_scope(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
    recipient: Option<&str>,
    delegate: Option<&Actor>,
) -> Result<ContextView> {
    view_for_scope_at(
        store,
        c,
        actor,
        conversation_id,
        limit,
        recipient,
        delegate,
        None,
    )
}

pub(crate) fn view_for_scope_at(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
    recipient: Option<&str>,
    delegate: Option<&Actor>,
    through: Option<u64>,
) -> Result<ContextView> {
    view_for_private_scope_at(
        store,
        c,
        actor,
        conversation_id,
        limit,
        recipient,
        delegate,
        through,
        None,
    )
}
pub(crate) fn view_for_private_scope_at(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
    recipient: Option<&str>,
    delegate: Option<&Actor>,
    through: Option<u64>,
    private_record: Option<&str>,
) -> Result<ContextView> {
    let audience: Option<Vec<(String,String,u64)>> = private_record.map(|record| {
        let raw: String = c.query_row("SELECT audience_json FROM native_consultation_legs WHERE operation_record_id=?1",[record],|r|r.get(0)).map_err(|_|Error::forbidden())?;
        let members: Vec<(String,String,u64)> = serde_json::from_str(&raw).map_err(|_|Error::storage())?;
        if members.len()!=2 {return Err(Error::forbidden());}
        Ok(members)
    }).transpose()?;
    view_for_audience_scope_at(
        store,
        c,
        actor,
        conversation_id,
        limit,
        recipient,
        delegate,
        through,
        private_record,
        audience.as_deref(),
    )
}

pub(crate) fn view_for_audience_scope_at(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
    recipient: Option<&str>,
    delegate: Option<&Actor>,
    through: Option<u64>,
    private_record: Option<&str>,
    audience: Option<&[(String, String, u64)]>,
) -> Result<ContextView> {
    crate::audience::member(c, &actor.identity_id, conversation_id, true)?;
    if delegate.is_some_and(|a| !a.permits("context_view")) {
        return Ok(ContextView {
            conversation_id: conversation_id.into(),
            watermark: 0,
            items: vec![],
            omitted: 0,
            deliveries: vec![],
            deliveries_omitted: 0,
        });
    }
    let recipient_scope = delegate
        .map(|a| serde_json::to_string(&a.scope.recipients))
        .transpose()
        .map_err(|_| Error::storage())?;
    let recipients =
        serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?;
    let scope = format!("recipient=?1 AND conversation_id=?2 AND (source_owner IS NULL OR source_owner=?1 OR EXISTS(SELECT 1 FROM json_each(?3) WHERE value IN(source_owner,'*'))) AND {} AND (?5 IS NULL OR NOT EXISTS(SELECT 1 FROM journal_sources export WHERE export.sequence=visible_journal.sequence AND NOT EXISTS(SELECT 1 FROM content_audiences a JOIN participants viewer ON viewer.participant_id=a.participant_id AND viewer.membership_epoch=a.membership_epoch AND viewer.active=1 JOIN identities i ON i.id=viewer.identity_id AND i.revoked=0 WHERE a.kind=export.kind AND a.id=export.id AND a.conversation_id=export.conversation_id AND viewer.identity_id=?5)))", crate::audience::journal_scope(3));
    let scope = format!("{scope} AND (?6 IS NULL OR NOT EXISTS(SELECT 1 FROM journal_sources export WHERE export.sequence=visible_journal.sequence AND export.owner_id!=?5 AND NOT EXISTS(SELECT 1 FROM json_each(?6) allowed WHERE allowed.value IN(export.owner_id,'*'))))");
    let scope = format!("{scope} AND (?7 IS NULL OR sequence<=?7)");
    let private_floor =
        private_export_refusal("export.kind", "export.id", "export.conversation_id", "?8");
    let scope=format!("{scope} AND (?5 IS NULL OR NOT EXISTS(SELECT 1 FROM journal_sources export JOIN private_content_sources private ON private.kind=export.kind AND private.source_id=export.id AND private.conversation_id=export.conversation_id WHERE export.sequence=visible_journal.sequence AND {private_floor}))");
    let audience_json = audience
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| Error::storage())?;
    // Every current listener must be allowed every exported source. An AI's
    // shared identity alone cannot move private history into a public Session.
    let scope = format!("{scope} AND (?9 IS NULL OR NOT EXISTS(SELECT 1 FROM journal_sources export JOIN json_each(?9) receiver WHERE export.sequence=visible_journal.sequence AND NOT EXISTS(SELECT 1 FROM content_audiences allowed JOIN participants viewer ON viewer.participant_id=allowed.participant_id AND viewer.membership_epoch=allowed.membership_epoch AND viewer.active=1 WHERE allowed.kind=export.kind AND allowed.id=export.id AND allowed.conversation_id=export.conversation_id AND viewer.identity_id=json_extract(receiver.value,'$[0]') AND viewer.participant_id=json_extract(receiver.value,'$[1]') AND viewer.membership_epoch=json_extract(receiver.value,'$[2]'))))");
    let records_enabled =
        actor.permits("record_get") && delegate.is_none_or(|a| a.permits("record_get"));
    let sources = format!("(message_id IS NOT NULL OR session_json IS NOT NULL OR artifact_json IS NOT NULL OR (kind='session_control' AND source_kind='session') OR ({} AND kind IN ('record_ready','record_history_shared') AND source_kind='record'))",if records_enabled {1}else{0});
    let watermark: u64 = c
        .query_row(
            &format!("SELECT COALESCE(MAX(sequence),0) FROM visible_journal WHERE {scope}"),
            params![
                actor.identity_id,
                conversation_id,
                recipients,
                limit,
                recipient,
                recipient_scope,
                through,
                private_record,
                audience_json
            ],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let total: u64 = c
        .query_row(
            &format!("SELECT COUNT(*) FROM visible_journal WHERE {scope} AND {sources}"),
            params![
                actor.identity_id,
                conversation_id,
                recipients,
                limit,
                recipient,
                recipient_scope,
                through,
                private_record,
                audience_json
            ],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut st=c.prepare(&format!("SELECT sequence,kind,message_id,session_json,artifact_json,source_id FROM visible_journal WHERE {scope} AND {sources} ORDER BY sequence DESC LIMIT ?4")).map_err(|_|Error::storage())?;
    let rows = st
        .query_map(
            params![
                actor.identity_id,
                conversation_id,
                recipients,
                limit,
                recipient,
                recipient_scope,
                through,
                private_record,
                audience_json
            ],
            |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut items = vec![];
    let mut selected_bytes = 0usize;
    let mut record_input_budget = 32 * 1024 * 1024;
    let mut record_listeners = audience.map(|a| a.to_vec());
    if record_listeners.is_none() {
        if let Some(peer) = recipient {
            let member = crate::audience::member(c, peer, conversation_id, true)?;
            record_listeners = Some(vec![(
                peer.into(),
                member.participant_id,
                member.membership_epoch,
            )]);
        }
    }
    for (sequence, kind, message_id, session_json, artifact_json, journal_source) in rows {
        let mut record = None;
        let (source_id, actor_id, body) =
            if matches!(kind.as_str(), "record_ready" | "record_history_shared") {
                let id = journal_source.ok_or_else(Error::storage)?;
                match crate::records::context::select(
                    store,
                    c,
                    actor,
                    conversation_id,
                    &id,
                    delegate,
                    record_listeners.as_deref(),
                    &mut record_input_budget,
                ) {
                    Ok(projected) => {
                        record = Some(projected.reference);
                        (id, projected.owner, projected.body)
                    }
                    Err(error) if error.code == "forbidden" => continue,
                    Err(error) if error.code == "capacity-exceeded" => break,
                    Err(error) => return Err(error),
                }
            } else if kind == "session_control" {
                session::canonical_session(c, &actor.identity_id, conversation_id, sequence)?
            } else if let Some(id) = message_id {
                let message =
                    crate::messages::project(c, actor, conversation_id, &id, recipient, delegate)?;
                let body = crate::messages::context_body(&message)?;
                (id, message.sender, body)
            } else if session_json.is_some() || artifact_json.is_some() {
                let resource_kind = if session_json.is_some() {
                    connect_protocol::membership::ContentKind::Session
                } else {
                    connect_protocol::membership::ContentKind::Artifact
                };
                match session::saved_metadata(
                    c,
                    &actor.identity_id,
                    conversation_id,
                    sequence,
                    resource_kind,
                ) {
                    Ok(saved) => saved,
                    Err(e) if e.code == "forbidden" => continue,
                    Err(e) => return Err(e),
                }
            } else {
                continue;
            };
        let size = kind.len()
            + source_id.len()
            + actor_id.len()
            + body.len()
            + std::mem::size_of::<u64>()
            + record
                .as_ref()
                .map(serde_json::to_vec)
                .transpose()
                .map_err(|_| Error::storage())?
                .map_or(0, |b| b.len());
        if selected_bytes.saturating_add(size) > MAX_CONTEXT_BYTES {
            break;
        }
        selected_bytes += size;
        items.push(ContextItem {
            sequence,
            kind,
            source_id,
            actor_id,
            body,
            record,
        });
    }
    items.reverse();
    let page = if recipient.is_some() {
        connect_protocol::ContextDeliveryPage {
            deliveries: vec![],
            next: None,
        }
    } else {
        deliveries(c, actor, conversation_id, None, 100)?
    };
    let delivery_count: u64 = if recipient.is_some() {
        0
    } else {
        c.query_row("SELECT COUNT(*) FROM context_deliveries d JOIN content_audiences a ON a.kind='context_delivery' AND a.id=d.id JOIN participants p ON p.participant_id=a.participant_id AND p.membership_epoch=a.membership_epoch AND p.active=1 WHERE d.conversation_id=?1 AND p.identity_id=?2 AND (d.owner_id=?2 OR EXISTS(SELECT 1 FROM json_each(?3) WHERE value IN(d.owner_id,'*')))",params![conversation_id,actor.identity_id,recipients],|r|r.get(0)).map_err(|_|Error::storage())?
    };
    Ok(ContextView {
        conversation_id: conversation_id.into(),
        watermark,
        omitted: total.saturating_sub(items.len() as u64),
        items,
        deliveries_omitted: delivery_count.saturating_sub(page.deliveries.len() as u64),
        deliveries: page.deliveries,
    })
}

impl Store {
    /// Build the exact authorized context projection used for one live Session.
    pub fn context_for_session(
        &self,
        credential_id: &str,
        session_id: &str,
    ) -> Result<ContextView> {
        let mut connection = self.connection()?;
        let c = connection.transaction().map_err(|_| Error::storage())?;
        crate::connection_control::private_consultation::require_connection_context(
            &c, session_id,
        )?;
        let actor = crate::identity::actor_from_connection(&c, credential_id)?;
        actor.require("context_view")?;
        let conversation_id: String = c
            .query_row(
                "SELECT conversation_id FROM sessions WHERE id=?1 AND (initiator_id=?2 OR participant_id=?2)",
                params![session_id, actor.identity_id],
                |row| row.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        crate::conversations::authorized_conversation(&c, &actor, &conversation_id)?;
        let peer:String=c.query_row("SELECT CASE WHEN initiator_id=?2 THEN participant_id ELSE initiator_id END FROM sessions WHERE id=?1",params![session_id,actor.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        crate::audience::member(&c, &peer, &conversation_id, true)?;
        let session = crate::sessions::authorize_binding(&c, &actor, session_id)?;
        crate::sessions::require_original_epoch(&c, &peer, &session)?;
        let delegate = crate::session_routes::context_actor(&c, &conversation_id, &peer)?;
        view_for_scope(
            self,
            &c,
            &actor,
            &conversation_id,
            1000,
            Some(&peer),
            delegate.as_ref(),
        )
    }

    /// Persist the model-visible selection before any provider effect.
    pub fn begin_context_delivery(
        &self,
        credential_id: &str,
        session_id: &str,
        provider: &str,
        selection: &ContextView,
    ) -> Result<String> {
        if provider != "vapi"
            || selection.items.is_empty()
            || selection.items.len() > 1000
            || serde_json::to_vec(selection)
                .map_err(|_| Error::storage())?
                .len()
                > 65536
        {
            return Err(Error::new("invalid-request", "Invalid context delivery"));
        }
        let mut connection = self.connection()?;
        let c = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        crate::connection_control::private_consultation::require_connection_context(
            &c, session_id,
        )?;
        let actor = crate::identity::actor_from_connection(&c, credential_id)?;
        actor.require("context_view")?;
        let authorized: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND conversation_id=?2 AND (initiator_id=?3 OR participant_id=?3) AND state IN ('accepted','ready','active'))",
                params![session_id, selection.conversation_id, actor.identity_id],
                |row| row.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !authorized {
            return Err(Error::forbidden());
        }
        crate::conversations::authorized_conversation(&c, &actor, &selection.conversation_id)?;
        let peer:String=c.query_row("SELECT CASE WHEN initiator_id=?2 THEN participant_id ELSE initiator_id END FROM sessions WHERE id=?1",params![session_id,actor.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
        let session = crate::sessions::authorize_binding(&c, &actor, session_id)?;
        crate::sessions::require_original_epoch(&c, &peer, &session)?;
        let delegate = crate::session_routes::context_actor(&c, &selection.conversation_id, &peer)?;
        let current = view_for_scope(
            self,
            &c,
            &actor,
            &selection.conversation_id,
            1,
            Some(&peer),
            delegate.as_ref(),
        )?;
        if selection.watermark > current.watermark
            || selection
                .items
                .iter()
                .any(|i| i.sequence > selection.watermark)
        {
            return Err(Error::forbidden());
        }
        let mut record_budget = 32 * 1024 * 1024;
        for item in &selection.items {
            authorize_export_event(
                &c,
                &actor,
                &peer,
                delegate.as_ref(),
                &selection.conversation_id,
                item.sequence,
            )?;
            let kind = crate::audience::context_kind(&item.kind);
            if kind != connect_protocol::membership::ContentKind::Record && item.record.is_some() {
                return Err(Error::forbidden());
            }
            if let Some(delegate) = &delegate {
                delegate.require("context_view")?;
                crate::audience::require(
                    &c,
                    delegate,
                    &selection.conversation_id,
                    kind,
                    &item.source_id,
                )?;
            }
            crate::audience::require(
                &c,
                &actor,
                &selection.conversation_id,
                kind,
                &item.source_id,
            )?;
            if !crate::audience::visible_to_identity(
                &c,
                &peer,
                &selection.conversation_id,
                kind,
                &item.source_id,
            )? {
                return Err(Error::forbidden());
            }
            let (source_actor, body) = match kind {
                connect_protocol::membership::ContentKind::Record => {
                    let member =
                        crate::audience::member(&c, &peer, &selection.conversation_id, true)?;

                    let current = crate::records::context::select(
                        self,
                        &c,
                        &actor,
                        &selection.conversation_id,
                        &item.source_id,
                        delegate.as_ref(),
                        Some(&[(peer.clone(), member.participant_id, member.membership_epoch)]),
                        &mut record_budget,
                    )?;
                    if item.record.as_ref() != Some(&current.reference) {
                        return Err(Error::forbidden());
                    }
                    (current.owner, current.body)
                }
                connect_protocol::membership::ContentKind::Artifact
                | connect_protocol::membership::ContentKind::Session => {
                    let (source, owner, body) = if item.kind == "session_control" {
                        session::canonical_session(
                            &c,
                            &actor.identity_id,
                            &selection.conversation_id,
                            item.sequence,
                        )?
                    } else {
                        session::saved_metadata(
                            &c,
                            &actor.identity_id,
                            &selection.conversation_id,
                            item.sequence,
                            kind,
                        )?
                    };
                    if source != item.source_id {
                        return Err(Error::forbidden());
                    }
                    (owner, body)
                }
                _ => {
                    let message = crate::messages::project(
                        &c,
                        &actor,
                        &selection.conversation_id,
                        &item.source_id,
                        Some(&peer),
                        delegate.as_ref(),
                    )?;
                    let body = crate::messages::context_body(&message)?;
                    (message.sender, body)
                }
            };
            if source_actor != item.actor_id || body != item.body {
                return Err(Error::forbidden());
            }
            let reference:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM visible_journal WHERE sequence=?1 AND conversation_id=?2 AND recipient=?3 AND source_id=?4 AND kind=?5)",params![item.sequence,selection.conversation_id,actor.identity_id,item.source_id,item.kind],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !reference {
                return Err(Error::forbidden());
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = crate::now();
        c.execute(
            "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence,owner_id) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7,?7,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries),?8)",
            params![id, selection.conversation_id, session_id, provider, selection.watermark, serde_json::to_string(selection).map_err(|_| Error::storage())?, timestamp,actor.identity_id],
        )
        .map_err(|_| Error::storage())?;
        crate::identity::actor_from_connection(&c, credential_id)?;
        c.commit().map_err(|_| Error::storage())?;
        Ok(id)
    }

    /// Complete a previously admitted delivery. Provider references remain
    /// opaque and bounded; private URLs and credentials are never accepted.
    pub fn finish_context_delivery(
        &self,
        delivery_id: &str,
        state: &str,
        provider_ref: Option<&str>,
    ) -> Result<()> {
        if !matches!(state, "delivered" | "failed")
            || provider_ref.is_some_and(|value| value.is_empty() || value.len() > 512)
        {
            return Err(Error::new(
                "invalid-request",
                "Invalid context delivery outcome",
            ));
        }
        let changed = self
            .connection()?
            .execute(
                "UPDATE context_deliveries SET state=?1,provider_ref=?2,updated_at=?3 WHERE id=?4 AND state='pending'",
                params![state, provider_ref, crate::now(), delivery_id],
            )
            .map_err(|_| Error::storage())?;
        if changed != 1 {
            return Err(Error::new(
                "conflict",
                "Context delivery is already terminal",
            ));
        }
        Ok(())
    }
}

/// Page by immutable insertion order, newest first. Cursor binds identity and conversation.
pub(crate) fn deliveries(
    c: &Connection,
    actor: &Actor,
    cid: &str,
    before: Option<&str>,
    limit: u32,
) -> Result<connect_protocol::ContextDeliveryPage> {
    let resource = format!("context-deliveries:{cid}");
    let boundary = crate::cursor::decode(c, &actor.identity_id, &resource, before)?;
    let mut statement = c.prepare("SELECT d.sequence,d.id,d.session_id,d.provider,d.watermark,d.state,d.provider_ref,d.created_at,d.updated_at FROM context_deliveries d WHERE d.conversation_id=?1 AND (?2=0 OR d.sequence<?2) AND (d.owner_id=?4 OR EXISTS(SELECT 1 FROM json_each(?5) WHERE value IN(d.owner_id,'*'))) AND EXISTS(SELECT 1 FROM content_audiences a JOIN participants p ON p.participant_id=a.participant_id AND p.membership_epoch=a.membership_epoch AND p.active=1 WHERE a.kind='context_delivery' AND a.id=d.id AND p.identity_id=?4) ORDER BY d.sequence DESC LIMIT ?3").map_err(|_|Error::storage())?;
    let mut rows = statement
        .query_map(
            params![
                cid,
                boundary,
                limit + 1,
                actor.identity_id,
                serde_json::to_string(&actor.scope.recipients).map_err(|_| Error::storage())?
            ],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    ContextDelivery {
                        id: r.get(1)?,
                        session_id: r.get(2)?,
                        provider: r.get(3)?,
                        watermark: r.get(4)?,
                        state: r.get(5)?,
                        provider_ref: r.get(6)?,
                        created_at: r.get(7)?,
                        updated_at: r.get(8)?,
                        destination: None,
                        problem: None,
                        lifecycle: None,
                    },
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    for (_, receipt) in &mut rows {
        delivery::metadata(c, receipt)?;
    }

    let next = if more {
        rows.last()
            .map(|(seq, _)| crate::cursor::encode(c, &actor.identity_id, &resource, *seq))
            .transpose()?
    } else {
        None
    };
    Ok(connect_protocol::ContextDeliveryPage {
        deliveries: rows.into_iter().map(|(_, v)| v).collect(),
        next,
    })
}

/// A fresh creation cannot adopt opaque memory from any already captured lane.
/// Called inside the creation-observation transaction, before a reference is bound
/// or a context export/cleanup capability for that reference can be used.
pub(crate) fn require_fresh_provider_reference(
    c: &rusqlite::Connection,
    reference: &str,
    delivery: &str,
) -> Result<()> {
    let reused:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM context_deliveries WHERE provider='vapi' AND provider_ref=?1 AND id!=?2) OR EXISTS(SELECT 1 FROM provider_jobs WHERE provider_id=?1) OR EXISTS(SELECT 1 FROM voice_calls WHERE provider_id=?1) OR EXISTS(SELECT 1 FROM private_provider_contexts WHERE provider_ref=?1 AND delivery_id!=?2) OR EXISTS(SELECT 1 FROM native_assistant_providers WHERE provider_ref=?1 AND delivery_id!=?2)",params![reference,delivery],|r|r.get(0)).map_err(|_|Error::storage())?;
    if reused {
        return Err(Error::new(
            "conflict",
            "Provider context reference is already in use",
        ));
    }
    Ok(())
}
