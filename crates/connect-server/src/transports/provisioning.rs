//! Trusted backend administration. Authentication and bounded admission precede
//! body decoding; every mutation rechecks current authority in its transaction.
use super::http::{authenticated_credential, HttpState};
use axum::{
    extract::{rejection::JsonRejection, Path, Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use connect_protocol::{provisioning::*, Error};

#[derive(Clone)]
pub(super) struct AdminCredential(String);
pub(super) async fn admit(
    State(state): State<HttpState>,
    mut request: Request,
    next: Next,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return response(Err(Error::new(
            "capacity-exceeded",
            "HTTP concurrency limit reached",
        )));
    };
    if request.headers().contains_key(header::ORIGIN)
        || request.headers().contains_key("x-call-id")
        || request.headers().contains_key("x-chat-id")
    {
        return response(Err(Error::forbidden()));
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let headers = request.headers().clone();
    let authenticate = async {
        let credential = authenticated_credential(&state, &headers)
            .await
            .map_err(|_| Error::unauthorized())?;
        let store = state.service.store.clone();
        let checked = credential.clone();
        tokio::task::spawn_blocking(move || store.revalidate(&checked)?.require("admin"))
            .await
            .map_err(|_| Error::storage())??;
        Ok::<_, Error>(credential)
    };
    let credential = tokio::select! {
        _ = state.cancellation.cancelled() => return interrupted(),
        result = tokio::time::timeout_at(deadline, authenticate) => match result {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => return response(Err(error)),
            Err(_) => return interrupted(),
        },
    };
    request.headers_mut().remove(header::AUTHORIZATION);
    request.extensions_mut().insert(AdminCredential(credential));
    let mut result = tokio::select! {
        _ = state.cancellation.cancelled() => interrupted(),
        result = tokio::time::timeout_at(deadline, next.run(request)) => result.unwrap_or_else(|_| interrupted()),
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
        "Provisioning interrupted; retry the same logical operation ID",
    )))
}
fn decode(
    body: Result<Json<ProvisioningCommand>, JsonRejection>,
) -> Result<ProvisioningCommand, Response> {
    body.map(|Json(value)| value).map_err(|error| {
        if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(ProvisioningResponse {
                    profile: PROVISIONING_PROFILE.into(),
                    result: Err(Error::new(
                        "payload-too-large",
                        "Provisioning body exceeds 64 KiB",
                    )),
                }),
            )
                .into_response()
        } else {
            response(Err(Error::new(
                "invalid-request",
                "Invalid provisioning body",
            )))
        }
    })
}
pub(super) async fn identity(
    State(state): State<HttpState>,
    Extension(AdminCredential(credential)): Extension<AdminCredential>,
    body: Result<Json<ProvisioningCommand>, JsonRejection>,
) -> Response {
    let command = match decode(body) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !matches!(command, ProvisioningCommand::Identity { .. }) {
        return response(Err(Error::new(
            "invalid-request",
            "Expected identity provisioning",
        )));
    }
    response(
        state
            .service
            .execute_provisioning(credential, command)
            .await,
    )
}
pub(super) async fn scoped(
    State(state): State<HttpState>,
    Path(id): Path<String>,
    Extension(AdminCredential(credential)): Extension<AdminCredential>,
    uri: axum::http::Uri,
    body: Result<Json<ProvisioningCommand>, JsonRejection>,
) -> Response {
    let command = match decode(body) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let path = uri.path();
    let matches = match &command {
        ProvisioningCommand::Participant {
            conversation_id, ..
        } => {
            conversation_id == &id
                && path.starts_with("/v2/conversations/")
                && path.ends_with("/participants")
        }
        ProvisioningCommand::Endpoint { identity_id, .. } => {
            identity_id == &id
                && path.starts_with("/v2/identities/")
                && path.ends_with("/endpoints")
        }
        ProvisioningCommand::VerifyEndpoint { endpoint_id, .. } => {
            endpoint_id == &id && path.starts_with("/v2/endpoints/") && path.ends_with("/verify")
        }
        ProvisioningCommand::RevokeEndpoint { endpoint_id, .. } => {
            endpoint_id == &id && path.starts_with("/v2/endpoints/") && path.ends_with("/revoke")
        }
        ProvisioningCommand::RevokeIdentity { identity_id, .. } => {
            identity_id == &id && path.starts_with("/v2/identities/") && path.ends_with("/revoke")
        }
        _ => false,
    };
    if !matches {
        return response(Err(Error::new(
            "invalid-request",
            "Provisioning target does not match route",
        )));
    }
    response(
        state
            .service
            .execute_provisioning(credential, command)
            .await,
    )
}
fn response(result: connect_protocol::Result<ProvisioningOutcome>) -> Response {
    let status = match &result {
        Ok(_) => StatusCode::OK,
        Err(e) => match e.code.as_str() {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            "conflict" => StatusCode::CONFLICT,
            "gone" => StatusCode::GONE,
            "capacity-exceeded" | "storage-unavailable" | "transport-uncertain" => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            _ => StatusCode::BAD_REQUEST,
        },
    };
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(ProvisioningResponse {
            profile: PROVISIONING_PROFILE.into(),
            result,
        }),
    )
        .into_response()
}
