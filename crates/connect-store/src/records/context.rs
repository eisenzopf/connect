//! Serialization-only Core bridge. These snapshots are data, never a lease or
//! authorization for provider delivery or later opaque-memory reuse.
use super::{RecordBytes, RecordHandle, RecordScope, RecordViewer};
use connect_protocol::Result;
use std::{collections::BTreeSet, sync::Arc};

/// Facts selected by current Store policy under the caller's original read.
/// Every exporter must intersect the actor, delegate and actual listeners before
/// calling the projector; a role label or copied snapshot grants no access.
pub struct RecordContextInput {
    pub scope: RecordScope,
    pub handle: RecordHandle,
    pub bytes: RecordBytes,
    pub viewer: RecordViewer,
    pub allowed_sources: BTreeSet<String>,
    pub correlation_id: String,
}
/// Earliest content/metadata boundary for the exact projected data. Callers must
/// shorten actual IO custody to this deadline and compare fresh current source
/// selection before reusing an already delivered provider instance.
pub struct RecordContextData {
    pub body: Vec<u8>,
    pub content_hash: String,
    pub valid_until: Option<i64>,
}
impl std::fmt::Debug for RecordContextData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecordContextData(redacted)")
    }
}
/// Trusted host composition, bounded and synchronous. Implementations perform
/// canonical source pruning/validation without IO, URL forwarding or fallback.
pub trait RecordContextProjector: Send + Sync {
    fn project(&self, input: RecordContextInput) -> Result<RecordContextData>;
}
impl crate::Store {
    /// Install the process's immutable serialization bridge once. Repeated
    /// service composition over the same Store preserves the first bridge.
    pub fn install_record_context_projector(&self, projector: Arc<dyn RecordContextProjector>) {
        let _ = self.record_context_projector.set(projector);
    }
}

pub(crate) struct ContextProjection {
    pub reference: connect_protocol::ContextRecord,
    pub owner: String,
    pub body: String,
    pub created_at: Option<i64>,
}
/// Resolve one already visible record under the original caller guard and SQL
/// snapshot. No authority reader or storage reservation is reacquired here.
pub(crate) fn select(
    store: &crate::Store,
    c: &rusqlite::Connection,
    actor: &crate::Actor,
    cid: &str,
    id: &str,
    delegate: Option<&crate::Actor>,
    listeners: Option<&[(String, String, u64)]>,
    remaining: &mut usize,
) -> connect_protocol::Result<ContextProjection> {
    use super::*;
    use connect_protocol::{membership::ContentKind, ContextRecord, ContextRecordState};
    actor.require("record_get")?;
    crate::audience::require(c, actor, cid, ContentKind::Record, id)?;
    let (tenant,sid,owner):(String,String,String)=c.query_row("SELECT i.tenant_id,i.session_id,r.owner_id FROM session_record_intents i JOIN audience_resources r ON r.kind='record' AND r.id=i.uuid AND r.conversation_id=i.conversation_id WHERE i.uuid=?1 AND i.conversation_id=?2 AND i.state='ready'",params![id,cid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::forbidden())?;
    let intent = RecordFetchIntent {
        conversation_id: cid.into(),
        session_id: sid.clone(),
        uuid: Some(id.into()),
        url: None,
    };
    let origin = read::participant_origin(c, actor, &intent)?;
    let scope = RecordScope {
        tenant_id: tenant,
        conversation_id: cid.into(),
        session_id: sid.clone(),
    };
    let input_size:usize=c.query_row("SELECT COALESCE(length(body),0)+COALESCE(length(manifest),0) FROM session_records WHERE uuid=?1 AND tenant_id=?2 AND conversation_id=?3 AND session_id=?4",params![id,scope.tenant_id,scope.conversation_id,scope.session_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    if input_size > *remaining {
        return Err(Error::new(
            "capacity-exceeded",
            "Record context input budget exceeded",
        ));
    }
    let stored = row(c, &scope, id)?.ok_or_else(Error::forbidden)?;
    let mut result = ContextProjection {
        reference: ContextRecord {
            uuid: id.into(),
            session_id: sid,
            revision: stored.handle.revision,
            state: ContextRecordState::Unavailable,
            content_hash: None,
            valid_until: None,
        },
        owner,
        body: String::new(),
        created_at: None,
    };
    // All listeners need their own current source floor. A tenant-role read is
    // deliberately not used to manufacture context or passive listener rights.
    if let Some(delegate) = delegate {
        delegate.require("record_get")?;
        crate::audience::require(c, delegate, cid, ContentKind::Record, id)?;
    }
    if let Some(listeners) = listeners {
        for (identity, pid, epoch) in listeners {
            let member = crate::audience::member(c, identity, cid, true)?;
            if member.participant_id != *pid
                || member.membership_epoch != *epoch
                || !crate::audience::visible_to_identity(c, identity, cid, ContentKind::Record, id)?
            {
                return Err(Error::forbidden());
            }
        }
    }
    let Some(bytes) = stored.bytes else {
        result.reference.state = ContextRecordState::Deleted;
        return Ok(result);
    };
    let size = bytes.body.len().saturating_add(bytes.manifest.len());
    if size > *remaining {
        return Err(Error::new(
            "capacity-exceeded",
            "Record context input budget exceeded",
        ));
    }
    *remaining -= size;
    let binding = RecordViewBinding {
        uuid: id.into(),
        revision: stored.handle.revision,
        body_hash: stored.handle.body_hash.clone(),
        manifest_hash: stored.handle.manifest_hash.clone(),
        viewer: origin.viewer.clone(),
    };
    let mut allowed = read::sources_from_origin(c, actor, &intent, &binding, origin.clone(), true)?;
    if let Some(delegate) = delegate {
        let other = read::participant_origin(c, delegate, &intent)?;
        let mut other_binding = binding.clone();
        other_binding.viewer = other.viewer.clone();
        let ceiling = read::sources_from_origin(c, delegate, &intent, &other_binding, other, true)?;
        allowed.retain(|id| ceiling.contains(id));
    }
    let body: serde_json::Value =
        serde_json::from_slice(&bytes.body).map_err(|_| Error::storage())?;
    let manifest: serde_json::Value =
        serde_json::from_slice(&bytes.manifest).map_err(|_| Error::storage())?;
    if let Some(listeners) = listeners {
        for (_, pid, epoch) in listeners {
            let floor = read::participant_sources(
                c,
                &intent,
                id,
                &body,
                &manifest,
                pid,
                *epoch,
                true,
                |_| true,
            )?;
            allowed.retain(|id| floor.contains(id));
        }
    }
    let Some(projector) = store.record_context_projector.get() else {
        return Ok(result);
    };
    let data = match projector.project(RecordContextInput {
        scope,
        handle: stored.handle,
        bytes,
        viewer: origin.viewer,
        allowed_sources: allowed,
        correlation_id: format!("record-context-{}", id),
    }) {
        Ok(data) => data,
        Err(error) if matches!(error.code.as_str(), "record-unavailable" | "forbidden") => {
            return Ok(result)
        }
        Err(error) => return Err(error),
    };
    if data.valid_until.is_some_and(|until| until <= crate::now()) {
        return Ok(result);
    }
    result.body = String::from_utf8(data.body).map_err(|_| Error::storage())?;
    let projected: serde_json::Value =
        serde_json::from_str(&result.body).map_err(|_| Error::storage())?;
    result.created_at = projected["created_at"]
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.timestamp());
    result.reference.state = ContextRecordState::Available;
    result.reference.content_hash = Some(data.content_hash);
    result.reference.valid_until = data.valid_until;
    Ok(result)
}
pub(crate) fn deadline(view: &connect_protocol::ContextView) -> Option<i64> {
    view.items
        .iter()
        .filter_map(|i| i.record.as_ref().and_then(|r| r.valid_until))
        .min()
}
/// Decode only a captured host-context envelope, never a new authority source.
pub(crate) fn encoded_deadline(raw: &str) -> connect_protocol::Result<Option<i64>> {
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|_| connect_protocol::Error::storage())?;
    let items = value
        .get("items")
        .or_else(|| value.pointer("/data/sources"))
        .and_then(|v| v.as_array());
    Ok(items
        .into_iter()
        .flatten()
        .filter_map(|item| item.pointer("/record/valid_until").and_then(|v| v.as_i64()))
        .min())
}
