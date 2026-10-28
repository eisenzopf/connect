//! Protected independent administrative issuance, with physical delivery custody.
use super::{
    http::{bearer_token, HttpState},
    http_record_io::{Custody, ProtectedResponse},
};
use axum::{
    extract::{rejection::JsonRejection, ConnectInfo, DefaultBodyLimit, Extension, Request, State},
    http::{header, StatusCode, Version},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use connect_protocol::{credential_issuance::*, Error};
use connect_store::IssuedScopedCredential;
use std::{sync::Arc, time::Duration};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Admission {
    issuer: String,
    deadline: Instant,
    capacity: Arc<OwnedSemaphorePermit>,
    custody: Custody,
}
pub(super) fn router(state: HttpState) -> Router<HttpState> {
    Router::new()
        .route("/v2/credentials", post(command))
        .route_layer(middleware::from_fn_with_state(state, admit))
        .layer(DefaultBodyLimit::max(65_536))
}
fn response(error: Error) -> Response {
    let status = match error.code.as_str() {
        "unauthorized" => StatusCode::UNAUTHORIZED,
        "forbidden" => StatusCode::FORBIDDEN,
        "conflict" => StatusCode::CONFLICT,
        "gone" => StatusCode::GONE,
        "request-too-large" => StatusCode::PAYLOAD_TOO_LARGE,
        "capacity-exceeded" => StatusCode::TOO_MANY_REQUESTS,
        "storage-unavailable"
        | "authority-busy"
        | "authority-unavailable"
        | "transport-uncertain"
        | "reconciliation-required" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
            (header::CONNECTION, "close"),
        ],
        Json(CredentialIssueResponse {
            profile: PROFILE.into(),
            result: Err(error),
        }),
    )
        .into_response()
}
fn interrupted() -> Response {
    response(Error::new(
        "transport-uncertain",
        "Issuance interrupted; preserve the complete original request and issuer",
    ))
}
async fn admit(State(state): State<HttpState>, mut request: Request, next: Next) -> Response {
    let Ok(capacity) = state.capacity.clone().try_acquire_owned() else {
        return response(Error::new(
            "capacity-exceeded",
            "Credential issuance capacity exhausted",
        ));
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let headers = request.headers();
    let host = state.host_allowed(headers);
    if !host
        || headers.contains_key(header::ORIGIN)
        || headers.contains_key("x-call-id")
        || headers.contains_key("x-chat-id")
    {
        return response(Error::forbidden());
    }
    if request.version() != Version::HTTP_11 || headers.contains_key(header::UPGRADE) {
        return response(Error::new(
            "invalid-request",
            "Credential delivery requires closing HTTP/1.1",
        ));
    }
    let Some(ConnectInfo(custody)) = request.extensions().get::<ConnectInfo<Custody>>().cloned()
    else {
        return response(Error::forbidden());
    };
    let token = match bearer_token(headers) {
        Ok(v) => v,
        Err(e) => return response(e),
    };
    let store = state.service.store.clone();
    let authenticate = tokio::task::spawn_blocking(move || {
        let actor = store.authenticate(&token)?;
        actor.require("admin")?;
        Ok::<_, Error>(actor.credential_id)
    });
    let cancellation = custody.cancellation();
    let issuer = tokio::select! {biased;
        _=cancellation.cancelled()=>return interrupted(),
        result=tokio::time::timeout_at(deadline,authenticate)=>match result {
            Ok(Ok(Ok(id)))=>id,Ok(Ok(Err(e)))=>return response(e),Ok(Err(_))=>return response(Error::storage()),Err(_)=>return interrupted(),
        }
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(Admission {
        issuer,
        deadline,
        capacity: Arc::new(capacity),
        custody,
    });
    tokio::select! {biased;
        _=cancellation.cancelled()=>interrupted(),
        result=tokio::time::timeout_at(deadline,next.run(request))=>result.unwrap_or_else(|_|interrupted()),
    }
}
struct Delivery {
    issued: IssuedScopedCredential,
    bytes: Vec<u8>,
    deadline: Instant,
    cancellation: CancellationToken,
}
impl ProtectedResponse for Delivery {
    fn status(&self) -> u16 {
        200
    }
    fn body(&self) -> connect_protocol::Result<&[u8]> {
        if !self.current() {
            return Err(Error::unauthorized());
        }
        Ok(&self.bytes)
    }
    fn deadline(&self) -> Instant {
        self.deadline
    }
    fn current(&self) -> bool {
        !self.cancellation.is_cancelled() && Instant::now() < self.deadline && self.issued.current()
    }
}
async fn command(
    State(state): State<HttpState>,
    Extension(admission): Extension<Admission>,
    body: Result<Json<CredentialIssueRequest>, JsonRejection>,
) -> Response {
    let request = match body {
        Ok(Json(v)) => v,
        Err(e) => {
            return response(Error::new(
                if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "request-too-large"
                } else {
                    "invalid-request"
                },
                "Invalid credential issuance request",
            ))
        }
    };
    let store = state.service.store.clone();
    let issuer = admission.issuer;
    let issued =
        match tokio::task::spawn_blocking(move || store.issue_scoped_credential(&issuer, request))
            .await
        {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return response(e),
            Err(_) => return response(Error::storage()),
        };
    let outcome = match issued.outcome() {
        Ok(v) => v.clone(),
        Err(e) => return response(e),
    };
    let bytes = match serde_json::to_vec(&CredentialIssueResponse {
        profile: PROFILE.into(),
        result: Ok(outcome),
    }) {
        Ok(v) => v,
        Err(_) => return response(Error::storage()),
    };
    let remaining = chrono::DateTime::from_timestamp(issued.expires_at(), 0)
        .and_then(|until| (until - chrono::Utc::now()).to_std().ok())
        .unwrap_or_default();
    let delivery = Delivery {
        issued,
        bytes,
        deadline: admission.deadline.min(Instant::now() + remaining),
        cancellation: admission.custody.cancellation(),
    };
    let (_, bytes) = match admission.custody.install(delivery, admission.capacity) {
        Ok(v) => v,
        Err(e) => return response(e),
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CONNECTION, "close"),
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        bytes,
    )
        .into_response()
}
