//! Explicit current human consent, authenticated before bounded body decoding.
use super::http::{bearer_token, HttpState};
use axum::{
    body::Body,
    extract::{rejection::JsonRejection, DefaultBodyLimit, Extension, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use connect_protocol::{user_delegation::*, Error};
struct Credential(String);
pub(super) fn router(state: HttpState) -> Router<HttpState> {
    Router::new()
        .route("/v2/user-delegations", post(command))
        .route_layer(middleware::from_fn_with_state(state, admit))
        .layer(DefaultBodyLimit::max(65_536))
}
fn response(result: connect_protocol::Result<UserDelegationOutcome>) -> Response {
    let result = result.map_err(|mut error| {
        if error.code == "request-conflict" {
            error.code = "conflict".into();
        }
        error
    });
    let status = match &result {
        Ok(_) => StatusCode::OK,
        Err(e) => match e.code.as_str() {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            "conflict" => StatusCode::CONFLICT,
            "capacity-exceeded" => StatusCode::TOO_MANY_REQUESTS,
            "request-too-large" => StatusCode::PAYLOAD_TOO_LARGE,
            "storage-unavailable"
            | "authority-busy"
            | "authority-unavailable"
            | "transport-uncertain" => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        },
    };
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(UserDelegationResponse {
            profile: "connect/2".into(),
            result,
        }),
    )
        .into_response()
}
async fn admit(State(state): State<HttpState>, mut request: Request<Body>, next: Next) -> Response {
    let token = match bearer_token(request.headers()) {
        Ok(t) => t,
        Err(e) => return response(Err(e)),
    };
    if request.headers().contains_key(header::ORIGIN)
        || request.headers().contains_key("x-call-id")
        || request.headers().contains_key("x-chat-id")
    {
        return response(Err(Error::forbidden()));
    }
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return response(Err(Error::new(
            "capacity-exceeded",
            "Delegation capacity exhausted",
        )));
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let store = state.service.store.clone();
    let authentication = tokio::task::spawn_blocking(move || {
        let actor = store.authenticate(&token)?;
        actor.require("credential.delegate")?;
        Ok::<_, Error>(actor.credential_id)
    });
    let unknown = || {
        response(Err(Error::new(
            "transport-uncertain",
            "Delegation outcome uncertain; preserve the original operation ID",
        )))
    };
    let credential = tokio::select! {biased;
        _=state.cancellation.cancelled()=>return unknown(),
        result=tokio::time::timeout_at(deadline,authentication)=>match result {Ok(Ok(Ok(id)))=>id,Ok(Ok(Err(e)))=>return response(Err(e)),Ok(Err(_))=>return response(Err(Error::storage())),Err(_)=>return unknown()},
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request
        .extensions_mut()
        .insert(std::sync::Arc::new(Credential(credential)));
    tokio::select! {biased;
        _=state.cancellation.cancelled()=>unknown(),
        result=tokio::time::timeout_at(deadline,next.run(request))=>result.unwrap_or_else(|_|unknown()),
    }
}
async fn command(
    State(state): State<HttpState>,
    Extension(credential): Extension<std::sync::Arc<Credential>>,
    body: Result<Json<UserDelegationRequest>, JsonRejection>,
) -> Response {
    let mut request = match body {
        Ok(Json(r)) => r,
        Err(e) => {
            return response(Err(Error::new(
                if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "request-too-large"
                } else {
                    "invalid-request"
                },
                "Invalid delegation request",
            )))
        }
    };
    if let Err(e) = request.normalize() {
        return response(Err(e));
    }
    let store = state.service.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let issued = store.issue_user_delegation(&credential.0, request.clone())?;
        Ok(UserDelegationOutcome {
            operation_id: request.operation_id,
            credential_id: issued.credential.id,
            identity_id: issued.identity_id,
            user_id: issued.authorization.user_id,
            operations: request.operations,
            recipients: request.recipients,
            expires_at: issued.expires_at,
            credential: issued.credential.token,
        })
    })
    .await
    .unwrap_or_else(|_| Err(Error::storage()));
    response(result)
}
