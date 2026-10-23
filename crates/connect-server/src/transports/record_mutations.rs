//! Named local erasure API. Authentication precedes bounded body decoding;
//! effects and receipt replay derive current source authority in the Store.
use super::http::{bearer_token, HttpState};
use axum::{
    body::Body,
    extract::{rejection::JsonRejection, DefaultBodyLimit, Extension, Path, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use connect_protocol::{record_mutations, Error};
struct Credential(String);
pub(super) fn router(state: HttpState) -> Router<HttpState> {
    Router::new()
        .route("/v2/records/{id}/mutations", post(command))
        .route_layer(middleware::from_fn_with_state(state, admit))
        .layer(DefaultBodyLimit::max(65_536))
}
fn response(result: connect_protocol::Result<record_mutations::Outcome>) -> Response {
    let code = match &result {
        Ok(_) => StatusCode::OK,
        Err(e) => match e.code.as_str() {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            "not-found" => StatusCode::NOT_FOUND,
            "conflict" => StatusCode::CONFLICT,
            "unsupported-command" => StatusCode::NOT_IMPLEMENTED,
            "capacity-exceeded" => StatusCode::TOO_MANY_REQUESTS,
            "payload-too-large" => StatusCode::PAYLOAD_TOO_LARGE,
            "storage-unavailable"
            | "authority-busy"
            | "authority-unavailable"
            | "transport-uncertain" => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        },
    };
    (
        code,
        [(header::CACHE_CONTROL, "no-store")],
        Json(record_mutations::Response {
            profile: "connect/2".into(),
            result,
        }),
    )
        .into_response()
}
async fn admit(State(state): State<HttpState>, mut request: Request<Body>, next: Next) -> Response {
    let token = match bearer_token(request.headers()) {
        Ok(token) => token,
        Err(error) => return response(Err(error)),
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let store = state.service.store.clone();
    let validating = tokio::task::spawn_blocking(move || store.authenticate(&token));
    let checked = tokio::select! {biased;
        _=state.cancellation.cancelled()=>return response(Err(Error::new("transport-uncertain","Record mutation acceptance unknown; reuse the original operation ID"))),
        result=tokio::time::timeout_at(deadline,validating)=>match result {Ok(result)=>result,Err(_)=>return response(Err(Error::new("transport-uncertain","Record mutation acceptance unknown; reuse the original operation ID")))},
    };
    let credential = match checked {
        Ok(Ok(actor)) => actor.credential_id,
        Ok(Err(error)) if error.code == "storage-unavailable" => return response(Err(error)),
        Ok(Err(_)) => return response(Err(Error::unauthorized())),
        Err(_) => return response(Err(Error::storage())),
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
            "Record mutation capacity reached",
        )));
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request
        .extensions_mut()
        .insert(std::sync::Arc::new(Credential(credential)));
    tokio::select! {biased;
        _=state.cancellation.cancelled()=>response(Err(Error::new("transport-uncertain","Record mutation acceptance unknown; reuse the original operation ID"))),
        result=tokio::time::timeout_at(deadline,next.run(request))=>result.unwrap_or_else(|_|response(Err(Error::new("transport-uncertain","Record mutation acceptance unknown; reuse the original operation ID")))),
    }
}
async fn command(
    State(state): State<HttpState>,
    Path(id): Path<String>,
    Extension(credential): Extension<std::sync::Arc<Credential>>,
    body: Result<Json<record_mutations::Request>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(request)) => request,
        Err(error) if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return response(Err(Error::new(
                "payload-too-large",
                "Record mutation input exceeds64KiB",
            )))
        }
        Err(_) => {
            return response(Err(Error::new(
                "invalid-request",
                "Invalid record mutation input",
            )))
        }
    };
    if id != request.uuid {
        return response(Err(Error::new(
            "invalid-request",
            "Record route and selector differ",
        )));
    }
    response(
        state
            .service
            .mutate_record(credential.0.clone(), request)
            .await,
    )
}
