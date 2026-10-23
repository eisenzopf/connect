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
    pub(super) provider_routes: Option<Arc<Vec<String>>>,
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
    _retirement: Arc<dyn connect_store::RuntimeRetirementSignal>,
}

struct Retirement(CancellationToken);
impl connect_store::RuntimeRetirementSignal for Retirement {
    fn retire(&self) {
        self.0.cancel()
    }
}
impl Listener {
    pub(crate) fn begin_shutdown(&self) {
        self.stop.cancel()
    }
    /// Bind the complete loopback application facade.
    pub async fn bind(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, false, None).await
    }

    /// Bind only the callback route. A tunnel may expose this listener without
    /// exposing the application API.
    pub async fn bind_webhooks(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, true, None).await
    }

    pub(crate) async fn bind_canonical_webhooks(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: &str,
        routes: Vec<String>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !routes.is_empty(),
            "Canonical callbacks require selected routes"
        );
        Self::bind_mode(
            address,
            service,
            Some(public_key),
            true,
            Some(Arc::new(routes)),
        )
        .await
    }

    async fn bind_mode(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
        webhooks_only: bool,
        provider_routes: Option<Arc<Vec<String>>>,
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
        let retirement: Arc<dyn connect_store::RuntimeRetirementSignal> =
            Arc::new(Retirement(stop.clone()));
        service.store.register_runtime_retirement(&retirement)?;
        let listener = tokio::net::TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let state = HttpState {
            provider_routes,
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
        let administration = Router::new()
            .route(
                "/v2/incoming-contacts",
                post(super::incoming_contacts::command),
            )
            .route("/v2/identities", post(super::provisioning::identity))
            .route(
                "/v2/credentials/{id}/revoke",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/identities/{id}/record-read-role",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/identities/{id}/signing-keys",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/signing-keys/{id}/revoke",
                post(super::provisioning::scoped),
            )
            .route("/v2/devices/{id}/revoke", post(super::provisioning::scoped))
            .route(
                "/v2/identities/{id}/endpoints",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/identities/{id}/revoke",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/conversations/{id}/participants",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/endpoints/{id}/verify",
                post(super::provisioning::scoped),
            )
            .route(
                "/v2/endpoints/{id}/revoke",
                post(super::provisioning::scoped),
            )
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                super::provisioning::admit,
            ))
            .layer(DefaultBodyLimit::max(65_536));
        let app = Router::new().route("/webhooks/telnyx", post(super::telnyx_http::webhook));
        let app = if webhooks_only {
            app
        } else {
            app.route("/v1/operations", post(operation))
                .merge(super::canonical_http::router(state.clone()))
                .merge(super::record_mutations::router(state.clone()))
                .merge(super::user_delegation::router(state.clone()))
                .merge(access)
                .merge(administration)
                .route(
                    "/v1/conversations/{conversation_id}/artifacts/{artifact_id}",
                    put(super::artifact_http::put_chunk).get(super::artifact_http::get_chunk),
                )
                .route(
                    "/v1/conversations/{conversation_id}/artifacts/{artifact_id}/links/{token}",
                    put(super::artifact_http::put_link_chunk)
                        .get(super::artifact_http::get_link_chunk),
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
            let guarded = super::http_record_io::GuardedListener {
                listener,
                cancellation: token.clone(),
                tls: None,
                handshakes: Arc::new(tokio::sync::Semaphore::new(1)),
            };
            axum::serve(
                guarded,
                app.into_make_service_with_connect_info::<super::http_record_io::Custody>(),
            )
            .with_graceful_shutdown(token.cancelled_owned())
            .await
        });
        Ok(Self {
            address,
            stop,
            task: Some(task),
            _retirement: retirement,
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
    let token = bearer_token(headers).map_err(|value| error(StatusCode::UNAUTHORIZED, value))?;
    let store = state.service.store.clone();
    match tokio::task::spawn_blocking(move || store.authenticate(&token)).await {
        Ok(Ok(actor)) => Ok(actor.credential_id),
        _ => Err(error(StatusCode::UNAUTHORIZED, Error::unauthorized())),
    }
}

pub(super) fn bearer_token(headers: &HeaderMap) -> Result<String, Error> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(Error::unauthorized());
    }
    let Some(token) = value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty() && value.len() <= 2048 && value.is_ascii())
        .map(str::to_owned)
    else {
        return Err(Error::unauthorized());
    };
    Ok(token)
}

async fn authenticated_application(
    state: &HttpState,
    headers: &HeaderMap,
) -> Result<rvoip_core::AuthenticatedPrincipal, Response> {
    use rvoip_auth_core::BearerValidator;
    let token = bearer_token(headers).map_err(|value| error(StatusCode::UNAUTHORIZED, value))?;
    let principal = connect_service::StoreValidator::new(state.service.store.clone())
        .validate_principal(&token)
        .await
        .map_err(|_| error(StatusCode::UNAUTHORIZED, Error::unauthorized()))?;
    if principal
        .subject
        .starts_with(connect_protocol::tasks::TASK_PRINCIPAL_PREFIX)
        && (headers.contains_key("x-call-id") || headers.contains_key("x-chat-id"))
    {
        return Err(error(StatusCode::FORBIDDEN, Error::forbidden()));
    }
    Ok(principal)
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
    let credential = match authenticated_application(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let result = if connect_protocol::application_api::response_kind(&operation).is_none() {
        Err(Error::new(
            "unsupported-command",
            "Use the canonical /v2/commands API",
        ))
    } else {
        state
            .service
            .execute_authenticated(credential, operation)
            .await
    };
    let status = match &result {
        Err(value) if value.code == "unsupported-command" => StatusCode::NOT_IMPLEMENTED,
        Err(value) if value.code == "unauthorized" => StatusCode::UNAUTHORIZED,
        Err(value) if value.code == "forbidden" => StatusCode::FORBIDDEN,
        Err(value)
            if matches!(
                value.code.as_str(),
                "authority-busy" | "authority-unavailable" | "storage-unavailable"
            ) =>
        {
            StatusCode::SERVICE_UNAVAILABLE
        }
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
