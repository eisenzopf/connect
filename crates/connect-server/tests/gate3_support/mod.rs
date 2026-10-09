#![allow(clippy::unwrap_used, dead_code)] // Shared integration fixtures use subsets per test.

pub(crate) mod webrtc;

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
use rvoip_vapi::{VapiAdapter, VapiApiKey, VapiConfig};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

#[derive(Clone)]
struct FakeState {
    websocket_url: Arc<Mutex<String>>,
    observed: mpsc::UnboundedSender<Value>,
}

pub(crate) struct FakeVapi {
    pub(crate) adapter: Arc<VapiAdapter>,
    observed: mpsc::UnboundedReceiver<Value>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeVapi {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (observed, observed_rx) = mpsc::unbounded_channel();
        let state = FakeState {
            websocket_url: Arc::new(Mutex::new(format!("ws://{address}/transport"))),
            observed,
        };
        let router = Router::new()
            .route("/call", post(create_call))
            .route("/transport", get(upgrade_socket))
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut config = VapiConfig::new(VapiApiKey::new("fake-private-key").unwrap())
            .with_api_base(format!("http://{address}/").parse().unwrap())
            .with_loopback_test_transport();
        config.heartbeat_interval = std::time::Duration::from_secs(60);
        config.graceful_shutdown_timeout = std::time::Duration::from_millis(100);
        config.startup_audio_frames = 1;
        config.jitter_target_frames = 1;
        config.jitter_target_floor_ms = 20;
        config.jitter_target_ceiling_ms = 20;
        Self {
            adapter: VapiAdapter::new(config).unwrap(),
            observed: observed_rx,
            task,
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

async fn create_call(State(state): State<FakeState>) -> Json<Value> {
    Json(json!({
        "id": "call-connect-gate-three",
        "status": "queued",
        "transport": {
            "websocketCallUrl": state.websocket_url.lock().unwrap().clone()
        }
    }))
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
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }
}
