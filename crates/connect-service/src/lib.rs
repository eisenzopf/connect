//! Shared authorization and durable application operations for every transport.
use async_trait::async_trait;
use connect_protocol::{
    Error, Operation, Outcome, Request, Response, Result, PROFILE, REQUEST_TYPE, REQUIRED_SCOPE,
    RESULT_TYPE,
};
use connect_store::{Actor, Store};
use rvoip_auth_core::{BearerAuthError, BearerValidator};
use rvoip_core::{
    ids::{ConversationId, IdentityId, ParticipantId, SessionId, TenantId},
    session::SessionMedium,
    AuthenticatedPrincipal, IdentityAssurance, Orchestrator,
};
use rvoip_uctp::{
    application::{ApplicationContext, ApplicationError, ApplicationHandler},
    envelope::UctpEnvelope,
    state::{
        ResourceBindingError, SessionBindingResolver, UCTP_DATA_SCOPE, UCTP_SESSION_SCOPE,
        UCTP_SUBSCRIBE_SCOPE,
    },
    types::MessageType,
};
use std::sync::Arc;

pub struct Service {
    pub store: Arc<Store>,
    changed: tokio::sync::Notify,
    runtime: connect_store::DiscoveryRuntime,
    #[cfg(test)]
    wait_boundary: Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>,
}
impl Service {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Self::with_runtime(store, connect_store::DiscoveryRuntime::default())
    }
    /// Configure discovery from successfully constructed runtime listeners and adapters.
    pub fn with_runtime(store: Arc<Store>, runtime: connect_store::DiscoveryRuntime) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            store,
            changed: tokio::sync::Notify::new(),
            #[cfg(test)]
            wait_boundary: None,
        })
    }
    /// Register the wake hint before journal catch-up. Only a committed journal
    /// read proves delivery; wake hints may be coalesced or absent after restart.
    pub async fn execute(&self, credential_id: String, operation: Operation) -> Result<Outcome> {
        let wake = self.changed.notified();
        tokio::pin!(wake);
        wake.as_mut().enable();
        let result = self.run(credential_id.clone(), operation.clone()).await;
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
                        && (envelope.cid.is_none()
                            || envelope.cid.as_deref() == request.operation.conversation_id()) =>
                {
                    let operation = request.operation;
                    let exchange_ref = diagnostic_ref(&envelope.id);
                    let logical_ref = operation.request_id().map(diagnostic_ref);
                    let credential_ref = diagnostic_ref(&context.principal.subject);
                    let execution = self.execute(context.principal.subject, operation.clone());
                    let result = if matches!(operation, Operation::Inbox { .. }) {
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

pub struct StoreValidator {
    store: Arc<Store>,
    route_credential: Option<String>,
}
impl StoreValidator {
    pub fn new(store: Arc<Store>) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: None,
        })
    }
    /// Accept only the credential selected for an administrator-configured
    /// WebRTC route; other valid installation credentials cannot assume it.
    pub fn for_credential(store: Arc<Store>, credential_id: String) -> Arc<Self> {
        Arc::new(Self {
            store,
            route_credential: Some(credential_id),
        })
    }
    fn principal(
        &self,
        actor: Actor,
    ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
        let expires_at = chrono::DateTime::from_timestamp(actor.expires_at, 0)
            .ok_or_else(|| BearerAuthError::Invalid("expiry".into()))?;
        let mut scopes = vec![REQUIRED_SCOPE.into()];
        if actor.permits("session_get") {
            scopes.extend(
                [
                    UCTP_SESSION_SCOPE,
                    UCTP_DATA_SCOPE,
                    UCTP_SUBSCRIBE_SCOPE,
                    "webrtc:connect",
                ]
                .map(str::to_owned),
            );
        }
        let mut principal = AuthenticatedPrincipal::from_assurance(IdentityAssurance::TaskScoped {
            identity: IdentityId::from_string(actor.identity_id),
            task_id: "connect-credential".into(),
            scopes,
            expires_at,
        });
        principal.subject = actor.credential_id;
        principal.tenant = Some(
            self.store
                .installation_id()
                .map_err(|_| BearerAuthError::Unavailable("storage".into()))?,
        );
        principal.issuer = Some("connect-local".into());
        Ok(principal)
    }
}

/// Maps an authenticated UCTP wire Session to one durable Connect Session.
/// The resolver rechecks the credential and current conversation membership on
/// every binding/reauthorization; identical attacker-chosen IDs cannot escape
/// the installation namespace.
pub struct StoreSessionBindingResolver {
    store: Arc<Store>,
    orchestrator: Arc<Orchestrator>,
    namespace: String,
}
impl StoreSessionBindingResolver {
    pub fn new(store: Arc<Store>, orchestrator: Arc<Orchestrator>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            namespace: store.installation_id()?,
            store,
            orchestrator,
        }))
    }
    /// Revalidate a stored credential and project its durable Session for a
    /// server-owned outbound transport before any peer-visible signaling.
    pub fn project_credential(
        &self,
        credential_id: &str,
        wire_session: &str,
    ) -> std::result::Result<SessionId, ResourceBindingError> {
        let actor = self.store.revalidate(credential_id).map_err(|error| {
            if error.code == "storage-unavailable" {
                ResourceBindingError::unavailable("connect-session-authority-unavailable")
            } else {
                ResourceBindingError::forbidden("connect-session-authority-required")
            }
        })?;
        let session = self
            .store
            .authorize_session_binding(credential_id, wire_session)
            .map_err(|error| {
                if error.code == "storage-unavailable" {
                    ResourceBindingError::unavailable("connect-session-authority-unavailable")
                } else {
                    ResourceBindingError::forbidden("connect-session-authority-required")
                }
            })?;
        let canonical_session =
            SessionId::from_string(format!("connect:{}:{wire_session}", self.namespace));
        let canonical_conversation = ConversationId::from_string(format!(
            "connect:{}:{}",
            self.namespace, session.conversation_id
        ));
        let medium = match (
            session.media.iter().any(|kind| kind == "audio"),
            session.media.iter().any(|kind| kind == "video"),
        ) {
            (true, true) => SessionMedium::VoiceVideo,
            (true, false) => SessionMedium::Voice,
            (false, true) => SessionMedium::Video,
            _ => SessionMedium::Mixed,
        };
        self.orchestrator
            .project_external_session(
                canonical_conversation,
                canonical_session.clone(),
                TenantId::from_string(self.namespace.clone()),
                medium,
                [ParticipantId::from_string(actor.identity_id)],
            )
            .map_err(|_| ResourceBindingError::unavailable("connect-session-projection-failed"))?;
        Ok(canonical_session)
    }
    fn resolve(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
    ) -> std::result::Result<SessionId, ResourceBindingError> {
        let session = self
            .store
            .authorize_session_binding(&principal.subject, wire_session.as_str())
            .map_err(|error| {
                if error.code == "storage-unavailable" {
                    ResourceBindingError::unavailable("connect-session-authority-unavailable")
                } else {
                    ResourceBindingError::forbidden("connect-session-authority-required")
                }
            })?;
        let canonical_session =
            SessionId::from_string(format!("connect:{}:{}", self.namespace, wire_session));
        let canonical_conversation = ConversationId::from_string(format!(
            "connect:{}:{}",
            self.namespace, session.conversation_id
        ));
        let medium = match (
            session.media.iter().any(|kind| kind == "audio"),
            session.media.iter().any(|kind| kind == "video"),
        ) {
            (true, true) => SessionMedium::VoiceVideo,
            (true, false) => SessionMedium::Voice,
            (false, true) => SessionMedium::Video,
            _ => SessionMedium::Mixed,
        };
        self.orchestrator
            .project_external_session(
                canonical_conversation,
                canonical_session.clone(),
                TenantId::from_string(self.namespace.clone()),
                medium,
                [
                    ParticipantId::from_string(session.initiator_id),
                    ParticipantId::from_string(session.participant_id),
                ],
            )
            .map_err(|_| ResourceBindingError::unavailable("connect-session-projection-failed"))?;
        Ok(canonical_session)
    }
}
impl SessionBindingResolver for StoreSessionBindingResolver {
    fn resolve_session(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
    ) -> std::result::Result<SessionId, ResourceBindingError> {
        self.resolve(principal, wire_session)
    }
    fn reauthorize_session(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
        canonical_session: &SessionId,
    ) -> std::result::Result<(), ResourceBindingError> {
        if self.resolve(principal, wire_session)? == *canonical_session {
            Ok(())
        } else {
            Err(ResourceBindingError::forbidden("connect-session-mismatch"))
        }
    }
}
#[async_trait]
impl BearerValidator for StoreValidator {
    async fn validate(
        &self,
        token: &str,
    ) -> std::result::Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_principal(token).await?.assurance)
    }
    async fn validate_principal(
        &self,
        token: &str,
    ) -> std::result::Result<AuthenticatedPrincipal, BearerAuthError> {
        let store = self.store.clone();
        let token = token.to_owned();
        let actor = tokio::task::spawn_blocking(move || store.authenticate(&token))
            .await
            .map_err(|_| BearerAuthError::Unavailable("storage".into()))?
            .map_err(|e| {
                if e.code == "storage-unavailable" {
                    BearerAuthError::Unavailable("storage".into())
                } else {
                    BearerAuthError::Invalid("credential".into())
                }
            })?;
        if self
            .route_credential
            .as_ref()
            .is_some_and(|credential| credential != &actor.credential_id)
        {
            return Err(BearerAuthError::Invalid("route credential".into()));
        }
        self.principal(actor)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // Panics are deliberate assertions in these fixtures.
mod tests {
    use super::*;
    use connect_protocol::Limits;
    use connect_store::Scope;
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
                    participants: vec![agent.id],
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
            store,
            changed: tokio::sync::Notify::new(),
            wait_boundary: Some((reached.clone(), release.clone())),
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
                    body: "boundary message".into(),
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
