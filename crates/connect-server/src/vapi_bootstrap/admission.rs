//! Authenticate setup before headers; verify provider creation before granting.
use super::state::Locked;
use async_trait::async_trait;
use axum::http::HeaderMap;
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_mcp::{McpAuthenticator, VapiOrigin};
use connect_protocol::{Error, Result};
use connect_service::{tasks::TaskInvocation, Service};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{path::PathBuf, sync::Arc};

/// Compose one protected first-invocation endpoint. Its credential is never
/// shared across tasks/assistants or interpreted as ordinary task authority.
pub struct BootstrapAuth {
    service: Arc<Service>,
    provider: Arc<Vapi>,
    directory: PathBuf,
}
impl BootstrapAuth {
    /// Compose before opening the listener, so preparation can save its actual
    /// URL. Missing/unprepared state denies requests; headers never mint it.
    pub fn new(service: Arc<Service>, provider: Arc<Vapi>, directory: PathBuf) -> Self {
        Self {
            service,
            provider,
            directory,
        }
    }
}
#[async_trait]
impl McpAuthenticator for BootstrapAuth {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<TaskInvocation> {
        let mut held = Locked::load(&self.directory).map_err(|_| Error::forbidden())?;
        let state = &held.state;
        let credential = bearer(headers)?;
        // HMAC's verify_slice compares fixed-size tags in constant time. The
        // domain key is public; this is comparison, not a substitute credential.
        let tag = |value: &str| -> Result<Hmac<Sha256>> {
            let mut mac = Hmac::<Sha256>::new_from_slice(b"connect-vapi-bootstrap-compare/1")
                .map_err(|_| Error::storage())?;
            mac.update(value.as_bytes());
            Ok(mac)
        };
        tag(credential)?
            .verify_slice(&tag(&state.secret)?.finalize().into_bytes())
            .map_err(|_| Error::unauthorized())?;
        let origin = match state.definition.origin {
            OriginKind::Call => VapiOrigin::Call,
            OriginKind::Chat => VapiOrigin::Chat,
            OriginKind::ChatSession => VapiOrigin::ChatSession,
        };
        let external = origin.correlate(headers)?;
        super::binding::bind(&self.service, &self.provider, &mut held, external).await
    }
}
fn bearer(headers: &HeaderMap) -> Result<&str> {
    let mut values = headers.get_all("authorization").iter();
    let token = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(Error::unauthorized)?;
    if values.next().is_some() {
        return Err(Error::unauthorized());
    }
    Ok(token)
}
