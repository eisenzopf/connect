//! Two independently addressable calls behind one account-free private API.
#![allow(clippy::unwrap_used, dead_code)]
use axum::{
    extract::{ws::Message, Path, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use rvoip_vapi::{VapiAdapter, VapiApiKey};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, Semaphore};

#[derive(Clone)]
struct Calls {
    base: String,
    resources: Arc<Mutex<BTreeMap<String, Value>>>,
    observed: mpsc::UnboundedSender<Value>,
    receipts: Arc<Semaphore>,
}
pub struct Fixture {
    pub adapter: Arc<VapiAdapter>,
    pub base: String,
    state: Calls,
    events: mpsc::UnboundedReceiver<Value>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start() -> Self {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let base = format!("http://{address}");
        let (observed, events) = mpsc::unbounded_channel();
        let state = Calls {
            base: format!("ws://{address}"),
            resources: Arc::new(Mutex::new(BTreeMap::new())),
            observed,
            receipts: Arc::new(Semaphore::new(0)),
        };
        let app = Router::new()
            .route("/assistant/{id}", get(assistant))
            .route("/call", post(create))
            .route("/call/{id}", get(lookup))
            .route("/transport/{id}", get(transport))
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        let mut config = connect_server::media_profile::vapi_config(
            VapiApiKey::new("two-call-private-key").unwrap(),
        )
        .with_api_base(format!("{base}/").parse().unwrap())
        .with_loopback_test_transport();
        config.heartbeat_interval = Duration::from_secs(60);
        config.graceful_shutdown_timeout = Duration::from_millis(100);
        config.startup_audio_frames = 1;
        config.jitter_target_frames = 1;
        config.jitter_target_floor_ms = 20;
        config.jitter_target_ceiling_ms = 20;
        Self {
            adapter: VapiAdapter::new(config).unwrap(),
            base,
            state,
            events,
            task,
        }
    }
    pub fn release_receipt(&self) {
        self.state.receipts.add_permits(1)
    }
    pub fn count(&self) -> usize {
        self.state.resources.lock().unwrap().len()
    }
    pub async fn next(&mut self, kind: &str) -> Value {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let event = self.events.recv().await.unwrap();
                if event["type"] == kind {
                    break event;
                }
            }
        })
        .await
        .unwrap()
    }
    pub async fn closed(&self, id: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.state.resources.lock().unwrap()[id]["status"] == "ended" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn authenticated(headers: &HeaderMap) -> Result<(), StatusCode> {
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        == Some("Bearer two-call-private-key")
    {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn assistant(Path(id): Path<String>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    authenticated(&headers)?;
    Ok(Json(
        json!({"id":id,"model":{"provider":"openai","model":"offline-fixture","tools":[]}}),
    ))
}
async fn create(
    State(state): State<Calls>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    authenticated(&headers)?;
    let resource = {
        let mut resources = state.resources.lock().unwrap();
        let id = format!("call-{}", resources.len() + 1);
        let value = json!({
            "id": id, "status": "queued", "assistantId": body["assistantId"],
            "assistantOverrides": body["assistantOverrides"],
            "transport": {"websocketCallUrl": format!("{}/transport/{id}", state.base)}
        });
        resources.insert(id, value.clone());
        value
    };
    let _ = state
        .observed
        .send(json!({"type":"created","resource":resource,"body":body}));
    state.receipts.acquire().await.unwrap().forget();
    Ok(Json(resource))
}
async fn lookup(
    State(state): State<Calls>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    authenticated(&headers)?;
    state
        .resources
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
async fn transport(
    State(state): State<Calls>,
    Path(id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    if !state.resources.lock().unwrap().contains_key(&id) {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(upgrade.on_upgrade(move |mut socket| async move {
        state.resources.lock().unwrap().get_mut(&id).unwrap()["status"] = json!("in-progress");
        while let Some(Ok(message)) = socket.next().await {
            match message {
                Message::Text(text) => {
                    if let Ok(body) = serde_json::from_str::<Value>(&text) {
                        let _ = state
                            .observed
                            .send(json!({"type":body["type"],"call_id":id,"body":body}));
                    }
                }
                Message::Binary(bytes) => {
                    let _ = state
                        .observed
                        .send(json!({"type":"audio","call_id":id,"size":bytes.len()}));
                    let _ = socket.send(Message::Binary(vec![0x2a; 160].into())).await;
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) => {}
            }
        }
        state.resources.lock().unwrap().get_mut(&id).unwrap()["status"] = json!("ended");
    }))
}
