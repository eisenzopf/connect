//! Loopback developer exchange; model grants cannot mint more authority.
use super::http::{authenticated_credential, HttpState};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use connect_protocol::{
    tasks::{TaskAccessRequest, TaskAccessResponse, TASK_ACCESS_PROFILE},
    Error,
};

#[derive(Clone)]
pub(super) struct DeveloperCredential(String);
pub(super) async fn admit(
    State(state): State<HttpState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Ok(_permit) = state.capacity.try_acquire() else {
        return response(Err(Error::new(
            "capacity-exceeded",
            "HTTP concurrency limit reached",
        )));
    };
    let headers = request.headers();
    if !state.host_allowed(headers) {
        return response(Err(Error::forbidden()));
    }
    if headers.contains_key(header::ORIGIN)
        || headers.contains_key("x-call-id")
        || headers.contains_key("x-chat-id")
    {
        return response(Err(Error::forbidden()));
    }
    let credential = tokio::select! {
        _ = state.cancellation.cancelled() => return interrupted(),
        result = tokio::time::timeout(std::time::Duration::from_secs(5), authenticated_credential(&state, headers)) => match result {
            Ok(Ok(value)) => value,
            _ => return response(Err(Error::unauthorized())),
        },
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request
        .extensions_mut()
        .insert(DeveloperCredential(credential));
    let mut result = tokio::select! {
        _ = state.cancellation.cancelled() => interrupted(),
        result = tokio::time::timeout(std::time::Duration::from_secs(5), next.run(request)) => result.unwrap_or_else(|_| interrupted()),
    };
    result.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    result
}
fn interrupted() -> Response {
    response(Err(Error::new(
        "transport-uncertain",
        "Task access interrupted; reconcile the original issuance request",
    )))
}
pub(super) async fn exchange(
    State(state): State<HttpState>,
    Extension(DeveloperCredential(credential)): Extension<DeveloperCredential>,
    Json(request): Json<TaskAccessRequest>,
) -> Response {
    response(state.service.execute_task_access(credential, request).await)
}
fn response(
    result: connect_protocol::Result<connect_protocol::tasks::TaskAccessOutcome>,
) -> Response {
    let status = match &result {
        Err(error) if error.code == "unauthorized" => StatusCode::UNAUTHORIZED,
        Err(error) if error.code == "forbidden" => StatusCode::FORBIDDEN,
        Err(error) if error.code == "storage-unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        Err(error) if error.code == "capacity-exceeded" => StatusCode::SERVICE_UNAVAILABLE,
        Err(error) if error.code == "transport-uncertain" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::OK,
    };
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(TaskAccessResponse {
            profile: TASK_ACCESS_PROFILE.into(),
            result,
        }),
    )
        .into_response()
}
