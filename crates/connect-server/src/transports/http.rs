//! Bounded loopback HTTP facade sharing the service's authority and receipts.
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{header, request::Parts, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{post, put},
    Json, Router,
};
use connect_protocol::{Error, Operation, PROFILE};
use connect_service::Service;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, CorsLayer};

#[derive(Clone)]
pub(super) struct HttpState {
    pub(super) service: Arc<Service>,
    pub(super) verifier: Option<Arc<telnyx::webhooks::Verifier>>,
    pub(super) capacity: Arc<tokio::sync::Semaphore>,
    pub(super) cancellation: CancellationToken,
    pub(super) address: SocketAddr,
}

/// A bounded HTTP listener for local SDK calls or Telnyx callbacks.
pub struct Listener {
    /// Actual bound address, including the selected port when configured with zero.
    pub address: SocketAddr,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl Listener {
    /// Bind the complete loopback application facade.
    pub async fn bind(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, false).await
    }

    /// Bind only the callback route. A tunnel may expose this listener without
    /// exposing the application API.
    pub async fn bind_webhooks(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, true).await
    }

    async fn bind_mode(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
        webhooks_only: bool,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(address.ip().is_loopback(), "HTTP listener must be loopback");
        let verifier = public_key
            .map(telnyx::webhooks::Verifier::new)
            .transpose()
            .map_err(|_| anyhow::anyhow!("Invalid TELNYX_PUBLIC_KEY"))?
            .map(Arc::new);
        let capacity = Arc::new(tokio::sync::Semaphore::new(
            service.store.limits().connections,
        ));
        let stop = CancellationToken::new();
        let listener = tokio::net::TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let state = HttpState {
            service,
            verifier,
            capacity,
            cancellation: stop.clone(),
            address,
        };
        let access = Router::new()
            .route("/v1/task-access", post(super::task_access::exchange))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                super::task_access::admit,
            ))
            .layer(DefaultBodyLimit::max(65_536));
        let app = Router::new().route("/webhooks/telnyx", post(super::telnyx_http::webhook));
        let app = if webhooks_only {
            app
        } else {
            app.route("/v1/operations", post(operation))
                .merge(access)
                .route(
                    "/v1/conversations/{conversation_id}/artifacts/{artifact_id}",
                    put(super::artifact_http::put_chunk).get(super::artifact_http::get_chunk),
                )
                .layer(browser_cors())
        };
        let body_limit = state
            .service
            .store
            .limits()
            .artifact_chunk_bytes
            .max(65_536) as usize;
        let app = app
            .layer(DefaultBodyLimit::max(body_limit))
            .with_state(state);
        let token = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(token.cancelled_owned())
                .await
        });
        Ok(Self {
            address,
            stop,
            task: Some(task),
        })
    }

    /// Stop accepting requests and wait for a bounded graceful shutdown.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(6), &mut task).await {
                Ok(result) => result??,
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    anyhow::bail!("HTTP shutdown timed out");
                }
            }
        }
        Ok(())
    }
}

/// Browser SDK access is intentionally limited to ordinary HTTP origins on
/// this machine. Bearer credentials must never be exposed through a wildcard
/// origin, and the independently exposed webhook listener does not need CORS.
fn browser_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(
            |origin: &HeaderValue, _request: &Parts| is_loopback_origin(origin),
        ))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .expose_headers([
            HeaderName::from_static("x-connect-offset"),
            HeaderName::from_static("x-connect-eof"),
        ])
        .max_age(Duration::from_secs(600))
}

fn is_loopback_origin(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    if uri.scheme_str() != Some("http") || uri.authority().is_none() {
        return false;
    }
    matches!(
        uri.host(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    )
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(super) fn error(status: StatusCode, error: Error) -> Response {
    (
        status,
        Json(connect_protocol::Response {
            profile: PROFILE.into(),
            result: Err(error),
        }),
    )
        .into_response()
}

pub(super) async fn authenticated_credential(
    state: &HttpState,
    headers: &HeaderMap,
) -> Result<String, Response> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(error(StatusCode::UNAUTHORIZED, Error::unauthorized()));
    }
    let Some(token) = value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty() && value.len() <= 2048 && value.is_ascii())
        .map(str::to_owned)
    else {
        return Err(error(StatusCode::UNAUTHORIZED, Error::unauthorized()));
    };
    let store = state.service.store.clone();
    match tokio::task::spawn_blocking(move || store.authenticate(&token)).await {
        Ok(Ok(actor)) => Ok(actor.credential_id),
        _ => Err(error(StatusCode::UNAUTHORIZED, Error::unauthorized())),
    }
}

async fn operation(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(operation): Json<Operation>,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            Error::new("capacity-exceeded", "HTTP concurrency limit reached"),
        );
    };
    let credential = match authenticated_credential(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let result = state.service.execute(credential, operation).await;
    let status = match &result {
        Err(value) if value.code == "unauthorized" => StatusCode::UNAUTHORIZED,
        Err(value) if value.code == "forbidden" => StatusCode::FORBIDDEN,
        _ => StatusCode::OK,
    };
    (
        status,
        Json(connect_protocol::Response {
            profile: PROFILE.into(),
            result,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::is_loopback_origin;
    use axum::http::HeaderValue;

    #[test]
    fn browser_origins_are_limited_to_loopback_http() {
        for allowed in [
            "http://localhost:8192",
            "http://127.0.0.1:8192",
            "http://[::1]:8192",
        ] {
            let origin = HeaderValue::from_str(allowed).expect("valid origin");
            assert!(is_loopback_origin(&origin), "should allow {allowed}");
        }

        for denied in [
            "https://localhost:8192",
            "http://localhost.example:8192",
            "http://127.0.0.2:8192",
            "https://example.com",
            "null",
        ] {
            let origin = HeaderValue::from_str(denied).expect("valid header");
            assert!(!is_loopback_origin(&origin), "should deny {denied}");
        }
    }
}
