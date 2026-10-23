#![recursion_limit = "256"]
// Native signal metadata requires deeper Send/Sync proofs across adapter types.
//! Shared authorization and durable application operations for every transport.
use async_trait::async_trait;
use connect_protocol::{
    Error, Operation, Outcome, Request, Response, Result, PROFILE, REQUEST_TYPE, REQUIRED_SCOPE,
    RESULT_TYPE,
};
use connect_store::Store;
use rvoip_uctp::{
    application::{ApplicationContext, ApplicationError, ApplicationHandler},
    envelope::UctpEnvelope,
    types::MessageType,
};
use std::sync::Arc;

mod authenticated_operations;
mod authentication;
mod credential_binding;
mod record_context;
pub use credential_binding::{StoreCredentialBinding, StoreInvocationAuthority};
mod canonical;
mod canonical_assurance;
mod canonical_capability_delivery;
mod canonical_conversations;
mod canonical_http;
mod canonical_media;
mod canonical_membership;
mod canonical_native;
mod canonical_native_capabilities;
mod canonical_native_data;
mod canonical_native_dtmf;
mod canonical_native_lifecycle;
mod canonical_native_quality;
mod canonical_native_streams;
mod canonical_native_subscriptions;
mod canonical_operations;
mod canonical_preparation;
mod canonical_receipts;
mod canonical_received_dtmf;
mod canonical_records;
mod canonical_runtime;
mod canonical_session_codecs;
mod canonical_session_lifecycle;
mod canonical_sip_signal;
pub use canonical_runtime::CanonicalRuntimeSnapshot;
mod assistant_context;
mod assistant_observations;
pub use assistant_context::{AssistantContextRead, AssistantProviderContextRead};
pub use assistant_observations::ProviderObservationSource;
mod canonical_consultation;
mod canonical_consultation_return;
mod canonical_handoff;
mod canonical_replacement;
mod initial_telephone;
mod telephone_replacement;
pub use initial_telephone::InitialTelephoneRoute;
pub use telephone_replacement::TelephoneReplacementRoute;
mod consultation_init;
pub use consultation_init::{ConsultationInitialization, ConsultationInitializer};
mod session_init;
pub use session_init::{SessionInitialization, SessionInitializer};
mod canonical_invitations;
#[cfg(test)]
mod canonical_session_decisions;
mod canonical_sessions;
mod canonical_subscriptions;
mod canonical_transitions;
mod peer_authority;
pub use peer_authority::StorePeerAuthority;
mod identity_access;
mod incoming_contacts;
mod membership;
mod messages;
mod native_media;
mod provisioning;
mod signing_keys;
pub use signing_keys::StoreKeyResolver;
mod session_bindings;
pub use authentication::StoreValidator;
pub use canonical::CanonicalHandler;
pub use identity_access::{IdentityAccess, IdentityAccessView, TaskCreateDefaults};
pub use native_media::{native_media_scope_policy, NATIVE_MEDIA_SCOPE};
pub use session_bindings::StoreSessionBindingResolver;
pub mod readiness;
pub mod tasks;

pub struct Service {
    pub(crate) assurance_policy: std::sync::OnceLock<Arc<canonical_assurance::Policy>>,
    /// Process-local readiness; no credentials or message contents.
    pub readiness: Arc<readiness::OperationalReadiness>,
    pub store: Arc<Store>,
    pub(crate) canonical_runtime: std::sync::OnceLock<Arc<canonical_runtime::Runtime>>,
    pub(crate) record_references:
        std::sync::OnceLock<Arc<rvoip_uctp::v2::recording::RecordReferenceRegistry>>,
    changed: tokio::sync::Notify,
    runtime: connect_store::DiscoveryRuntime,
    #[cfg(test)]
    wait_boundary: Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>,
    #[cfg(test)]
    blocking_boundary: Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>,
}
impl Service {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Self::with_runtime(store, connect_store::DiscoveryRuntime::default())
    }
    /// Configure discovery from successfully constructed runtime listeners and adapters.
    pub fn with_runtime(store: Arc<Store>, runtime: connect_store::DiscoveryRuntime) -> Arc<Self> {
        Self::with_readiness(
            store,
            runtime,
            Arc::new(readiness::OperationalReadiness::default()),
        )
    }
    /// Share component observations with runtime workers; never persisted as authority.
    pub fn with_readiness(
        store: Arc<Store>,
        runtime: connect_store::DiscoveryRuntime,
        readiness: Arc<readiness::OperationalReadiness>,
    ) -> Arc<Self> {
        store.install_record_context_projector(Arc::new(record_context::CanonicalRecordContext));
        Arc::new(Self {
            assurance_policy: std::sync::OnceLock::new(),
            runtime,
            store,
            canonical_runtime: std::sync::OnceLock::new(),
            record_references: std::sync::OnceLock::new(),
            changed: tokio::sync::Notify::new(),
            readiness,
            #[cfg(test)]
            wait_boundary: None,
            #[cfg(test)]
            blocking_boundary: None,
        })
    }
    /// Install the canonical Core/QUIC composition before accepting peers.
    pub fn install_canonical_runtime(&self) -> Result<()> {
        let runtime = canonical_runtime::Runtime::new(self.store.clone())?;
        self.canonical_runtime
            .set(runtime.clone())
            .map_err(|_| Error::new("conflict", "Canonical runtime is already installed"))?;
        if tokio::runtime::Handle::try_current().is_ok() {
            runtime.ensure_session_codec_worker()?;
        }
        Ok(())
    }
    pub fn canonical_runtime_snapshot(&self) -> Option<CanonicalRuntimeSnapshot> {
        self.canonical_runtime
            .get()
            .map(|runtime| runtime.snapshot())
    }
    /// Actual Core metadata for isolated contention tests; grants no IO authority.
    #[cfg(feature = "test-support")]
    pub fn canonical_session_metadata_for_test(
        &self,
        session_id: &str,
    ) -> Result<Arc<std::sync::RwLock<rvoip_core::session::Session>>> {
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let session = runtime
            .core
            .session(&rvoip_core::ids::SessionId::from_string(session_id))
            .ok_or_else(Error::forbidden)?;
        Ok(session)
    }
    /// Hold actual leaf binding metadata for isolated contention tests. The
    /// callback receives no binding, credential or authority object.
    #[cfg(feature = "test-support")]
    pub fn with_canonical_native_binding_for_test(
        &self,
        connection_id: &str,
        hold: impl FnOnce(),
    ) -> Result<()> {
        self.canonical_runtime
            .get()
            .ok_or_else(Error::forbidden)?
            .media
            .with_native_binding_for_test(connection_id, hold)
    }
    /// Drain the registered adapter's owned work after physical intake stops.
    pub async fn drain_canonical_runtime(&self) {
        let _ = self.drain_session_initialization().await;
        let _ = self.drain_consultation_initialization().await;
        if let Some(runtime) = self.canonical_runtime.get() {
            runtime.stop.cancel();
            self.store.uninstall_session_codec_coordinator();
            let worker = runtime
                .session_codec_worker
                .lock()
                .ok()
                .and_then(|mut w| w.take());
            if let Some(mut worker) = worker {
                if tokio::time::timeout(std::time::Duration::from_secs(3), &mut worker)
                    .await
                    .is_err()
                {
                    worker.abort();
                    let _ = worker.await;
                }
            }
            let worker = {
                runtime
                    .session_lifecycle_worker
                    .lock()
                    .ok()
                    .and_then(|mut w| w.take())
            };
            if let Some(mut worker) = worker {
                if tokio::time::timeout(std::time::Duration::from_secs(3), &mut worker)
                    .await
                    .is_err()
                {
                    worker.abort();
                    let _ = worker.await;
                }
            }
            let worker = {
                runtime
                    .session_reconnect_worker
                    .lock()
                    .ok()
                    .and_then(|mut w| w.take())
            };
            if let Some(mut worker) = worker {
                if tokio::time::timeout(std::time::Duration::from_secs(3), &mut worker)
                    .await
                    .is_err()
                {
                    worker.abort();
                    let _ = worker.await;
                }
            }

            runtime.core.drain_prepared_outbound_connections().await;
            runtime.core.drain_connection_lifecycle_tasks().await;
            let mut work =
                { std::mem::take(&mut *runtime.work.lock().unwrap_or_else(|p| p.into_inner())) };
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                while work.join_next().await.is_some() {}
            })
            .await;
            let telephone = runtime.telephone.lock().ok().and_then(|t| t.clone());
            if let Some(telephone) = telephone {
                if telephone.adapter.drain().await.is_err() {
                    tracing::debug!(target:"connect_service::telephone_replacement", "Telephone adapter drain remains incomplete");
                }
            }
        }
    }
    /// Register the wake hint before journal catch-up. Only a committed journal
    /// read proves delivery; wake hints may be coalesced or absent after restart.
    pub async fn execute(&self, credential_id: String, operation: Operation) -> Result<Outcome> {
        if matches!(&operation, Operation::SessionControl { request } if matches!(request.action, connect_protocol::session_control::Action::Renegotiate { .. }))
        {
            if let Some(runtime) = self.canonical_runtime.get() {
                runtime.ensure_session_codec_worker()?;
            }
        }
        if let Operation::Task { request } = operation {
            return self
                .execute_task(credential_id, *request)
                .await
                .map(|outcome| Outcome::Task(Box::new(outcome)));
        }
        let wake = self.changed.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let mut result = self.run(credential_id.clone(), operation.clone()).await;
        if let Ok(Outcome::Health { readiness, .. }) = &mut result {
            *readiness = Some(self.readiness.snapshot());
        }
        if let (
            Operation::SessionObservations { query, .. },
            Ok(Outcome::SessionObservations(page)),
        ) = (&operation, &result)
        {
            if query.wait_ms > 0
                && page.observations.is_empty()
                && !page.resync_required
                && !page.has_more
            {
                let remaining = page
                    .expires_at
                    .saturating_sub(chrono::Utc::now().timestamp())
                    .max(0) as u64
                    * 1000;
                let mut retry = operation.clone();
                if let Operation::SessionObservations { query, .. } = &mut retry {
                    query.after = Some(page.cursor.clone());
                }
                #[cfg(test)]
                if let Some((reached, release)) = &self.wait_boundary {
                    reached.wait().await;
                    release.wait().await;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(u64::from(query.wait_ms).min(remaining)),
                    wake,
                )
                .await;
                return self.run(credential_id, retry).await;
            }
        }
        if let (
            Operation::ConversationEvents { query, .. },
            Ok(Outcome::ConversationEvents(page)),
        ) = (&operation, &result)
        {
            if query.wait_ms > 0
                && page.events.is_empty()
                && page.snapshot.is_none()
                && !page.resync_required
                && !page.has_more
            {
                let remaining = page
                    .expires_at
                    .unwrap_or(0)
                    .saturating_sub(chrono::Utc::now().timestamp())
                    .max(0) as u64
                    * 1000;
                let mut retry = operation.clone();
                if let Operation::ConversationEvents { query, .. } = &mut retry {
                    query.lease = page.lease.clone();
                }
                #[cfg(test)]
                if let Some((reached, release)) = &self.wait_boundary {
                    reached.wait().await;
                    release.wait().await;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(u64::from(query.wait_ms).min(remaining)),
                    wake,
                )
                .await;
                return self.run(credential_id, retry).await;
            }
        }
        if let Operation::Inbox {
            cursor: previous,
            wait_ms,
            ..
        } = &operation
        {
            if *wait_ms > 0
                && matches!(&result, Ok(Outcome::Inbox {events,cursor}) if events.is_empty() && cursor == previous)
            {
                #[cfg(test)]
                if let Some((reached, release)) = &self.wait_boundary {
                    reached.wait().await;
                    release.wait().await;
                }
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(u64::from(*wait_ms)),
                    wake,
                )
                .await;
                return self.run(credential_id, operation).await;
            }
        } else if operation.request_id().is_some() && result.is_ok() {
            self.changed.notify_waiters();
        }
        result
    }
    /// Trusted adapter composition; provider payloads cannot select this handle.
    pub fn publish_session_observation(
        &self,
        source: &connect_store::SessionObservationSource,
        event: &str,
        data: connect_protocol::events::ObservationDraft,
    ) -> Result<String> {
        let id = self
            .store
            .publish_session_observation(source, event, data)?;
        self.changed.notify_waiters();
        Ok(id)
    }
    /// Wake subscribers after connector journal commits; journal reads remain authoritative.
    pub fn notify_changed(&self) {
        self.changed.notify_waiters();
    }
    async fn run(&self, credential_id: String, operation: Operation) -> Result<Outcome> {
        let store = self.store.clone();
        let runtime = self.runtime.clone();
        tokio::task::spawn_blocking(move || {
            store.execute_with_runtime(&credential_id, &operation, &runtime)
        })
        .await
        .map_err(|_| Error::storage())?
    }
}
#[async_trait]
impl ApplicationHandler for Service {
    fn profile(&self) -> &'static str {
        PROFILE
    }
    fn required_scope(&self) -> &'static str {
        REQUIRED_SCOPE
    }
    async fn handle(
        &self,
        context: ApplicationContext,
        envelope: UctpEnvelope,
    ) -> std::result::Result<UctpEnvelope, ApplicationError> {
        let result = if envelope.v != rvoip_uctp::UCTP_ENVELOPE_VERSION
            || envelope.msg_type != MessageType::Unknown(REQUEST_TYPE.into())
        {
            Err(Error::new(
                "invalid-request",
                "Invalid application envelope",
            ))
        } else {
            match envelope.decode_payload::<Request>() {
                Ok(request)
                    if request.profile == PROFILE
                        && connect_protocol::application_api::response_kind(&request.operation)
                            .is_none() =>
                {
                    Err(Error::new(
                        "unsupported-command",
                        "Use the canonical command API",
                    ))
                }
                Ok(request)
                    if request.profile == PROFILE
                        && (envelope.cid.is_none()
                            || envelope.cid.as_deref() == request.operation.conversation_id()) =>
                {
                    let operation = request.operation;
                    let exchange_ref = diagnostic_ref(&envelope.id);
                    let logical_ref = operation.request_id().map(diagnostic_ref);
                    let credential_ref = diagnostic_ref(&context.principal.subject);
                    let execution =
                        self.execute_authenticated(context.principal, operation.clone());
                    let waiting = matches!(
                        operation,
                        Operation::Inbox { .. }
                            | Operation::ConversationEvents { .. }
                            | Operation::SessionObservations { .. }
                    ) || matches!(&operation, Operation::Task { request } if request.command.wait_ms().is_some());
                    let result = if waiting {
                        tokio::select! {
                            result = execution => result,
                            _ = context.closed.cancelled() => Err(Error::new("cancelled", "Physical peer closed; resume from the durable cursor")),
                        }
                    } else {
                        execution.await
                    };
                    #[cfg(feature = "test-support")]
                    if result.is_ok() {
                        if let Some(id) = operation.request_id() {
                            connect_store::fault_point("before-ack", id);
                        }
                    }
                    tracing::info!(target:"connect_server::operations",operation=operation.name(),exchange_ref,logical_ref,credential_ref,status=result.as_ref().err().map_or("ok",|e|e.code.as_str()),"application operation completed");
                    result
                }
                _ => Err(Error::new("invalid-request", "Invalid application request")),
            }
        };
        let mut response = UctpEnvelope::new(
            MessageType::Unknown(RESULT_TYPE.into()),
            serde_json::to_value(Response {
                profile: PROFILE.into(),
                result,
            })
            .map_err(|_| ApplicationError::new(500, "response-encoding"))?,
        );
        response.cid = envelope.cid;
        Ok(response)
    }
    async fn replay(
        &self,
        context: ApplicationContext,
        envelope: UctpEnvelope,
    ) -> std::result::Result<UctpEnvelope, ApplicationError> {
        // The store owns logical deduplication. Replaying reads is harmless;
        // mutations must atomically compare their recorded digest and receipt.
        self.handle(context, envelope).await
    }
}

fn diagnostic_ref(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))[..16].to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Panics are deliberate assertions in these fixtures.
mod tests {
    use super::*;
    use connect_protocol::Limits;
    use connect_store::Scope;
    use rvoip_auth_core::BearerValidator;
    #[tokio::test]
    async fn inbox_wait_is_bounded_and_cancelled_with_physical_peer() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let identity = store.create_identity("app").unwrap();
        let key = store
            .issue(
                &identity.id,
                Scope::operator(),
                chrono::Utc::now().timestamp() + 60,
            )
            .unwrap();
        let service = Service::new(store.clone());
        let empty = service.execute(
            key.id.clone(),
            Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 20,
            },
        );
        assert!(
            matches!(tokio::time::timeout(std::time::Duration::from_secs(1),empty).await.unwrap().unwrap(),Outcome::Inbox {events,..} if events.is_empty())
        );
        let principal = StoreValidator::new(store)
            .validate_principal(&key.token)
            .await
            .unwrap();
        let (outbound, _) = tokio::sync::mpsc::channel(1);
        let closed = tokio_util::sync::CancellationToken::new();
        let context = ApplicationContext {
            principal,
            outbound,
            closed: closed.clone(),
        };
        let envelope = UctpEnvelope::new(
            MessageType::Unknown(REQUEST_TYPE.into()),
            serde_json::to_value(Request::new(Operation::Inbox {
                cursor: None,
                limit: 100,
                wait_ms: 5000,
            }))
            .unwrap(),
        );
        let handler = service.clone();
        let pending = tokio::spawn(async move { handler.handle(context, envelope).await.unwrap() });
        closed.cancel();
        let response = tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap();
        let response: Response = response.decode_payload().unwrap();
        assert_eq!(response.result.unwrap_err().code, "cancelled");
    }
    #[tokio::test]
    async fn commit_between_catchup_and_wait_is_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let app = store.create_identity("app").unwrap();
        let agent = store.create_identity("agent").unwrap();
        let expiry = chrono::Utc::now().timestamp() + 60;
        let app_key = store
            .issue(&app.id, Scope::messaging(vec![agent.id.clone()]), expiry)
            .unwrap();
        let agent_key = store
            .issue(&agent.id, Scope::messaging(vec![app.id]), expiry)
            .unwrap();
        let conversation = match store
            .execute(
                &app_key.id,
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        agent.id,
                    ]),
                },
            )
            .unwrap()
        {
            Outcome::Conversation(c) => c.id,
            _ => panic!("conversation"),
        };
        let cursor = match store
            .execute(
                &agent_key.id,
                &Operation::Inbox {
                    cursor: None,
                    limit: 100,
                    wait_ms: 0,
                },
            )
            .unwrap()
        {
            Outcome::Inbox { cursor, .. } => cursor,
            _ => panic!("inbox"),
        };
        let reached = Arc::new(tokio::sync::Barrier::new(2));
        let release = Arc::new(tokio::sync::Barrier::new(2));
        let service = Arc::new(Service {
            assurance_policy: std::sync::OnceLock::new(),
            store,
            canonical_runtime: std::sync::OnceLock::new(),
            record_references: std::sync::OnceLock::new(),
            changed: tokio::sync::Notify::new(),
            readiness: Arc::new(readiness::OperationalReadiness::default()),
            wait_boundary: Some((reached.clone(), release.clone())),
            blocking_boundary: None,
            runtime: Default::default(),
        });
        let waiter = service.clone();
        let waiting = tokio::spawn(async move {
            waiter
                .execute(
                    agent_key.id,
                    Operation::Inbox {
                        cursor,
                        limit: 100,
                        wait_ms: 5000,
                    },
                )
                .await
        });
        reached.wait().await; // journal catch-up definitively finished empty
        service
            .execute(
                app_key.id,
                Operation::Send {
                    request_id: "send".into(),
                    conversation_id: conversation,
                    message: connect_protocol::messages::MessageDraft::text_all("boundary message"),
                },
            )
            .await
            .unwrap();
        release.wait().await; // only now permit the reader to start waiting
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(result,Outcome::Inbox {events,..} if events.len()==1 && events[0].message.as_ref().unwrap().body=="boundary message")
        );
    }
}

#[cfg(test)]
mod event_tests;

pub use canonical_records::http::CanonicalHttpRecordResponse;
