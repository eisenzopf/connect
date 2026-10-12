//! Protected setup for an already-created, exact Vapi invocation resource.
use connect_connectors::{
    vapi_origin::{OriginKind, OriginVerificationError},
    Vapi,
};
use connect_mcp::{VapiBindingAuth, VapiOrigin};
use connect_protocol::{
    tasks::{GrantDefinition, InvocationBinding, InvocationKind, TaskPermission},
    Error, Result,
};
use connect_service::{tasks::IssuedTaskGrant, Service};
use std::sync::Arc;

/// Developer-owned setup, separate from model parameters and correlation headers.
/// A resource must already exist; creation/uncertain creation and first-request
/// bootstrap are separate lifecycle steps, not inferred from an ID supplied here.
pub struct VapiGrantSetup {
    /// Current backing credential reference, never its plaintext token.
    pub credential_id: String,
    /// Stable issuance intent retained across response uncertainty.
    pub request_id: String,
    /// Exact existing task selected by the developer.
    pub task_id: String,
    /// Exact provider origin mode.
    pub origin: OriginKind,
    /// Existing call/chat/session selected through protected setup.
    pub external_id: String,
    /// Saved assistant expected by the protected configuration.
    pub assistant_id: String,
    /// Explicit subset of task operations for this invocation.
    pub operations: Vec<TaskPermission>,
    /// Original bounded expiry, never extended on replay.
    pub expires_at: i64,
}

/// Intended protected credential returned only after private verification and
/// current atomic issuance. It cannot be constructed from model JSON.
pub struct VerifiedVapiTaskGrant {
    issued: IssuedTaskGrant,
    origin: VapiOrigin,
}
impl VerifiedVapiTaskGrant {
    /// Explicit protected export. Never log the bearer or install it globally.
    pub fn issued(&self) -> &IssuedTaskGrant {
        &self.issued
    }
    /// Compose correlation admission for exactly the verified registered origin.
    /// Current task/grant authority is rechecked on every MCP request.
    pub fn authenticator(&self, service: Arc<Service>) -> Result<VapiBindingAuth> {
        VapiBindingAuth::new(service, self.issued.grant.binding.clone(), self.origin)
    }
}

/// Preflight current authority, perform the private SDK lookup, then atomically
/// register and issue under current authority again. No remote resource is
/// created and no model is invoked. A failed lookup cannot register a binding.
pub async fn issue(
    service: Arc<Service>,
    provider: &Vapi,
    setup: VapiGrantSetup,
) -> Result<VerifiedVapiTaskGrant> {
    let (kind, adapter, origin) = match setup.origin {
        OriginKind::Call => (InvocationKind::ProviderCall, "vapi-call", VapiOrigin::Call),
        OriginKind::Chat => (InvocationKind::ProviderChat, "vapi-chat", VapiOrigin::Chat),
        OriginKind::ChatSession => (
            InvocationKind::ProviderChat,
            "vapi-chat-session",
            VapiOrigin::ChatSession,
        ),
    };
    let definition = GrantDefinition {
        task_id: setup.task_id,
        binding: InvocationBinding {
            kind,
            adapter: adapter.into(),
            external_id: setup.external_id.clone(),
        },
        operations: setup.operations,
        expires_at: setup.expires_at,
    };
    service
        .authorize_provider_task_setup(
            setup.credential_id.clone(),
            setup.request_id.clone(),
            definition.clone(),
        )
        .await?;
    // Only this read-only verified private-API result permits the final step.
    let _proof = provider
        .verify_origin(setup.origin, &setup.external_id, &setup.assistant_id)
        .await
        .map_err(verification_error)?;
    let issued = service
        .issue_verified_provider_task_grant(setup.credential_id, setup.request_id, definition)
        .await?;
    Ok(VerifiedVapiTaskGrant { issued, origin })
}

fn verification_error(error: OriginVerificationError) -> Error {
    let code = match error {
        OriginVerificationError::Unavailable => "provider-origin-unavailable",
        OriginVerificationError::InvalidIdentifier => "invalid-request",
        OriginVerificationError::Mismatch | OriginVerificationError::Inactive => {
            "provider-origin-denied"
        }
    };
    Error::new(code, &error.to_string())
}
