//! Admission precedes model arguments and provider correlation headers.
use async_trait::async_trait;
use axum::{
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use connect_protocol::{
    tasks::{InvocationBinding, InvocationKind},
    Error,
};
use connect_service::{tasks::TaskInvocation, Service};
use std::{sync::Arc, time::Duration};

/// Trusted origin adapter. Returned handles have private service/store fields
/// and every tool execution revalidates their current durable authority.
#[async_trait]
pub trait McpAuthenticator: Send + Sync {
    /// Authenticate configured narrow authority before interpreting any provider
    /// correlation headers. Headers or model parameters never mint a binding.
    async fn authenticate(&self, headers: &HeaderMap) -> Result<TaskInvocation, Error>;
}

/// Protected native/host installation with one explicitly registered binding.
/// Provider calls/chats require their own verified lifecycle authenticator.
pub struct FixedBindingAuth {
    service: Arc<Service>,
    binding: InvocationBinding,
}
impl FixedBindingAuth {
    /// Composition must register this origin and issue its grant through the
    /// trusted service before exporting the secret to protected host config.
    pub fn new(service: Arc<Service>, binding: InvocationBinding) -> Result<Self, Error> {
        if !matches!(
            binding.kind,
            InvocationKind::Native | InvocationKind::HostSession
        ) {
            return Err(Error::forbidden());
        }
        Ok(Self { service, binding })
    }
}
#[async_trait]
impl McpAuthenticator for FixedBindingAuth {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<TaskInvocation, Error> {
        let token = bearer(headers)?;
        let invocation = self
            .service
            .authenticate_task_grant(token.to_owned(), self.binding.clone())
            .await?;
        // Correlation is not a substitute for this installation's binding.
        if headers.contains_key("x-call-id") || headers.contains_key("x-chat-id") {
            return Err(Error::forbidden());
        }
        Ok(invocation)
    }
}

fn bearer(headers: &HeaderMap) -> Result<&str, Error> {
    let mut values = headers.get_all("authorization").iter();
    let value = values.next().ok_or_else(Error::forbidden)?;
    if values.next().is_some() {
        return Err(Error::forbidden());
    }
    let token = value
        .to_str()
        .ok()
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty() && token.len() <= 2048 && token.is_ascii())
        .ok_or_else(Error::forbidden)?;
    Ok(token)
}

#[derive(Clone)]
pub(crate) struct Admission {
    pub authenticator: Arc<dyn McpAuthenticator>,
    pub capacity: Arc<tokio::sync::Semaphore>,
    pub cancellation: tokio_util::sync::CancellationToken,
}
pub(crate) async fn admit(
    State(state): State<Admission>,
    mut request: Request,
    next: Next,
) -> Response {
    let Ok(_permit) = state.capacity.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let admission = tokio::select! {
        _ = state.cancellation.cancelled() => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        result = tokio::time::timeout(Duration::from_secs(5), state.authenticator.authenticate(request.headers())) => result,
    };
    let invocation = match admission {
        Ok(Ok(invocation)) => invocation,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    // The SDK receives a verified handle, never bearer plaintext in HTTP Parts.
    request.headers_mut().remove("authorization");
    request.extensions_mut().insert(invocation);
    // Also cancel body decoding, not just authentication and parsed tool work.
    // A client can pause a chunked body before the SDK creates request context.
    tokio::select! {
        _ = state.cancellation.cancelled() => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        response = next.run(request) => response,
    }
}
