//! Owned loopback listener; MCP sessions are never communication task identities.
use crate::{
    auth::{admit, Admission},
    handler::Handler,
    McpAuthenticator,
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
}
impl RunningMcp {
    /// Start one stateless Streamable HTTP endpoint at `/mcp`. Legacy
    /// initialize/list/call is supported; newer optional MCP facilities are not
    /// advertised. Native and provider origins use separately supplied admission.
    pub async fn start_local(
        service: Arc<Service>,
        authenticator: Arc<dyn McpAuthenticator>,
        bind: SocketAddr,
    ) -> io::Result<Self> {
        if !bind.ip().is_loopback() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "MCP listener must be loopback",
            ));
        }
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
            capacity: Arc::new(tokio::sync::Semaphore::new(32)),
            cancellation: cancellation.clone(),
        };
        let app = Router::new()
            .route_service("/mcp", transport)
            .layer(middleware::from_fn_with_state(admission, admit));
        let signal = cancellation.clone();
        let readiness = service_readiness;
        readiness.set(
            "mcp",
            connect_protocol::ReadinessState::Ready,
            "listener-bound",
        );
        let handle = tokio::spawn(async move {
            let expected_stop = signal.clone();
            let run = async move {
                axum::serve(listener, app)
                    .with_graceful_shutdown(signal.cancelled_owned())
                    .await
            };
            let result = AssertUnwindSafe(run)
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(io::Error::other("MCP listener failed")));
            if !expected_stop.is_cancelled() {
                readiness.fail("mcp");
            } else {
                readiness.set(
                    "mcp",
                    connect_protocol::ReadinessState::Unavailable,
                    "listener-stopped",
                );
            }
            result
        });
        Ok(Self {
            address,
            cancellation,
            handle: Some(handle),
        })
    }
    /// Cancel transport work, drain within five seconds and report failure.
    pub async fn shutdown(mut self) -> io::Result<()> {
        self.cancellation.cancel();
        let Some(mut handle) = self.handle.take() else {
            return Ok(());
        };
        match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
            Ok(result) => result.map_err(io::Error::other)?,
            Err(_) => {
                handle.abort();
                let _ = handle.await;
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
