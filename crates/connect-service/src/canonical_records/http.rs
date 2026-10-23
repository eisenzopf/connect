//! Explicit bearer record read. The server must own the returned custody until
//! its physical response writer ends, including buffered/backpressured writes.
use super::*;
use crate::{Service, StoreCredentialBinding};
use connect_protocol::{canonical_http::ReplyBatch, Error, Result as ConnectResult};
use connect_store::{AuthorityGuardSet, AuthorityWait};
use rvoip_core::{
    ids::SessionId,
    records::{
        RecordProjection, RecordViewControl, RecordViewError, RecordViewLease, RecordViewRequest,
    },
    store::VconStore,
};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, MAX_DISPATCH_WAIT},
    recording::{GuardedRecordBody, RecordFetchError, RecordReferenceRegistry},
    scope_policy, Envelope, Payload,
};
use serde_json::json;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

struct Sources {
    request: RecordViewRequest,
    _held: Arc<AuthorityGuardSet>,
    allowed: std::collections::BTreeSet<String>,
    expires_at: chrono::DateTime<chrono::Utc>,
}
impl RecordViewLease for Sources {
    fn current(
        &self,
        request: &RecordViewRequest,
    ) -> std::result::Result<std::collections::BTreeSet<String>, RecordViewError> {
        // Original Identity/Conversation fences exclude every supported current
        // authority writer. No SQL, blocking admission or reader reacquisition.
        if request != &self.request || chrono::Utc::now() >= self.expires_at {
            return Err(RecordViewError::Authority);
        }
        Ok(self.allowed.clone())
    }
}
/// Opaque custody for a single response. This is not Clone and not a wire grant.
/// Retain through socket IO, not just body serialization or HTTP Body EOS.
pub struct CanonicalHttpRecordResponse {
    status: u16,
    body: Vec<u8>,
    projection: Option<RecordProjection>,
    download: Option<GuardedRecordBody>,
    references: Option<Arc<RecordReferenceRegistry>>,
    deadline: Instant,
    cancellation: CancellationToken,
}
impl CanonicalHttpRecordResponse {
    pub fn current(&self) -> bool {
        !self.cancellation.is_cancelled()
            && Instant::now() < self.deadline
            && self.projection.as_ref().is_none_or(|p| p.view().is_ok())
            && self.download.as_ref().is_none_or(|p| p.view().is_ok())
            && self.references.as_ref().is_none_or(|r| r.is_available())
    }
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn body(&self) -> ConnectResult<&[u8]> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(&self.body)
    }
}
impl Service {
    /// Trusted HTTP transport only. The supplied cancellation must cover the
    /// owning socket and server runtime; this does not mint a native peer proof.
    pub async fn execute_canonical_http_record(
        self: &Arc<Self>,
        binding: StoreCredentialBinding,
        request: Envelope,
        cancellation: CancellationToken,
    ) -> ConnectResult<CanonicalHttpRecordResponse> {
        if request.original().get("signature").is_some()
            || request.scope(rvoip_uctp::v2::Scope::Operation).is_some()
        {
            return Err(Error::new(
                "invalid-request",
                "HTTP record request contains a transport or mutation assertion",
            ));
        }
        let intent = super::resolve_selector(
            super::native::intent(&request)?,
            self.record_references.get().map(AsRef::as_ref),
        );
        let Payload::RecordingVconFetch(query) = request.payload() else {
            return Err(Error::forbidden());
        };
        let Admission {
            held,
            origin,
            policy,
            expires,
            deadline,
        } = self
            .admit_record_read(&binding, intent.clone(), &cancellation)
            .await?;
        let scope = VconScope::new(
            TenantId::from_string(origin.scope.tenant_id),
            ConversationId::from_string(origin.scope.conversation_id),
            SessionId::from_string(origin.scope.session_id),
        );
        let uuid = query
            .uuid
            .as_ref()
            .map(|v| uuid::Uuid::parse_str(v))
            .transpose()
            .map_err(|_| Error::new("invalid-request", "Invalid record selector"))?;
        let locator = VconLocator::new(uuid, query.url.clone())
            .map_err(|_| Error::new("invalid-request", "Invalid record selector"))?;
        let backend = Records::for_fetch(self.store.clone(), self.record_references.get().cloned());
        let read = tokio::select! {biased;
            _=cancellation.cancelled()=>return Err(Error::new("cancelled","Record read cancelled")),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::forbidden()),
            read=backend.lookup(&scope,&locator)=>read,
        };
        let mut projection = None;
        let mut references = None;
        let outcome = (|| -> std::result::Result<serde_json::Value, (u16, &'static str)> {
            let read = read
                .map_err(|_| (503, "record_store_unavailable"))?
                .ok_or((404, "record_not_found"))?;
            let policy = policy.ok_or((403, "record_authority_denied"))?;
            let h = read.handle();
            let request = RecordViewRequest::new(
                scope,
                viewer(origin.viewer),
                read.record().uuid(),
                h.revision,
                h.content_hash.clone(),
                h.manifest_hash.clone(),
                format!(
                    "record-view-{}-{}",
                    read.record().uuid().simple(),
                    h.revision
                ),
            )
            .map_err(|_| (403, "record_authority_denied"))?;
            let binding =
                view_binding(&request, &policy).ok_or((403, "record_authority_denied"))?;
            if binding != policy.binding {
                return Err((403, "record_authority_denied"));
            }
            let source = Sources {
                request: request.clone(),
                _held: held,
                allowed: policy.sources,
                expires_at: expires,
            };
            let control = RecordViewControl::new(
                request.clone(),
                Box::new(source),
                cancellation.clone(),
                deadline,
            )
            .map_err(|_| (403, "record_authority_denied"))?;
            let prepared = RecordProjection::prepare(read, control)
                .map_err(|_| (503, "record_delivery_unavailable"))?;
            let view = prepared
                .view()
                .map_err(|_| (403, "record_authority_denied"))?;
            if query
                .content_hash
                .as_ref()
                .is_some_and(|hash| hash != view.content_hash)
            {
                return Err((409, "record_view_changed"));
            }
            if view.body.len() > 128 * 1024 {
                let registry = self
                    .record_references
                    .get()
                    .filter(|r| r.is_available())
                    .ok_or((503, "record_delivery_unavailable"))?;
                let reference = registry
                    .issue(&request, &view)
                    .map_err(|_| (503, "record_delivery_unavailable"))?;
                let data = json!({"delivery":"url","url":reference.url,"expires_at":reference.expires_at,"content_hash":view.content_hash});
                references = Some(registry.clone());
                projection = Some(prepared);
                return Ok(data);
            }
            let data = json!({"delivery":"inline","vcon":std::str::from_utf8(view.body).map_err(|_|(503,"record_delivery_unavailable"))?,"content_hash":view.content_hash});
            projection = Some(prepared);
            Ok(data)
        })();
        let (status, frames) = match outcome {
            Ok(data) => (
                200,
                vec![
                    crate::canonical::reply(&request, "recording.vcon-fetched", data)?,
                    crate::canonical::reply(
                        &request,
                        "ack",
                        json!({"details":{"reply_complete":true}}),
                    )?,
                ],
            ),
            Err((status, reason)) => (
                status,
                vec![crate::canonical::reply(
                    &request,
                    "error",
                    json!({"category":"recording","code":status,"reason":reason}),
                )?],
            ),
        };
        let batch = ReplyBatch {
            profile: connect_protocol::canonical_http::PROFILE.into(),
            request_id: request.id().into(),
            envelopes: frames.into_iter().map(|f| f.original().clone()).collect(),
        };
        let body = serde_json::to_vec(&batch).map_err(|_| Error::storage())?;
        if body.len() > connect_protocol::canonical_http::MAX_RESPONSE_BYTES {
            return Err(Error::new(
                "capacity-exceeded",
                "Record response exceeds its bound",
            ));
        }
        let response = CanonicalHttpRecordResponse {
            status,
            body,
            projection,
            download: None,
            references,
            deadline,
            cancellation,
        };
        if !response.current() {
            return Err(Error::forbidden());
        }
        Ok(response)
    }
}

struct Admission {
    held: Arc<AuthorityGuardSet>,
    origin: durable::RecordViewOrigin,
    policy: Option<durable::RecordPolicySnapshot>,
    expires: chrono::DateTime<chrono::Utc>,
    deadline: Instant,
}
impl Service {
    async fn admit_record_read(
        self: &Arc<Self>,
        binding: &StoreCredentialBinding,
        intent: durable::RecordFetchIntent,
        cancellation: &CancellationToken,
    ) -> ConnectResult<Admission> {
        let credential = self
            .authorize_local_principal(
                binding.principal(),
                Some(binding),
                Some(connect_protocol::REQUIRED_SCOPE),
            )
            .await?;
        if !binding.principal().has_scope(scope_policy::SESSION) {
            return Err(Error::forbidden());
        }
        let expires = binding
            .principal()
            .expires_at
            .ok_or_else(Error::forbidden)?;
        let remaining = (expires - chrono::Utc::now())
            .to_std()
            .map_err(|_| Error::forbidden())?;
        let deadline = Instant::now() + remaining.min(MAX_DISPATCH_WAIT);
        let wait = AuthorityWait::new(std::time::Duration::from_secs(2));
        let _cancel_wait = wait.cancel_on_drop();
        let store = self.store.clone();
        let selected = intent.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            let held = store.record_read_authority(&credential, &selected, &wait)?;
            let (origin, policy) = store.record_read_policy(&held, &credential, &selected)?;
            Ok::<_, Error>((held, origin, policy))
        });
        let (held, origin, policy) = tokio::select! { biased;
            _=cancellation.cancelled()=>return Err(Error::new("cancelled","Record read cancelled")),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::forbidden()),
            result=prepared=>result.map_err(|_|Error::storage())??,
        };
        // Bind the same original held credential to the trusted bearer facts.
        // This performs SQL revalidation but acquires no second authority reader.
        self.authorize_local_principal(
            binding.principal(),
            Some(binding),
            Some(connect_protocol::REQUIRED_SCOPE),
        )
        .await?;
        Ok(Admission {
            held,
            origin,
            policy,
            expires,
            deadline,
        })
    }
}

fn reference_error(error: RecordFetchError) -> Error {
    match error {
        RecordFetchError::Missing => Error::new("not-found", "Record reference unavailable"),
        RecordFetchError::Changed => Error::new(
            "record-reference-changed",
            "Record reference changed or expired",
        ),
        RecordFetchError::Authority => Error::forbidden(),
        RecordFetchError::Storage => Error::storage(),
        RecordFetchError::Delivery => Error::new(
            "record-reference-unavailable",
            "Record delivery unavailable",
        ),
    }
}
impl Service {
    /// Install exactly one real constructed endpoint, before public intake.
    pub fn install_record_references(
        &self,
        registry: Arc<RecordReferenceRegistry>,
    ) -> ConnectResult<()> {
        self.record_references
            .set(registry)
            .map_err(|_| Error::new("conflict", "Record endpoint already installed"))
    }
    /// Trusted HTTPS adapter. The locator resolves resource metadata only;
    /// current bearer/source authority is derived anew for this download.
    pub async fn retrieve_record_reference(
        self: &Arc<Self>,
        binding: StoreCredentialBinding,
        url: &str,
        cancellation: CancellationToken,
    ) -> ConnectResult<CanonicalHttpRecordResponse> {
        self.authorize_local_principal(
            binding.principal(),
            Some(&binding),
            Some(connect_protocol::REQUIRED_SCOPE),
        )
        .await?;
        let registry = self
            .record_references
            .get()
            .filter(|r| r.is_available())
            .ok_or_else(|| {
                Error::new(
                    "record-reference-unavailable",
                    "Record delivery unavailable",
                )
            })?;
        let original = registry.reference_request(url).map_err(reference_error)?;
        let intent = durable::RecordFetchIntent {
            conversation_id: original.scope().conversation_id().to_string(),
            session_id: original.scope().session_id().to_string(),
            uuid: Some(original.uuid().to_string()),
            url: None,
        };
        let Admission {
            held,
            origin,
            policy,
            expires,
            deadline,
        } = self
            .admit_record_read(&binding, intent, &cancellation)
            .await?;
        let scope = VconScope::new(
            TenantId::from_string(origin.scope.tenant_id),
            ConversationId::from_string(origin.scope.conversation_id),
            SessionId::from_string(origin.scope.session_id),
        );
        let viewer = viewer(origin.viewer);
        let check = cancellation.clone();
        let authority = AuthorityLease::checking(held.clone(), deadline, move || {
            !check.is_cancelled() && chrono::Utc::now() < expires
        });
        let guarded = registry
            .retrieve(
                url,
                &scope,
                &viewer,
                authority,
                &Records::new(self.store.clone()),
                move |request, _authority| {
                    let policy =
                        policy.ok_or(rvoip_uctp::v2::auth_dispatch::DispatchError::Authority)?;
                    let binding = view_binding(request, &policy)
                        .ok_or(rvoip_uctp::v2::auth_dispatch::DispatchError::Authority)?;
                    if binding != policy.binding {
                        return Err(rvoip_uctp::v2::auth_dispatch::DispatchError::Authority);
                    }
                    Ok(Box::new(Sources {
                        request: request.clone(),
                        _held: held,
                        allowed: policy.sources,
                        expires_at: expires,
                    }))
                },
                cancellation.clone(),
                deadline,
            )
            .await
            .map_err(reference_error)?;
        let deadline = guarded.deadline();
        let body = guarded.view().map_err(reference_error)?.body.to_vec();
        if body.len() > 16 * 1024 * 1024 {
            return Err(Error::new(
                "capacity-exceeded",
                "Record download exceeds bound",
            ));
        }
        let response = CanonicalHttpRecordResponse {
            status: 200,
            body,
            projection: None,
            download: Some(guarded),
            references: Some(registry.clone()),
            deadline,
            cancellation,
        };
        if !response.current() {
            return Err(Error::forbidden());
        }
        Ok(response)
    }
}
