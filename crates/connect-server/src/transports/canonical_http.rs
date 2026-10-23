//! Current HTTP canonical commands. No stock Core or historical profile fallback.
use super::http::{bearer_token, HttpState};
use axum::{
    body::{Body, Bytes},
    extract::{rejection::BytesRejection, ConnectInfo, DefaultBodyLimit, Extension, State},
    http::{HeaderMap, Request, StatusCode, Version},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use connect_protocol::{canonical_http::*, Error};
use connect_service::{StoreCredentialBinding, StoreValidator};
use rvoip_auth_core::BearerValidator;
use rvoip_uctp::v2::Envelope;

pub(super) fn router(state: HttpState) -> Router<HttpState> {
    Router::new()
        .route("/v2/commands", post(command))
        .route_layer(middleware::from_fn_with_state(state, admit))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
}
fn rejection(error: Error) -> Response {
    let problem = Problem::from_error(&error);
    let status = StatusCode::from_u16(problem.code).unwrap_or(StatusCode::BAD_REQUEST);
    let mut response = (
        status,
        Json(Rejection {
            profile: PROFILE.into(),
            problem,
        }),
    )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
async fn admit(State(state): State<HttpState>, mut request: Request<Body>, next: Next) -> Response {
    let token = match bearer_token(request.headers()) {
        Ok(token) => token,
        Err(error) => return rejection(error),
    };
    let credential = match StoreValidator::canonical(state.service.store.clone())
        .validate_credential(&token)
        .await
    {
        Ok(credential) => credential,
        Err(rvoip_auth_core::bearer::BearerAuthError::Unavailable(_)) => {
            return rejection(Error::storage())
        }
        Err(_) => return rejection(Error::unauthorized()),
    };
    let store = state.service.store.clone();
    let binding = match tokio::task::spawn_blocking(move || {
        StoreCredentialBinding::from_validated(&store, &credential)
    })
    .await
    {
        Ok(Ok(binding)) => binding,
        Ok(Err(error)) if error.code == "storage-unavailable" => return rejection(error),
        Ok(Err(_)) => return rejection(Error::unauthorized()),
        Err(_) => return rejection(Error::storage()),
    };
    // Bound body collection as well as command execution. Authentication still
    // precedes application parsing and its capacity refusal.
    let Ok(permit) = state.capacity.clone().try_acquire_owned() else {
        return rejection(Error::new(
            "capacity-exceeded",
            "Canonical HTTP concurrency exhausted",
        ));
    };
    request.extensions_mut().insert(std::sync::Arc::new(permit));
    request.extensions_mut().insert(binding);
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
async fn command(
    State(state): State<HttpState>,
    Extension(binding): Extension<StoreCredentialBinding>,
    Extension(capacity): Extension<std::sync::Arc<tokio::sync::OwnedSemaphorePermit>>,
    connection: Option<Extension<ConnectInfo<super::http_record_io::Custody>>>,
    version: Version,
    headers: HeaderMap,
    input: Result<Bytes, BytesRejection>,
) -> Response {
    let json = headers
        .get_all(axum::http::header::CONTENT_TYPE)
        .iter()
        .count()
        == 1
        && headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.split(';').next())
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"));
    if !json {
        return rejection(Error::new(
            "invalid-request",
            "Canonical HTTP requires JSON",
        ));
    }
    let input = match input {
        Ok(bytes) => bytes,
        Err(error) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return rejection(Error::new(
                "request-too-large",
                "Canonical HTTP body exceeds its bound",
            ))
        }
        Err(_) => return rejection(Error::new("invalid-request", "Invalid canonical HTTP body")),
    };
    let request = match Envelope::parse(&input) {
        Ok(request) => request,
        Err(_) => return rejection(Error::new("invalid-request", "Invalid canonical envelope")),
    };
    if matches!(
        request.payload(),
        rvoip_uctp::v2::Payload::RecordingVconFetch(_)
    ) {
        // HTTP/2 multiplexing cannot lend a connection-wide response guard.
        // Only this explicit closing HTTP/1.1 physical-writer profile is admitted.
        if version != Version::HTTP_11 || headers.contains_key(axum::http::header::UPGRADE) {
            return rejection(Error::new(
                "unsupported-command",
                "Record delivery requires closing HTTP/1.1",
            ));
        }
        let Some(Extension(ConnectInfo(connection))) = connection else {
            return rejection(Error::forbidden());
        };
        let response = match state
            .service
            .execute_canonical_http_record(binding, request, connection.cancellation())
            .await
        {
            Ok(response) => response,
            Err(error) => return rejection(error),
        };
        let (status, bytes) = match connection.install(response, capacity) {
            Ok(value) => value,
            Err(error) => return rejection(error),
        };
        return (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            [
                (axum::http::header::CONTENT_TYPE, "application/json"),
                (axum::http::header::CONNECTION, "close"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response();
    }
    match state.service.execute_canonical_http(binding, request).await {
        Ok((status, response)) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            Json(response),
        )
            .into_response(),
        Err(error) => rejection(error),
    }
}
