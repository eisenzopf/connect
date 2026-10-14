//! Verified Telnyx callback admission and durable SMS projection.
use super::http::{error, HttpState};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use connect_protocol::Error;

pub(super) async fn webhook(
    State(state): State<HttpState>,
    headers: HeaderMap,
    raw: Bytes,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Some(verifier) = state.verifier else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if verifier.verify_headers(&headers, &raw).is_err() {
        return error(StatusCode::UNAUTHORIZED, Error::unauthorized());
    }
    let value: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let data = &value["data"];
    let Some(id) = data["id"].as_str().map(str::to_owned) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let kind = data["event_type"].as_str().unwrap_or_default().to_owned();
    let payload = data["payload"].clone();
    let occurred_at = data["occurred_at"]
        .as_str()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.timestamp());
    // Signature covers the original bytes, but delivery-attempt metadata is not
    // event identity. Telnyx may change meta.attempt when retrying the same data.
    let event_bytes = match serde_json::to_vec(data) {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let digest = connect_store::provider_event_digest(&event_bytes);
    let store = state.service.store.clone();
    let result = tokio::task::spawn_blocking(move || {
        let required = |field: &serde_json::Value| {
            field
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Error::new("invalid-request", "Missing callback field"))
        };
        match kind.as_str() {
            "message.received" => {
                if payload["direction"]
                    .as_str()
                    .is_some_and(|v| v.eq_ignore_ascii_case("outbound"))
                {
                    return Ok(false);
                }
                let from = required(&payload["from"]["phone_number"])?;
                let to = required(&payload["to"][0]["phone_number"])?;
                let body = required(&payload["text"])?;
                let automatic = payload["autoresponse_type"].as_str().unwrap_or_default();
                // Signed provider classification also covers custom stop/help phrases.
                let control = if connect_store::sms_control_kind(automatic) == Some(Some(true)) {
                    automatic
                } else if connect_store::sms_control_kind(&body).is_some() {
                    &body
                } else {
                    automatic
                };
                if connect_store::sms_control_kind(control).is_some() {
                    let at = occurred_at.ok_or_else(|| {
                        Error::new(
                            "invalid-request",
                            "SMS controls require provider occurred_at",
                        )
                    })?;
                    return store.receive_sms_control(
                        &format!("telnyx:{id}"),
                        &digest,
                        &from,
                        &to,
                        control,
                        at,
                    );
                }
                if payload["autoresponse_type"]
                    .as_str()
                    .is_some_and(|v| !v.trim().is_empty())
                {
                    return Ok(false);
                }
                store.receive_sms(&format!("telnyx:{id}"), &digest, &from, &to, &body)
            }
            "message.finalized" => {
                let provider_id = required(&payload["id"])?;
                let status = required(&payload["to"][0]["status"])?;
                let final_state =
                    match connect_connectors::provider_mapping::telnyx_message_state(&status) {
                        Ok(connect_connector_api::lifecycle::MessageState::Delivered) => {
                            "delivered"
                        }
                        Ok(connect_connector_api::lifecycle::MessageState::Failed) => "failed",
                        _ => return Ok(false),
                    };
                store.delivery_sms(&format!("telnyx:{id}"), &digest, &provider_id, final_state)
            }
            _ => Ok(false),
        }
    })
    .await;
    match result {
        Ok(Ok(_)) => {
            state.service.notify_changed();
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(Err(value)) => {
            let status = match value.code.as_str() {
                "forbidden" => StatusCode::FORBIDDEN,
                "invalid-request" | "conflict" => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            error(status, value)
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
