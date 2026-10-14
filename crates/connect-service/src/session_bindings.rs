//! Durable Session projection keeps resource authority at the store boundary.
use connect_protocol::{tasks::TASK_PRINCIPAL_PREFIX, Error, Result, Session};
use connect_store::Store;
use rvoip_core::{
    ids::{ConversationId, ParticipantId, SessionId, TenantId},
    session::SessionMedium,
    AuthenticatedPrincipal, Orchestrator,
};
use rvoip_uctp::state::{ResourceBindingError, SessionBindingResolver};
use std::sync::Arc;

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
    /// Compose the current durable authority boundary and core media namespace.
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
        let actor = self
            .store
            .revalidate(credential_id)
            .map_err(resource_error)?;
        let session = self
            .store
            .authorize_session_binding(credential_id, wire_session)
            .map_err(resource_error)?;
        self.project(&session, [ParticipantId::from_string(actor.identity_id)])
    }
    fn project(
        &self,
        session: &Session,
        participants: impl IntoIterator<Item = ParticipantId>,
    ) -> std::result::Result<SessionId, ResourceBindingError> {
        let wire_session = &session.id;
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
                participants,
            )
            .map_err(|_| ResourceBindingError::unavailable("connect-session-projection-failed"))?;
        Ok(canonical_session)
    }
    fn resolve(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
    ) -> std::result::Result<SessionId, ResourceBindingError> {
        let session = if principal.subject.starts_with(TASK_PRINCIPAL_PREFIX) {
            if principal.issuer.as_deref() != Some("connect-local")
                || principal.tenant.as_deref() != Some(self.namespace.as_str())
                || principal.is_expired()
            {
                return Err(resource_error(Error::forbidden()));
            }
            self.store
                .authorize_native_task_session_binding(&principal.subject, wire_session.as_str())
        } else {
            self.store
                .authorize_session_binding(&principal.subject, wire_session.as_str())
        }
        .map_err(resource_error)?;
        self.project(
            &session,
            [
                ParticipantId::from_string(session.initiator_id.clone()),
                ParticipantId::from_string(session.participant_id.clone()),
            ],
        )
    }
}

#[cfg(test)]
#[path = "session_binding_tests.rs"]
mod tests;
impl SessionBindingResolver for StoreSessionBindingResolver {
    fn supports_terminal_session_retirement(&self) -> bool {
        true
    }

    fn reauthorize_peer(
        &self,
        principal: &AuthenticatedPrincipal,
    ) -> std::result::Result<(), ResourceBindingError> {
        if principal.issuer.as_deref() != Some("connect-local")
            || principal.tenant.as_deref() != Some(self.namespace.as_str())
            || principal.is_expired()
        {
            return Err(resource_error(Error::forbidden()));
        }
        if principal.subject.starts_with(TASK_PRINCIPAL_PREFIX) {
            self.store
                .revalidate_native_task_principal(&principal.subject)
                .map(|_| ())
        } else {
            self.store.revalidate(&principal.subject).map(|_| ())
        }
        .map_err(resource_error)
    }

    fn session_is_terminal(
        &self,
        principal: &AuthenticatedPrincipal,
        wire_session: &SessionId,
        canonical_session: &SessionId,
    ) -> std::result::Result<bool, ResourceBindingError> {
        self.reauthorize_peer(principal)?;
        let session = if principal.subject.starts_with(TASK_PRINCIPAL_PREFIX) {
            self.store
                .authorize_native_task_session_lifetime(&principal.subject, wire_session.as_str())
        } else {
            self.store
                .authorize_session_lifetime(&principal.subject, wire_session.as_str())
        }
        .map_err(resource_error)?;
        let expected = format!("connect:{}:{}", self.namespace, session.id);
        if expected != canonical_session.as_str() {
            return Err(ResourceBindingError::forbidden("connect-session-mismatch"));
        }
        Ok(session.state == "ended")
    }

    fn authorize_envelope(
        &self,
        principal: &AuthenticatedPrincipal,
        envelope: &rvoip_uctp::envelope::UctpEnvelope,
    ) -> std::result::Result<(), ResourceBindingError> {
        if principal.subject.starts_with(TASK_PRINCIPAL_PREFIX) {
            super::native_media::authorize(&self.store, &self.namespace, principal, envelope)
        } else {
            super::native_media::authorize_owner_video(&self.store, principal, envelope)
        }
    }
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

pub(crate) fn resource_error(error: Error) -> ResourceBindingError {
    if error.code == "storage-unavailable" {
        ResourceBindingError::unavailable("connect-session-authority-unavailable")
    } else {
        ResourceBindingError::forbidden("connect-session-authority-required")
    }
}
