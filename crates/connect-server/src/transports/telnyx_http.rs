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
    let digest = connect_store::provider_event_digest(&raw);
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
                if suppress_inbound_message(&payload) {
                    return Ok(false);
                }
                let from = required(&payload["from"]["phone_number"])?;
                let to = required(&payload["to"][0]["phone_number"])?;
                let body = required(&payload["text"])?;
                store.receive_sms(&format!("telnyx:{id}"), &digest, &from, &to, &body)
            }
            "message.finalized" => {
                let provider_id = required(&payload["id"])?;
                let status = required(&payload["to"][0]["status"])?;
                let final_state = match status.as_str() {
                    "delivered" => "delivered",
                    "delivery_failed" | "sending_failed" | "failed" => "failed",
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

fn suppress_inbound_message(payload: &serde_json::Value) -> bool {
    payload["direction"]
        .as_str()
        .is_some_and(|value| value.eq_ignore_ascii_case("outbound"))
        || payload["autoresponse_type"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
        || matches!(
            payload["text"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_ascii_uppercase()
                .as_str(),
            "STOP"
                | "STOPALL"
                | "STOP ALL"
                | "UNSUBSCRIBE"
                | "CANCEL"
                | "END"
                | "QUIT"
                | "START"
                | "UNSTOP"
                | "HELP"
        )
}
