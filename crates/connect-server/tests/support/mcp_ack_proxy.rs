//! Fixture proxy that withholds one actual accepted tool response.
#![allow(clippy::unwrap_used, dead_code)]
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct Proxy {
    pub address: std::net::SocketAddr,
    pub accepted: Arc<tokio::sync::Notify>,
    pub result: Arc<Mutex<Option<serde_json::Value>>>,
    stop: CancellationToken,
    handle: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}
#[derive(Clone)]
struct Forward {
    client: reqwest_mcp::Client,
    target: String,
    accepted: Arc<tokio::sync::Notify>,
    result: Arc<Mutex<Option<serde_json::Value>>>,
    stop: CancellationToken,
}
impl Proxy {
    pub async fn start(target: std::net::SocketAddr) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::task::spawn_blocking(|| {
            reqwest_mcp::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap()
        })
        .await
        .unwrap();
        let accepted = Arc::new(tokio::sync::Notify::new());
        let result = Arc::new(Mutex::new(None));
        let stop = CancellationToken::new();
        let state = Forward {
            client,
            target: format!("http://{target}/mcp"),
            accepted: accepted.clone(),
            result: result.clone(),
            stop: stop.clone(),
        };
        let app = Router::new()
            .route("/mcp", post(forward))
            .layer(axum::extract::DefaultBodyLimit::max(512 * 1024))
            .with_state(state);
        let signal = stop.clone();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(signal.cancelled_owned())
                .await
        });
        Self {
            address,
            accepted,
            result,
            stop,
            handle: Some(handle),
        }
    }
    pub async fn close(mut self) {
        self.stop.cancel();
        let mut handle = self.handle.take().unwrap();
        match tokio::time::timeout(Duration::from_secs(5), &mut handle).await {
            Ok(result) => result.unwrap().unwrap(),
            Err(_) => {
                handle.abort();
                let _ = handle.await;
                panic!("owned ACK proxy did not drain");
            }
        }
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}
async fn forward(State(state): State<Forward>, mut headers: HeaderMap, body: Bytes) -> Response {
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let hold = request["method"] == "tools/call" && request["params"]["name"] == "connect_send";
    // Forward the caller's original bearer, not fixture owner authority.
    for name in ["host", "content-length", "transfer-encoding", "connection"] {
        headers.remove(name);
    }
    let response = state
        .client
        .post(&state.target)
        .headers(headers)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    if hold {
        assert_eq!(status, StatusCode::OK);
        let rpc: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        *state.result.lock().unwrap() = Some(rpc["result"].clone());
        state.accepted.notify_one();
        state.stop.cancelled().await;
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    (status, [("content-type", "application/json")], bytes).into_response()
}
