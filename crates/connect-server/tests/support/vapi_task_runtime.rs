//! Session creation and a controllably slow chat behind one private loopback API.
#![allow(clippy::unwrap_used, dead_code)]
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use connect_connectors::Vapi;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};
pub const ASSISTANT: &str = "12345678-1234-1234-1234-123456789012";
pub const SESSION: &str = "task-runtime-session";
#[derive(Clone)]
struct Api {
    session: Arc<Mutex<Value>>,
    chat_requests: Arc<Mutex<Vec<Value>>>,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
}
pub struct Fixture {
    pub provider: Arc<Vapi>,
    state: Api,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start() -> Self {
        let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = socket.local_addr().unwrap();
        let state = Api {
            session: Arc::new(Mutex::new(Value::Null)),
            chat_requests: Arc::new(Mutex::new(Vec::new())),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Semaphore::new(0)),
        };
        let app = Router::new()
            .route("/session", post(create_session))
            .route("/session/task-runtime-session", get(session))
            .route(
                "/assistant/12345678-1234-1234-1234-123456789012",
                get(assistant),
            )
            .route("/chat", post(chat))
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(socket, app).await.unwrap() });
        Self {
            provider: Arc::new(
                Vapi::new("task-runtime-private", &format!("http://{address}")).unwrap(),
            ),
            state,
            task,
        }
    }
    pub async fn posted_chat(&self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let entered = self.state.entered.notified();
                if !self.state.chat_requests.lock().unwrap().is_empty() {
                    break;
                }
                entered.await;
            }
        })
        .await
        .unwrap()
    }
    pub fn release_chat(&self) {
        self.state.release.add_permits(1);
    }
    pub fn requests(&self) -> Vec<Value> {
        self.state.chat_requests.lock().unwrap().clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
fn authenticate(headers: &HeaderMap) -> Result<(), StatusCode> {
    if headers.get("authorization").and_then(|h| h.to_str().ok())
        == Some("Bearer task-runtime-private")
    {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
async fn create_session(
    State(state): State<Api>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    authenticate(&headers)?;
    let value = json!({"id":SESSION,"assistantId":body["assistantId"],"status":"active","metadata":{"variableValues":body["assistantOverrides"]["variableValues"]}});
    *state.session.lock().unwrap() = value.clone();
    Ok(Json(value))
}
async fn session(State(state): State<Api>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    authenticate(&headers)?;
    let value = state.session.lock().unwrap().clone();
    if value.is_null() {
        Err(StatusCode::NOT_FOUND)
    } else {
        Ok(Json(value))
    }
}
async fn assistant(headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    authenticate(&headers)?;
    Ok(Json(
        json!({"id":ASSISTANT,"model":{"messages":[{"role":"system","content":"History: {{connect_context}}"}]}}),
    ))
}
async fn chat(
    State(state): State<Api>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    authenticate(&headers)?;
    state.chat_requests.lock().unwrap().push(body);
    state.entered.notify_one();
    state.release.acquire().await.unwrap().forget();
    Ok(Json(
        json!({"id":"slow-chat-reply","assistantId":ASSISTANT,"output":[{"role":"assistant","content":"Slow provider answer: use sapphire-nine."}]}),
    ))
}
