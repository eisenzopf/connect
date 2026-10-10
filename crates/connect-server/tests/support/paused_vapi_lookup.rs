//! Explicit response barrier for authority changes during the private API read.
#![allow(clippy::unwrap_used, dead_code)]
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
    Json, Router,
};
use connect_connectors::Vapi;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{oneshot, Notify, Semaphore},
    task::JoinHandle,
};

#[derive(Clone)]
struct Gate {
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
}
pub struct PausedLookup {
    gate: Gate,
    stop: oneshot::Sender<()>,
    job: JoinHandle<std::io::Result<()>>,
}
impl PausedLookup {
    pub async fn start() -> (Self, Vapi) {
        let gate = Gate {
            entered: Arc::new(Notify::new()),
            release: Arc::new(Semaphore::new(0)),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/call/resource-one", get(lookup))
            .with_state(gate.clone());
        let (stop, stopped) = oneshot::channel();
        let job = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });
        let sdk = Vapi::new("private-test-key", &format!("http://{address}")).unwrap();
        (Self { gate, stop, job }, sdk)
    }
    pub async fn wait_for_lookup(&self) {
        tokio::time::timeout(Duration::from_secs(10), self.gate.entered.notified())
            .await
            .unwrap();
    }
    pub fn release(&self) {
        self.gate.release.add_permits(1);
    }
    pub async fn close(self) {
        self.stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(10), self.job)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
async fn lookup(State(gate): State<Gate>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Bearer private-test-key")
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    gate.entered.notify_one();
    let permit = gate
        .release
        .acquire()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    permit.forget();
    Ok(Json(
        json!({"id":"resource-one","assistantId":"assistant-one","status":"in-progress"}),
    ))
}
