//! Loopback HTTP/JSON facade, sharing the same service, scopes and receipts.
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use connect_protocol::{Error, Operation, PROFILE};
use connect_service::Service;
use std::time::Duration;
use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct HttpState {
    service: Arc<Service>,
    verifier: Option<Arc<telnyx::webhooks::Verifier>>,
    capacity: Arc<tokio::sync::Semaphore>,
}
pub struct Listener {
    pub address: SocketAddr,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}
impl Listener {
    pub async fn bind(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, false).await
    }
    /// A tunnel can expose this listener without exposing the application API.
    pub async fn bind_webhooks(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
    ) -> anyhow::Result<Self> {
        Self::bind_mode(address, service, public_key, true).await
    }
    async fn bind_mode(
        address: SocketAddr,
        service: Arc<Service>,
        public_key: Option<&str>,
        webhooks_only: bool,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(address.ip().is_loopback(), "HTTP listener must be loopback");
        let verifier = public_key
            .map(telnyx::webhooks::Verifier::new)
            .transpose()
            .map_err(|_| anyhow::anyhow!("Invalid TELNYX_PUBLIC_KEY"))?
            .map(Arc::new);
        let capacity = Arc::new(tokio::sync::Semaphore::new(
            service.store.limits().connections,
        ));
        let app = Router::new().route("/webhooks/telnyx", post(telnyx_webhook));
        let app = if webhooks_only {
            app
        } else {
            app.route("/v1/operations", post(operation))
        };
        let app = app
            .layer(DefaultBodyLimit::max(65_536))
            .with_state(HttpState {
                service,
                verifier,
                capacity,
            });
        let listener = tokio::net::TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let stop = CancellationToken::new();
        let token = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(token.cancelled_owned())
                .await
        });
        Ok(Self {
            address,
            stop,
            task: Some(task),
        })
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(6), &mut task).await {
                Ok(result) => {
                    result??;
                }
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    anyhow::bail!("HTTP shutdown timed out");
                }
            }
        }
        Ok(())
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
fn error(status: StatusCode, e: Error) -> Response {
    (
        status,
        Json(connect_protocol::Response {
            profile: PROFILE.into(),
            result: Err(e),
        }),
    )
        .into_response()
}
async fn operation(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(op): Json<Operation>,
) -> Response {
    let Ok(_permit) = state.capacity.clone().try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            Error::new("capacity-exceeded", "HTTP concurrency limit reached"),
        );
    };
    let Some(token) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
    else {
        return error(StatusCode::UNAUTHORIZED, Error::unauthorized());
    };
    let store = state.service.store.clone();
    let actor = match tokio::task::spawn_blocking(move || store.authenticate(&token)).await {
        Ok(Ok(actor)) => actor,
        _ => return error(StatusCode::UNAUTHORIZED, Error::unauthorized()),
    };
    let result = state.service.execute(actor.credential_id, op).await;
    let status = match &result {
        Err(e) if e.code == "unauthorized" => StatusCode::UNAUTHORIZED,
        Err(e) if e.code == "forbidden" => StatusCode::FORBIDDEN,
        _ => StatusCode::OK,
    };
    (
        status,
        Json(connect_protocol::Response {
            profile: PROFILE.into(),
            result,
        }),
    )
        .into_response()
}
async fn telnyx_webhook(
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
        Ok(v) => v,
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
        let required = |v: &serde_json::Value| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Error::new("invalid-request", "Missing callback field"))
        };
        match kind.as_str() {
            "message.received" => {
                if payload["direction"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case("outbound"))
                    || payload["autoresponse_type"]
                        .as_str()
                        .is_some_and(|s| !s.trim().is_empty())
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
                {
                    return Ok(false);
                }

                let from = required(&payload["from"]["phone_number"])?;
                let to = required(&payload["to"][0]["phone_number"])?;
                let body = required(&payload["text"])?;
                store.receive_sms(&format!("telnyx:{id}"), &digest, &from, &to, &body)
            }
            "message.finalized" => {
                let pid = required(&payload["id"])?;
                let status = required(&payload["to"][0]["status"])?;
                let final_state = match status.as_str() {
                    "delivered" => "delivered",
                    "delivery_failed" | "sending_failed" | "failed" => "failed",
                    _ => return Ok(false),
                };
                store.delivery_sms(&format!("telnyx:{id}"), &digest, &pid, final_state)
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
        Ok(Err(e)) => {
            let status = match e.code.as_str() {
                "forbidden" => StatusCode::FORBIDDEN,
                "invalid-request" | "conflict" => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            error(status, e)
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}
