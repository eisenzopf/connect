//! Owned loopback listener; MCP sessions are never communication task identities.
use crate::{
    auth::{admit, Admission},
    handler::Handler,
    supervision::Observation,
    McpAuthenticator, McpCapacity,
};
use axum::{middleware, Router};
use connect_service::Service;
use futures_util::FutureExt;
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use std::{io, net::SocketAddr, panic::AssertUnwindSafe, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Running local MCP transport with explicit shutdown and no public ingress.
pub struct RunningMcp {
    /// Actual loopback address, including an allocated ephemeral port.
    pub address: SocketAddr,
    cancellation: CancellationToken,
    handle: Option<tokio::task::JoinHandle<io::Result<()>>>,
    readiness: Arc<connect_service::readiness::OperationalReadiness>,
    primary: bool,
}
impl RunningMcp {
    /// Start one stateless Streamable HTTP endpoint at `/mcp`. Legacy
    /// initialize/list/call is supported; newer optional MCP facilities are not
    /// advertised. Native and provider origins use separately supplied admission.
    /// This convenience creates an independent pool; multi-endpoint servers
    /// should use one shared [`McpCapacity`] instead.
    pub async fn start_local(
        service: Arc<Service>,
        authenticator: Arc<dyn McpAuthenticator>,
        bind: SocketAddr,
    ) -> io::Result<Self> {
        McpCapacity::default()
            .start_primary(service, authenticator, bind)
            .await
    }
    /// Start a lifecycle-owned task endpoint without changing the main MCP
    /// endpoint's readiness. Its unexpected exit still fails runtime supervision;
    /// normal task teardown must not declare another live endpoint unavailable.
    /// This convenience creates an independent pool; use [`McpCapacity`] to
    /// share aggregate resource limits with other endpoints.
    pub async fn start_local_auxiliary(
        service: Arc<Service>,
        authenticator: Arc<dyn McpAuthenticator>,
        bind: SocketAddr,
    ) -> io::Result<Self> {
        McpCapacity::default()
            .start_auxiliary(service, authenticator, bind)
            .await
    }
    pub(crate) async fn start(
        service: Arc<Service>,
        authenticator: Arc<dyn McpAuthenticator>,
        bind: SocketAddr,
        primary: bool,
        capacity: &McpCapacity,
    ) -> io::Result<Self> {
        if !bind.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MCP listener must be loopback",
            ));
        }
        let listener_slot = capacity
            .listeners
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "MCP listener capacity exhausted")
            })?;
        let listener = tokio::net::TcpListener::bind(bind).await?;
        let address = listener.local_addr()?;
        let cancellation = CancellationToken::new();
        let mut config = StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_allowed_hosts([address.to_string(), format!("localhost:{}", address.port())])
            .enforce_origin_validation()
            .with_cancellation_token(cancellation.clone());
        config.max_request_body_bytes = 512 * 1024;
        let handler_signal = cancellation.clone();
        let service_readiness = service.readiness.clone();
        let transport = StreamableHttpService::new(
            move || {
                Ok(Handler {
                    service: service.clone(),
                    cancellation: handler_signal.clone(),
                })
            },
            LocalSessionManager::default().into(),
            config,
        );
        let admission = Admission {
            authenticator,
            capacity: capacity.requests.clone(),
            cancellation: cancellation.clone(),
        };
        let app = Router::new()
            .route_service("/mcp", transport)
            .layer(middleware::from_fn_with_state(admission, admit));
        let signal = cancellation.clone();
        // Construct before spawn: even abort-before-first-poll drops this guard.
        let observation = Observation::new(service_readiness.clone(), signal.clone(), primary);
        let handle = tokio::spawn(async move {
            let _observation = observation;
            let _listener_slot = listener_slot;
            let run = async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(signal.cancelled_owned())
                    .await
            };
            AssertUnwindSafe(run)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(io::Error::other("MCP listener failed")))
        });
        Ok(Self {
            address,
            cancellation,
            handle: Some(handle),
            readiness: service_readiness,
            primary,
        })
    }
    /// Cancel transport work, drain within five seconds and report failure.
    pub async fn shutdown(mut self) -> io::Result<()> {
        crate::supervision::draining(&self.readiness, self.primary);
        self.cancellation.cancel();
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        // Dropping a JoinHandle detaches its task. A cancelled shutdown must
        // instead retain abort ownership until the listener has been joined.
        struct Owned(tokio::task::JoinHandle<io::Result<()>>);
        impl Drop for Owned {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let mut owned = Owned(handle);
        match tokio::time::timeout(Duration::from_secs(5), &mut owned.0).await {
            Ok(result) => result.map_err(io::Error::other)?,
            Err(_) => {
                owned.0.abort();
                let _ = (&mut owned.0).await;
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "MCP shutdown did not drain",
                ))
            }
        }
    }
}
impl Drop for RunningMcp {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}
#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
