use crate::{Actor, Store};
use connect_protocol::{ContextDelivery, ContextItem, ContextView, Error, Result};
use rusqlite::{params, Connection};

const MAX_CONTEXT_BYTES: usize = 32 * 1024;

pub(crate) fn view(
    c: &Connection,
    actor: &Actor,
    conversation_id: &str,
    limit: u32,
) -> Result<ContextView> {
    let watermark: u64 = c
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM journal WHERE recipient=?1 AND conversation_id=?2",
            params![actor.identity_id, conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let total: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM journal WHERE recipient=?1 AND conversation_id=?2 AND (message_id IS NOT NULL OR session_json IS NOT NULL OR artifact_json IS NOT NULL)",
            params![actor.identity_id, conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let mut statement = c
        .prepare(
            "SELECT sequence,kind,message_id,session_json,artifact_json FROM journal WHERE recipient=?1 AND conversation_id=?2 AND (message_id IS NOT NULL OR session_json IS NOT NULL OR artifact_json IS NOT NULL) ORDER BY sequence DESC LIMIT ?3",
        )
        .map_err(|_| Error::storage())?;
    let rows = statement
        .query_map(params![actor.identity_id, conversation_id, limit], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut items = Vec::with_capacity(rows.len());
    let mut selected_bytes = 0usize;
    for (sequence, kind, message_id, session_json, artifact_json) in rows {
        let item = if let Some(message_id) = message_id {
            let (sender, body): (String, String) = c
                .query_row(
                    "SELECT sender,body FROM messages WHERE id=?1",
                    [&message_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| Error::storage())?;
            ContextItem {
                sequence,
                kind,
                source_id: message_id,
                actor_id: sender,
                body,
            }
        } else if let Some(raw) = session_json {
            let session: connect_protocol::Session =
                serde_json::from_str(&raw).map_err(|_| Error::storage())?;
            let body = serde_json::to_string(&session).map_err(|_| Error::storage())?;
            ContextItem {
                sequence,
                kind,
                source_id: session.id,
                actor_id: session.initiator_id,
                body,
            }
        } else if let Some(raw) = artifact_json {
            let artifact: connect_protocol::Artifact =
                serde_json::from_str(&raw).map_err(|_| Error::storage())?;
            let body = serde_json::to_string(&artifact).map_err(|_| Error::storage())?;
            ContextItem {
                sequence,
                kind,
                source_id: artifact.id,
                actor_id: artifact.owner_id,
                body,
            }
        } else {
            continue;
        };
        let item_bytes = item.kind.len()
            + item.source_id.len()
            + item.actor_id.len()
            + item.body.len()
            + std::mem::size_of::<u64>();
        if selected_bytes.saturating_add(item_bytes) > MAX_CONTEXT_BYTES {
            break;
        }
        selected_bytes += item_bytes;
        items.push(item);
    }
    items.reverse();
    let page = deliveries(c, actor, conversation_id, None, 100)?;
    let delivery_count: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM context_deliveries WHERE conversation_id=?1",
            [conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let deliveries_omitted = delivery_count.saturating_sub(page.deliveries.len() as u64);
    Ok(ContextView {
        conversation_id: conversation_id.into(),
        watermark,
        omitted: total.saturating_sub(items.len() as u64),
        items,
        deliveries: page.deliveries,
        deliveries_omitted,
    })
}

impl Store {
    /// Build the exact authorized context projection used for one live Session.
    pub fn context_for_session(
        &self,
        credential_id: &str,
        session_id: &str,
    ) -> Result<ContextView> {
        let c = self.connection()?;
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
        view(&c, &actor, &conversation_id, 1000)
    }

    /// Persist the model-visible selection before any provider effect.
    pub fn begin_context_delivery(
        &self,
        credential_id: &str,
        session_id: &str,
        provider: &str,
        selection: &ContextView,
    ) -> Result<String> {
        if provider != "vapi" || selection.items.is_empty() {
            return Err(Error::new("invalid-request", "Invalid context delivery"));
        }
        let c = self.connection()?;
        let actor = crate::identity::actor_from_connection(&c, credential_id)?;
        actor.require("context_view")?;
        let authorized: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND conversation_id=?2 AND (initiator_id=?3 OR participant_id=?3) AND state IN ('accepted','ready'))",
                params![session_id, selection.conversation_id, actor.identity_id],
                |row| row.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !authorized {
            return Err(Error::forbidden());
        }
        crate::conversations::authorized_conversation(&c, &actor, &selection.conversation_id)?;
        let id = uuid::Uuid::new_v4().to_string();
        let timestamp = crate::now();
        c.execute(
            "INSERT INTO context_deliveries(id,conversation_id,session_id,provider,watermark,selection_json,state,created_at,updated_at,sequence) VALUES(?1,?2,?3,?4,?5,?6,'pending',?7,?7,(SELECT COALESCE(MAX(sequence),0)+1 FROM context_deliveries))",
            params![id, selection.conversation_id, session_id, provider, selection.watermark, serde_json::to_string(selection).map_err(|_| Error::storage())?, timestamp],
        )
        .map_err(|_| Error::storage())?;
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
    let mut statement = c.prepare("SELECT sequence,id,session_id,provider,watermark,state,provider_ref,created_at,updated_at FROM context_deliveries WHERE conversation_id=?1 AND (?2=0 OR sequence<?2) ORDER BY sequence DESC LIMIT ?3").map_err(|_|Error::storage())?;
    let mut rows = statement
        .query_map(params![cid, boundary, limit + 1], |r| {
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
                },
            ))
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
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
