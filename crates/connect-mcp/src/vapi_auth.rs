//! Vapi correlation checks follow authentication of an already registered origin.
use crate::{auth::bearer, McpAuthenticator};
use async_trait::async_trait;
use axum::http::HeaderMap;
use connect_protocol::{
    tasks::{InvocationBinding, InvocationKind, MAX_TASK_ID_BYTES},
    Error,
};
use connect_service::{tasks::TaskInvocation, Service};
use std::sync::Arc;

/// The provider resource whose verified ID is stored in the invocation binding.
///
/// A session binding permits the chats in that one Vapi session, not unrelated
/// chats. A standalone chat binding permits only its exact chat ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VapiOrigin {
    /// A voice call; `external_id` is the verified Vapi call ID.
    Call,
    /// A standalone chat; `external_id` is the verified Vapi chat ID.
    Chat,
    /// Session-based chats; `external_id` is the verified Vapi session ID.
    ChatSession,
}

/// Fixed provider-origin admission using a separate narrow grant per binding.
///
/// Trusted composition must verify the provider resource and register its
/// binding before constructing this adapter. This adapter cannot verify a
/// webhook, look up a provider resource, register an origin or issue a grant.
/// Configure its grant in the intended invocation's protected MCP headers,
/// never as a shared assistant-wide credential. Correlation headers do not
/// select a grant, task, actor or conversation.
pub struct VapiBindingAuth {
    service: Arc<Service>,
    binding: InvocationBinding,
    origin: VapiOrigin,
}

impl VapiBindingAuth {
    /// Configure one previously verified and registered provider resource.
    /// Local/native and host-session bindings are rejected.
    pub fn new(
        service: Arc<Service>,
        binding: InvocationBinding,
        origin: VapiOrigin,
    ) -> Result<Self, Error> {
        binding.validate()?;
        let expected = match origin {
            VapiOrigin::Call => InvocationKind::ProviderCall,
            VapiOrigin::Chat | VapiOrigin::ChatSession => InvocationKind::ProviderChat,
        };
        if binding.kind != expected {
            return Err(Error::forbidden());
        }
        Ok(Self {
            service,
            binding,
            origin,
        })
    }

    fn correlate(&self, headers: &HeaderMap) -> Result<(), Error> {
        let external = match self.origin {
            VapiOrigin::Call => {
                absent(headers, &["x-chat-id", "x-session-id"])?;
                identifier(headers, "x-call-id")?
            }
            VapiOrigin::Chat => {
                absent(headers, &["x-call-id", "x-session-id"])?;
                identifier(headers, "x-chat-id")?
            }
            VapiOrigin::ChatSession => {
                absent(headers, &["x-call-id"])?;
                // The chat ID is correlation within this authorized session.
                // It is deliberately not interpreted as another authority.
                identifier(headers, "x-chat-id")?;
                identifier(headers, "x-session-id")?
            }
        };
        if external != self.binding.external_id {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}

#[async_trait]
impl McpAuthenticator for VapiBindingAuth {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<TaskInvocation, Error> {
        let token = bearer(headers)?;
        // Current durable grant/task/issuer authority is checked before any
        // provider header is interpreted. The registered binding is fixed.
        let invocation = self
            .service
            .authenticate_task_grant(token.to_owned(), self.binding.clone())
            .await?;
        self.correlate(headers)?;
        Ok(invocation)
    }
}

fn absent(headers: &HeaderMap, names: &[&str]) -> Result<(), Error> {
    if names.iter().any(|name| headers.contains_key(*name)) {
        return Err(Error::forbidden());
    }
    Ok(())
}

fn identifier<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, Error> {
    let mut values = headers.get_all(name).iter();
    let value = values.next().ok_or_else(Error::forbidden)?;
    if values.next().is_some() {
        return Err(Error::forbidden());
    }
    value
        .to_str()
        .ok()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_TASK_ID_BYTES
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
        })
        .ok_or_else(Error::forbidden)
}
