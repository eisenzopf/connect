//! Enrollment carries Store ownership through the closing physical HTTP writer.
use super::{
    http::{bearer_token, HttpState},
    http_record_io::{Custody, ProtectedResponse},
};
use axum::{
    extract::{
        rejection::JsonRejection, ConnectInfo, DefaultBodyLimit, Extension, Path, Request, State,
    },
    http::{header, StatusCode, Version},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use connect_protocol::{enrollment::*, Error};
use connect_store::OwnedEnrollment;
use serde::Serialize;
use std::{sync::Arc, time::Duration};
use tokio::{sync::OwnedSemaphorePermit, time::Instant};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct Admission {
    issuer: Option<String>,
    origin: Option<String>,
    deadline: Instant,
    capacity: Arc<OwnedSemaphorePermit>,
    custody: Custody,
}
pub(super) fn administration(state: HttpState) -> Router<HttpState> {
    Router::new()
        .route("/v2/enrollments", post(approve))
        .route("/v2/enrollments/{id}/revoke", post(revoke))
        .route_layer(middleware::from_fn_with_state(state, admit_admin))
        .layer(DefaultBodyLimit::max(65_536))
}
pub(super) fn application(state: HttpState) -> Router<HttpState> {
    Router::new()
        .route("/v2/enrollments/{id}/claim", post(claim))
        .route_layer(middleware::from_fn_with_state(state, admit_claim))
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
        Json(EnrollmentResponse::<()> {
            profile: PROFILE.into(),
            result: Err(error),
        }),
    )
        .into_response()
}
fn interrupted() -> Response {
    response(Error::new(
        "transport-uncertain",
        "Enrollment interrupted; preserve the original intent, IDs and approved proof context",
    ))
}
async fn admit_admin(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    admit(state, request, next, true).await
}
async fn admit_claim(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    admit(state, request, next, false).await
}
async fn admit(state: HttpState, mut request: Request, next: Next, admin: bool) -> Response {
    let Ok(capacity) = state.capacity.clone().try_acquire_owned() else {
        return response(Error::new(
            "capacity-exceeded",
            "Enrollment capacity exhausted",
        ));
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let headers = request.headers();
    if !state.host_allowed(headers)
        || request.uri().query().is_some()
        || headers.contains_key("x-call-id")
        || headers.contains_key("x-chat-id")
        || (admin && headers.contains_key(header::ORIGIN))
        || (!admin && headers.contains_key(header::AUTHORIZATION))
    {
        return response(Error::forbidden());
    }
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let origin = match origins.next() {
        Some(value) => match value.to_str() {
            Ok(value) if origins.next().is_none() && value.len() <= 2048 => Some(value.to_owned()),
            _ => return response(Error::forbidden()),
        },
        None => None,
    };
    if request.version() != Version::HTTP_11 || headers.contains_key(header::UPGRADE) {
        return response(Error::new(
            "invalid-request",
            "Enrollment delivery requires closing HTTP/1.1",
        ));
    }
    let Some(ConnectInfo(custody)) = request.extensions().get::<ConnectInfo<Custody>>().cloned()
    else {
        return response(Error::forbidden());
    };
    let cancellation = custody.cancellation();
    let capacity = Arc::new(capacity);
    let issuer = if admin {
        let token = match bearer_token(headers) {
            Ok(v) => v,
            Err(e) => return response(e),
        };
        let store = state.service.store.clone();
        let held = capacity.clone();
        let auth = tokio::task::spawn_blocking(move || {
            let _held = held;
            let actor = store.authenticate(&token)?;
            actor.require("admin")?;
            Ok::<_, Error>(actor.credential_id)
        });
        Some(tokio::select! {biased;
            _=cancellation.cancelled()=>return interrupted(),
            result=tokio::time::timeout_at(deadline,auth)=>match result {
                Ok(Ok(Ok(id)))=>id,Ok(Ok(Err(e)))=>return response(e),Ok(Err(_))=>return response(Error::storage()),Err(_)=>return interrupted(),
            }
        })
    } else {
        None
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(Admission {
        issuer,
        origin,
        deadline,
        capacity,
        custody,
    });
    tokio::select! {biased;
        _=cancellation.cancelled()=>interrupted(),
        result=tokio::time::timeout_at(deadline,next.run(request))=>result.unwrap_or_else(|_|interrupted()),
    }
}
struct Delivery<T> {
    owned: OwnedEnrollment<T>,
    bytes: Vec<u8>,
    deadline: Instant,
    cancellation: CancellationToken,
}
impl<T: Send> ProtectedResponse for Delivery<T> {
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
        !self.cancellation.is_cancelled() && Instant::now() < self.deadline && self.owned.current()
    }
}
fn deliver<T: Send + Serialize + 'static>(
    admission: Admission,
    owned: OwnedEnrollment<T>,
) -> Response {
    let outcome = match owned.outcome() {
        Ok(v) => v,
        Err(e) => return response(e),
    };
    let bytes = match serde_json::to_vec(&EnrollmentResponse {
        profile: PROFILE.into(),
        result: Ok(outcome),
    }) {
        Ok(v) => v,
        Err(_) => return response(Error::storage()),
    };
    let remaining = chrono::DateTime::from_timestamp(owned.expires_at(), 0)
        .and_then(|until| (until - chrono::Utc::now()).to_std().ok())
        .unwrap_or_default();
    let delivery = Delivery {
        owned,
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
fn decode<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(v)| v).map_err(|e| {
        response(Error::new(
            if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
                "request-too-large"
            } else {
                "invalid-request"
            },
            "Invalid enrollment request",
        ))
    })
}
async fn approve(
    State(state): State<HttpState>,
    Extension(admission): Extension<Admission>,
    body: Result<Json<EnrollmentRequest>, JsonRejection>,
) -> Response {
    let request = match decode(body) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(issuer) = admission.issuer.clone() else {
        return response(Error::forbidden());
    };
    let store = state.service.store.clone();
    let capacity = admission.capacity.clone();
    match tokio::task::spawn_blocking(move || {
        let _held = capacity;
        store.approve_enrollment(&issuer, request)
    })
    .await
    {
        Ok(Ok(owned)) => deliver(admission, owned),
        Ok(Err(e)) => response(e),
        Err(_) => response(Error::storage()),
    }
}
async fn claim(
    State(state): State<HttpState>,
    Extension(admission): Extension<Admission>,
    Path(id): Path<String>,
    body: Result<Json<EnrollmentClaim>, JsonRejection>,
) -> Response {
    let request = match decode(body) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if request.enrollment_id != id {
        return response(Error::forbidden());
    }
    let store = state.service.store.clone();
    let origin = admission.origin.clone();
    let capacity = admission.capacity.clone();
    match tokio::task::spawn_blocking(move || {
        let _held = capacity;
        store.claim_enrollment(request, origin.as_deref())
    })
    .await
    {
        Ok(Ok(owned)) => deliver(admission, owned),
        Ok(Err(e)) => response(e),
        Err(_) => response(Error::storage()),
    }
}
async fn revoke(
    State(state): State<HttpState>,
    Extension(admission): Extension<Admission>,
    Path(id): Path<String>,
    body: Result<Json<EnrollmentRevokeRequest>, JsonRejection>,
) -> Response {
    let request = match decode(body) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(issuer) = admission.issuer.clone() else {
        return response(Error::forbidden());
    };
    let store = state.service.store.clone();
    let capacity = admission.capacity.clone();
    match tokio::task::spawn_blocking(move || {
        let _held = capacity;
        store.revoke_enrollment(&issuer, &id, &request.operation_id)
    })
    .await
    {
        Ok(Ok(owned)) => deliver(admission, owned),
        Ok(Err(e)) => response(e),
        Err(_) => response(Error::storage()),
    }
}
