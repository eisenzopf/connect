//! Additional immutable enrollment ceilings at ordinary application HTTP intake.
use super::http::{bearer_token, HttpState};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use connect_protocol::{
    canonical_http::{Problem, Rejection},
    Error,
};
use std::time::Duration;

fn refused(error: Error) -> Response {
    let problem = Problem::from_error(&error);
    let status = StatusCode::from_u16(problem.code).unwrap_or(StatusCode::FORBIDDEN);
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(Rejection {
            profile: "connect/2".into(),
            problem,
        }),
    )
        .into_response()
}
pub(super) async fn admit(
    State(state): State<HttpState>,
    request: Request,
    next: Next,
) -> Response {
    // Claim has its own bearer-free, signed proof/Origin admission. Never turn
    // a bearer into authorization for that public endpoint.
    if !request.headers().contains_key(header::AUTHORIZATION)
        || (request.uri().path().starts_with("/v2/enrollments/")
            && request.uri().path().ends_with("/claim"))
    {
        return next.run(request).await;
    }
    let token = match bearer_token(request.headers()) {
        Ok(v) => v,
        // Each route still owns its authentication and body-decoding order.
        Err(_) => return next.run(request).await,
    };
    let mut origins = request.headers().get_all(header::ORIGIN).iter();
    let origin = match origins.next() {
        None => None,
        Some(v) => match v.to_str() {
            Ok(v) if origins.next().is_none() && v.len() <= 2048 => Some(v.to_owned()),
            _ => return refused(Error::forbidden()),
        },
    };
    let Ok(capacity) = state.capacity.clone().try_acquire_owned() else {
        return refused(Error::new(
            "capacity-exceeded",
            "Enrollment admission capacity exhausted",
        ));
    };
    let store = state.service.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _capacity = capacity;
        store.check_enrollment_http_bearer(&token, origin.as_deref())
    });
    let checked = tokio::select! {biased;
        _=state.cancellation.cancelled()=>return refused(Error::new("transport-uncertain","Admission interrupted")),
        result=tokio::time::timeout(Duration::from_secs(5),result)=>result,
    };
    match checked {
        Ok(Ok(Ok(()))) => next.run(request).await,
        Ok(Ok(Err(e))) if e.code == "unauthorized" => next.run(request).await,
        Ok(Ok(Err(e))) => refused(e),
        _ => refused(Error::storage()),
    }
}
