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
    pub(super) canonical_discovery:
        Option<Arc<connect_protocol::canonical_discovery::TransportDiscovery>>,
    pub(super) service: Arc<Service>,
    pub(super) provider_routes: Option<Arc<Vec<String>>>,
    pub(super) verifier: Option<Arc<telnyx::webhooks::Verifier>>,
    pub(super) capacity: Arc<tokio::sync::Semaphore>,
    pub(super) cancellation: CancellationToken,
    pub(super) address: SocketAddr,
    pub(super) public_authority: Option<String>,
}
impl HttpState {
    pub(super) fn host_allowed(&self, headers: &HeaderMap) -> bool {
        let mut hosts = headers.get_all(header::HOST).iter();
        let host = hosts.next().and_then(|h| h.to_str().ok());
        hosts.next().is_none()
            && host.is_some_and(|host| match &self.public_authority {
                Some(authority) => host == authority,
                None => {
                    host == self.address.to_string()
                        || host == format!("localhost:{}", self.address.port())
                }
            })
    }
}

/// A bounded HTTP listener for local SDK calls or Telnyx callbacks.
pub struct Listener {
    /// Actual bound address, including the selected port when configured with zero.
    pub address: SocketAddr,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
    _retirement: Arc<dyn connect_store::RuntimeRetirementSignal>,
}

struct Retirement(CancellationToken);
impl connect_store::RuntimeRetirementSignal for Retirement {
    fn retire(&self) {
        self.0.cancel()
    }
}
impl Listener {
    pub(crate) async fn bind_https(
        config: &crate::hosting::HttpsListenerConfig,
        service: Arc<Service>,
        discovery: connect_protocol::canonical_discovery::TransportDiscovery,
        mut tls: rustls::ServerConfig,
    ) -> anyhow::Result<Self> {
        let listener = tokio::net::TcpListener::bind(config.bind).await?;
        let address = listener.local_addr()?;
        let origin = config.contact(address);
        let authority = origin
            .strip_prefix("https://")
            .expect("validated HTTPS origin")
            .to_owned();
        let maximum = service.store.limits().connections;
        let readiness = service.readiness.clone();
        let stop = CancellationToken::new();
        let retirement: Arc<dyn connect_store::RuntimeRetirementSignal> =
            Arc::new(Retirement(stop.clone()));
        service.store.register_runtime_retirement(&retirement)?;
        let verifier = config
            .telnyx_callbacks
            .as_ref()
            .map(|c| telnyx::webhooks::Verifier::new(&c.public_key))
            .transpose()
            .map_err(|_| anyhow::anyhow!("Invalid callback verifier"))?
            .map(Arc::new);
        let state = HttpState {
            canonical_discovery: Some(Arc::new(discovery)),
            service,
            provider_routes: config
                .telnyx_callbacks
                .as_ref()
                .map(|c| Arc::new(c.routes.clone())),
            verifier,
            capacity: Arc::new(tokio::sync::Semaphore::new(maximum)),
            cancellation: stop.clone(),
            address,
            public_authority: Some(authority.clone()),
        };
        let body_limit = state
            .service
            .store
            .limits()
            .artifact_chunk_bytes
            .max(65_536) as usize;
        let mut app = routes(state.clone(), Some(config.surface), false);
        if config.surface == crate::hosting::HttpSurface::Application {
            let allowed = config
                .allowed_origins
                .iter()
                .map(|o| o.parse::<HeaderValue>())
                .collect::<Result<Vec<_>, _>>()?;
            app = app.layer(
                CorsLayer::new()
                    .allow_origin(allowed)
                    .allow_methods([Method::GET, Method::POST, Method::PUT])
                    .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
                    .expose_headers([
                        HeaderName::from_static("x-connect-offset"),
                        HeaderName::from_static("x-connect-eof"),
                    ]),
            );
        }
        let app = app
            .layer(DefaultBodyLimit::max(body_limit))
            .layer(axum::middleware::from_fn_with_state(
                Arc::new(super::hosting_policy::Policy {
                    config: config.clone(),
                    authority,
                }),
                super::hosting_policy::admit,
            ))
            .with_state(state);
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let token = stop.clone();
        let component = match config.surface {
            crate::hosting::HttpSurface::Application => "https-application",
            crate::hosting::HttpSurface::Administration => "https-administration",
            crate::hosting::HttpSurface::Callbacks => "https-callbacks",
            crate::hosting::HttpSurface::Health => "https-health",
        };
        let task = crate::worker::spawn(readiness, component, stop.clone(), async move {
            let io = super::http_record_io::GuardedListener {
                listener,
                cancellation: token.clone(),
                tls: Some(Arc::new(tls)),
                handshakes: Arc::new(tokio::sync::Semaphore::new(maximum)),
                connections: Arc::new(tokio::sync::Semaphore::new(maximum)),
            };
            axum::serve(
                io,
                app.into_make_service_with_connect_info::<super::http_record_io::Custody>(),
            )
            .with_graceful_shutdown(token.cancelled_owned())
            .await?;
            Ok(())
        });
        Ok(Self {
            address,
            stop,
            task: Some(task),
            _retirement: retirement,
        })
    }
    pub(crate) fn begin_shutdown(&self) {
        self.stop.cancel()
    }
    /// Bind the complete loopback application facade.
    pub async fn bind(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, false, None, None).await
    }

    /// Bind only the callback route. A tunnel may expose this listener without
    /// exposing the application API.
    pub async fn bind_webhooks(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, true, None, None).await
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
            None,
        )
        .await
    }

    /// Protected canonical composition advertises only its configured listener.
    pub(crate) async fn bind_canonical(
        address: SocketAddr,
        service: Arc<Service>,
        discovery: connect_protocol::canonical_discovery::TransportDiscovery,
    ) -> anyhow::Result<Self> {
        discovery.validate()?;
        Self::bind_mode(
            address,
            service,
            None,
            false,
            None,
            Some(Arc::new(discovery)),
        )
        .await
    }

    async fn bind_mode(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
        webhooks_only: bool,
        provider_routes: Option<Arc<Vec<String>>>,
        canonical_discovery: Option<Arc<connect_protocol::canonical_discovery::TransportDiscovery>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(address.ip().is_loopback(), "HTTP listener must be loopback");
        let verifier = public_key
            .map(telnyx::webhooks::Verifier::new)
            .transpose()
            .map_err(|_| anyhow::anyhow!("Invalid TELNYX_PUBLIC_KEY"))?
            .map(Arc::new);
        let service_limit = service.store.limits().connections;
        let capacity = Arc::new(tokio::sync::Semaphore::new(service_limit));
        let stop = CancellationToken::new();
        let retirement: Arc<dyn connect_store::RuntimeRetirementSignal> =
            Arc::new(Retirement(stop.clone()));
        service.store.register_runtime_retirement(&retirement)?;
        let listener = tokio::net::TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let state = HttpState {
            canonical_discovery,
            provider_routes,
            service,
            verifier,
            capacity,
            cancellation: stop.clone(),
            address,
            public_authority: None,
        };
        let app = routes(state.clone(), None, webhooks_only);
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
        let capacity_for_connections = Arc::new(tokio::sync::Semaphore::new(service_limit));
        let task = tokio::spawn(async move {
            let guarded = super::http_record_io::GuardedListener {
                listener,
                cancellation: token.clone(),
                tls: None,
                handshakes: Arc::new(tokio::sync::Semaphore::new(1)),
                connections: capacity_for_connections,
            };
            axum::serve(
                guarded,
                app.into_make_service_with_connect_info::<super::http_record_io::Custody>(),
            )
            .with_graceful_shutdown(token.cancelled_owned())
            .await?;
            Ok(())
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

fn routes(
    state: HttpState,
    surface: Option<crate::hosting::HttpSurface>,
    webhooks_only: bool,
) -> Router<HttpState> {
    use crate::hosting::HttpSurface;
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
        .layer(DefaultBodyLimit::max(65_536))
        .merge(super::credential_issuance::router(state.clone()))
        .merge(super::enrollment::administration(state.clone()))
        .merge(super::user_delegation::router(state.clone()))
        .merge(access);
    let application = Router::new()
        .route("/v1/operations", post(operation))
        .merge(super::enrollment::application(state.clone()))
        .merge(super::canonical_http::router(state.clone()))
        .merge(super::record_mutations::router(state.clone()))
        .route(
            "/v1/conversations/{conversation_id}/artifacts/{artifact_id}",
            put(super::artifact_http::put_chunk).get(super::artifact_http::get_chunk),
        )
        .route(
            "/v1/conversations/{conversation_id}/artifacts/{artifact_id}/links/{token}",
            put(super::artifact_http::put_link_chunk).get(super::artifact_http::get_link_chunk),
        );
    let callbacks = Router::new().route("/webhooks/telnyx", post(super::telnyx_http::webhook));
    match surface {
        Some(HttpSurface::Application) => application,
        Some(HttpSurface::Administration) => administration,
        Some(HttpSurface::Callbacks) => callbacks,
        Some(HttpSurface::Health) => Router::new().route("/healthz", axum::routing::get(health)),
        None if webhooks_only => callbacks,
        None => application
            .merge(administration)
            .merge(callbacks)
            .layer(browser_cors()),
    }
}
async fn health(State(state): State<HttpState>) -> Response {
    let Ok(_permit) = state.capacity.try_acquire() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"profile":"connect/2","ready":false})),
        )
            .into_response();
    };
    let ready = !state.cancellation.is_cancelled()
        && state.service.readiness.snapshot().state == connect_protocol::ReadinessState::Ready;
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(serde_json::json!({"profile":"connect/2","ready":ready})),
    )
        .into_response()
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
