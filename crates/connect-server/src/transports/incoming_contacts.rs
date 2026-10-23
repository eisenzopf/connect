//! Private account-owner administration; middleware authenticates before JSON.
use super::{http::HttpState, provisioning::AdminCredential};
use axum::{
    extract::{rejection::JsonRejection, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use connect_protocol::{incoming_contacts::*, Error, Result};
pub(super) async fn command(
    State(state): State<HttpState>,
    Extension(AdminCredential(credential)): Extension<AdminCredential>,
    body: std::result::Result<Json<IncomingContactCommand>, JsonRejection>,
) -> Response {
    let command = match body {
        Ok(Json(command)) => command,
        Err(error) => {
            return response(Err(Error::new(
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "payload-too-large"
                } else {
                    "invalid-request"
                },
                "Invalid incoming contact command",
            )))
        }
    };
    response(
        state
            .service
            .execute_incoming_contact(credential, command)
            .await,
    )
}
fn response(result: Result<IncomingContactOutcome>) -> Response {
    let status = match &result {
        Ok(_) => StatusCode::OK,
        Err(error) => match error.code.as_str() {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            "conflict" | "provider-binding-conflict" => StatusCode::CONFLICT,
            "gone" => StatusCode::GONE,
            "payload-too-large" => StatusCode::PAYLOAD_TOO_LARGE,
            "operation-pending"
            | "provider-unavailable"
            | "capacity-exceeded"
            | "storage-unavailable"
            | "transport-uncertain"
            | "authority-busy"
            | "authority-unavailable" => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::BAD_REQUEST,
        },
    };
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(IncomingContactResponse {
            profile: INCOMING_CONTACT_PROFILE.into(),
            result,
        }),
    )
        .into_response()
}
