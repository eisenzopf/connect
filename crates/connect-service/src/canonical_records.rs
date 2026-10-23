//! Durable Core record backend. Storage custody is distinct from the current
//! source audience authority required by the public retrieval handler.
use connect_store::{records as durable, Store};
use rvoip_core::{
    error::{Result, RvoipError},
    ids::{ConversationId, TenantId},
    records::{
        redaction::{VconRedaction, VconRedactionFact},
        RecordManifest, VconRecord,
    },
    store::{VconDeletion, VconHandle, VconLocator, VconRead, VconScope, VconStore, VconTombstone},
};
use std::sync::Arc;

pub(crate) struct Records {
    store: Arc<Store>,
    references: Option<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
}
impl Records {
    pub(crate) fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            references: None,
        }
    }
    fn for_fetch(
        store: Arc<Store>,
        references: Option<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
    ) -> Self {
        Self { store, references }
    }
}
/// An issued URL is a resource locator only. Resolve metadata before source
/// admission, preserving the caller's Conversation/Session/UUID agreement.
/// Unknown, expired and mismatched selectors stay unresolved and never cause network IO.
pub(super) fn resolve_selector(
    mut intent: durable::RecordFetchIntent,
    references: Option<&rvoip_uctp::v2::recording::RecordReferenceRegistry>,
) -> durable::RecordFetchIntent {
    let Some((url, registry)) = intent.url.as_deref().zip(references) else {
        return intent;
    };
    let Ok(selected) = registry.reference_request(url) else {
        return intent;
    };
    if selected.scope().conversation_id().as_str() != intent.conversation_id
        || selected.scope().session_id().as_str() != intent.session_id
        || intent
            .uuid
            .as_ref()
            .is_some_and(|id| uuid::Uuid::parse_str(id).ok() != Some(selected.uuid()))
    {
        return intent;
    }
    intent.uuid = Some(selected.uuid().to_string());
    intent.url = None;
    intent
}
fn failed() -> RvoipError {
    RvoipError::InvalidState("Connect record persistence failed")
}
fn scope(s: &VconScope) -> durable::RecordScope {
    durable::RecordScope {
        tenant_id: s.tenant_id().to_string(),
        conversation_id: s.conversation_id().to_string(),
        session_id: s.session_id().to_string(),
    }
}
fn viewer(value: durable::RecordViewer) -> rvoip_core::records::RecordViewer {
    use rvoip_core::ids::{IdentityId, ParticipantId};
    match value {
        durable::RecordViewer::Participant {
            participant_id,
            membership_epoch,
        } => rvoip_core::records::RecordViewer::Participant {
            participant: ParticipantId::from_string(participant_id),
            membership_epoch,
        },
        durable::RecordViewer::TenantRole {
            identity_id,
            tenant_id,
            role,
            revision,
            private_sources,
        } => rvoip_core::records::RecordViewer::TenantRole {
            identity: IdentityId::from_string(identity_id),
            tenant: TenantId::from_string(tenant_id),
            role,
            revision,
            private_sources,
        },
    }
}
fn view_binding(
    request: &rvoip_core::records::RecordViewRequest,
    policy: &durable::RecordPolicySnapshot,
) -> Option<durable::RecordViewBinding> {
    if request.viewer() != &viewer(policy.origin.viewer.clone()) {
        return None;
    }
    Some(durable::RecordViewBinding {
        uuid: request.uuid().to_string(),
        revision: request.revision(),
        body_hash: request.body_hash().into(),
        manifest_hash: request.manifest_hash().into(),
        viewer: policy.origin.viewer.clone(),
    })
}
fn handle(h: durable::RecordHandle) -> VconHandle {
    VconHandle {
        url: h.location,
        content_hash: h.body_hash,
        manifest_hash: h.manifest_hash,
        revision: h.revision,
    }
}
fn expected(h: &VconHandle) -> durable::RecordHandle {
    durable::RecordHandle {
        location: h.url.clone(),
        body_hash: h.content_hash.clone(),
        manifest_hash: h.manifest_hash.clone(),
        revision: h.revision,
    }
}
fn bytes(record: &VconRecord) -> Result<durable::RecordBytes> {
    Ok(durable::RecordBytes {
        uuid: record.uuid().to_string(),
        body: record.body().to_vec(),
        manifest: record.manifest().encode()?,
    })
}
fn record(bytes: &durable::RecordBytes) -> Result<VconRecord> {
    VconRecord::new(
        bytes.body.clone().into(),
        RecordManifest::decode(&bytes.manifest)?,
    )
}
fn read(s: VconScope, stored: Option<durable::RecordRead>) -> Result<Option<VconRead>> {
    stored
        .map(|stored| {
            let record = record(&stored.bytes)?;
            VconRead::protected(s, handle(stored.handle.clone()), record, stored)
        })
        .transpose()
}
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> connect_protocol::Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| failed())?
        .map_err(|_| failed())
}
#[async_trait::async_trait]
impl VconStore for Records {
    async fn put(&self, s: &VconScope, value: VconRecord) -> Result<VconHandle> {
        let (store, s, value) = (self.store.clone(), scope(s), bytes(&value)?);
        blocking(move || store.put_record(&s, value))
            .await
            .map(handle)
    }
    async fn get(&self, s: &VconScope, h: &VconHandle) -> Result<Option<VconRead>> {
        let (store, owner, h) = (self.store.clone(), scope(s), expected(h));
        let stored =
            blocking(move || store.read_record(&owner, None, Some(&h.location), Some(&h))).await?;
        read(s.clone(), stored)
    }
    async fn lookup(&self, s: &VconScope, l: &VconLocator) -> Result<Option<VconRead>> {
        if let Some((url, references)) = l.url().zip(self.references.as_deref()) {
            if references
                .reference_request(url)
                .is_ok_and(|selected| selected.scope() != s)
            {
                return Ok(None);
            }
        }
        let intent = resolve_selector(
            durable::RecordFetchIntent {
                conversation_id: s.conversation_id().to_string(),
                session_id: s.session_id().to_string(),
                uuid: l.uuid().map(|id| id.to_string()),
                url: l.url().map(str::to_owned),
            },
            self.references.as_deref(),
        );
        let (store, owner, id, url) = (self.store.clone(), scope(s), intent.uuid, intent.url);
        let stored =
            blocking(move || store.read_record(&owner, id.as_deref(), url.as_deref(), None))
                .await?;
        read(s.clone(), stored)
    }
    async fn delete(&self, s: &VconScope, i: &VconDeletion) -> Result<Option<VconTombstone>> {
        let (store, owner, expected, op) = (
            self.store.clone(),
            scope(s),
            expected(i.expected()),
            i.operation_id().to_owned(),
        );
        let fact = blocking(move || {
            store.mutate_record(
                &owner,
                &op,
                durable::RecordMutationKind::Delete,
                "delete-v1",
                &expected,
                |_| Ok(None),
            )
        })
        .await?;
        fact.map(|f| {
            VconTombstone::recorded(
                s.clone(),
                f.result.location,
                f.operation_id,
                f.result.revision,
                f.at,
            )
        })
        .transpose()
    }
    async fn tombstone(&self, s: &VconScope, url: &str) -> Result<Option<VconTombstone>> {
        let (store, owner, url) = (self.store.clone(), scope(s), url.to_owned());
        let fact = blocking(move || store.record_tombstone(&owner, &url)).await?;
        fact.map(|f| {
            VconTombstone::recorded(
                s.clone(),
                f.result.location,
                f.operation_id,
                f.result.revision,
                f.at,
            )
        })
        .transpose()
    }
    async fn redact(&self, s: &VconScope, i: &VconRedaction) -> Result<Option<VconRedactionFact>> {
        let (store, owner, intent) = (self.store.clone(), scope(s), i.clone());
        let fact = blocking(move || {
            store.mutate_record(
                &owner,
                intent.operation_id(),
                durable::RecordMutationKind::Redact,
                intent.fingerprint(),
                &expected(intent.expected()),
                |current| {
                    let current =
                        record(current).map_err(|_| connect_protocol::Error::storage())?;
                    let next = rvoip_core::records::redaction::apply_redaction(&current, &intent)
                        .map_err(|_| connect_protocol::Error::storage())?;
                    bytes(&next)
                        .map(Some)
                        .map_err(|_| connect_protocol::Error::storage())
                },
            )
        })
        .await?;
        fact.map(|f| {
            VconRedactionFact::recorded(f.operation_id, f.fingerprint, handle(f.result), f.at)
        })
        .transpose()
    }
    async fn list_for_session(&self, s: &VconScope) -> Result<Vec<VconHandle>> {
        let (store, owner) = (self.store.clone(), scope(s));
        blocking(move || {
            store.list_records(
                &owner.tenant_id,
                &owner.conversation_id,
                Some(&owner.session_id),
            )
        })
        .await
        .map(|v| v.into_iter().map(handle).collect())
    }
    async fn list_for_conversation(
        &self,
        t: &TenantId,
        c: &ConversationId,
    ) -> Result<Vec<VconHandle>> {
        let (store, t, c) = (self.store.clone(), t.to_string(), c.to_string());
        blocking(move || store.list_records(&t, &c, None))
            .await
            .map(|v| v.into_iter().map(handle).collect())
    }
}

#[cfg(test)]
mod tests;

mod outcomes;
mod retention;

pub(crate) mod native;

pub(crate) mod http;

mod mutations;
