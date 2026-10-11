#![allow(clippy::unwrap_used, dead_code)] // Shared integration fixtures use subsets per test.

pub(crate) mod webrtc;
pub(crate) mod wire_audio;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use rvoip_vapi::{VapiAdapter, VapiApiKey};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
struct FakeState {
    websocket_url: Arc<Mutex<String>>,
    observed: mpsc::UnboundedSender<Value>,
    burst: bool,
    call_gate: Option<Arc<tokio::sync::Semaphore>>,
    resource: Arc<Mutex<Value>>,
    create_status: axum::http::StatusCode,
}

pub(crate) struct FakeVapi {
    pub(crate) adapter: Arc<VapiAdapter>,
    observed: mpsc::UnboundedReceiver<Value>,
    task: tokio::task::JoinHandle<()>,
    call_gate: Option<Arc<tokio::sync::Semaphore>>,
    pub(crate) api_base: String,
}

impl FakeVapi {
    pub(crate) async fn start() -> Self {
        Self::start_profile(false, false, false).await
    }

    /// Saturate the real queue profile; retain the socket until cancelled.
    pub(crate) async fn start_burst() -> Self {
        Self::start_profile(true, false, false).await
    }

    /// Pause the provider create response to exercise revocation during attachment.
    pub(crate) async fn start_paused() -> Self {
        Self::start_profile(false, true, false).await
    }
    /// Accept one call remotely but lose its successful creation receipt.
    pub(crate) async fn start_uncertain() -> Self {
        Self::start_profile(false, false, true).await
    }
    pub(crate) fn release_call(&self) {
        self.call_gate.as_ref().unwrap().add_permits(1);
    }
    pub(crate) async fn assert_no_message(&mut self, kind: &str) {
        let received =
            tokio::time::timeout(std::time::Duration::from_millis(250), self.next(kind)).await;
        assert!(received.is_err(), "unexpected provider message: {kind}");
    }
    async fn start_profile(burst: bool, paused: bool, uncertain: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (observed, observed_rx) = mpsc::unbounded_channel();
        let call_gate = paused.then(|| Arc::new(tokio::sync::Semaphore::new(0)));
        let state = FakeState {
            websocket_url: Arc::new(Mutex::new(format!("ws://{address}/transport"))),
            observed,
            burst,
            call_gate: call_gate.clone(),
            resource: Arc::new(Mutex::new(Value::Null)),
            create_status: if uncertain {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            } else {
                axum::http::StatusCode::OK
            },
        };
        let router = Router::new()
            .route("/call", post(create_call))
            .route("/call/call-connect-gate-three", get(private_call))
            .route("/transport", get(upgrade_socket))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut config = connect_server::media_profile::vapi_config(
            VapiApiKey::new("fake-private-key").unwrap(),
        )
        .with_api_base(format!("http://{address}/").parse().unwrap())
        .with_loopback_test_transport();
        if !burst {
            config.heartbeat_interval = std::time::Duration::from_secs(60);
            config.graceful_shutdown_timeout = std::time::Duration::from_millis(100);
            config.startup_audio_frames = 1;
            config.jitter_target_frames = 1;
            config.jitter_target_floor_ms = 20;
            config.jitter_target_ceiling_ms = 20;
        }
        Self {
            adapter: VapiAdapter::new(config).unwrap(),
            observed: observed_rx,
            task,
            call_gate,
            api_base: format!("http://{address}"),
        }
    }

    pub(crate) async fn next(&mut self, kind: &str) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let value = self.observed.recv().await.unwrap();
                if value.get("type").and_then(Value::as_str) == Some(kind) {
                    break value;
                }
            }
        })
        .await
        .unwrap()
    }
}

impl Drop for FakeVapi {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn create_call(
    State(state): State<FakeState>,
    Json(body): Json<Value>,
) -> (axum::http::StatusCode, Json<Value>) {
    let resource = json!({
        "id": "call-connect-gate-three", "status":"queued",
        "assistantId":body["assistantId"], "assistantOverrides":body["assistantOverrides"],
        "transport":{"websocketCallUrl":state.websocket_url.lock().unwrap().clone()}
    });
    *state.resource.lock().unwrap() = resource.clone();
    let _ = state
        .observed
        .send(json!({"type":"create-call", "body":body}));
    if let Some(gate) = &state.call_gate {
        gate.acquire().await.unwrap().forget();
    }
    (state.create_status, Json(resource))
}
async fn private_call(
    State(state): State<FakeState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, axum::http::StatusCode> {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer fake-private-key")
    {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }
    Ok(Json(state.resource.lock().unwrap().clone()))
}

async fn upgrade_socket(State(state): State<FakeState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| fake_socket(socket, state))
}

async fn fake_socket(mut socket: WebSocket, state: FakeState) {
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) => {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    let _ = state.observed.send(value);
                }
            }
            Message::Binary(bytes) => {
                let _ = state.observed.send(json!({
                    "type": "audio",
                    "size": bytes.len(),
                    "first": bytes.first().copied()
                }));
                if state.burst {
                    // One coalesced message forces the jitter queue to drop
                    // frames before its paced drain can consume them.
                    socket
                        .send(Message::Binary(vec![0x2a; 160 * 1000].into()))
                        .await
                        .unwrap();
                    continue;
                }
                for _ in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    let _ = socket.send(Message::Binary(vec![0x2a; 160].into())).await;
                }
                let transcript = json!({
                    "type": "transcript",
                    "role": "assistant",
                    "transcriptType": "final",
                    "transcript": "The voice decision is sapphire-nine."
                })
                .to_string();
                let _ = socket.send(Message::Text(transcript.clone().into())).await;
                let _ = socket.send(Message::Text(transcript.into())).await;
                let outcome = json!({
                    "type": "end-of-call-report",
                    "endedReason": "assistant-ended-call",
                    "analysis": {
                        "summary": "The release decision was recorded.",
                        "successEvaluation": "true"
                    },
                    "privateProviderField": "must-not-enter-context"
                })
                .to_string();
                let _ = socket.send(Message::Text(outcome.clone().into())).await;
                let _ = socket.send(Message::Text(outcome.into())).await;
            }
            Message::Close(_) => {
                state.resource.lock().unwrap()["status"] = json!("ended");
                break;
            }
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }
}
