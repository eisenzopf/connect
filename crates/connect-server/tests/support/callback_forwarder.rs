//! Owned local HTTP proxy: preserves credentials and rewrites Host for the backend.
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use std::{net::SocketAddr, time::Duration};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub struct Forwarder {
    stop: CancellationToken,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

#[derive(Clone)]
struct Backend {
    client: reqwest_mcp::Client,
    url: String,
}

impl Forwarder {
    pub async fn start(listener: TcpListener, target: SocketAddr) -> Self {
        assert!(listener.local_addr().unwrap().ip().is_loopback());
        assert!(target.ip().is_loopback());
        let client = tokio::task::spawn_blocking(|| {
            reqwest_mcp::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap()
        })
        .await
        .unwrap();
        let app = Router::new()
            .route("/mcp", any(forward))
            .layer(axum::extract::DefaultBodyLimit::max(512 * 1024))
            .with_state(Backend {
                client,
                url: format!("http://{target}/mcp"),
            });
        let stop = CancellationToken::new();
        let signal = stop.clone();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(signal.cancelled_owned())
                .await
        });
        Self {
            stop,
            task: Some(task),
        }
    }

    pub async fn close(mut self) {
        self.stop.cancel();
        tokio::time::timeout(Duration::from_secs(5), self.task.as_mut().unwrap())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        self.task.take();
    }
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn forward(
    State(backend): State<Backend>,
    method: Method,
    mut headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Reqwest sets the backend Host. Do not replace Authorization, Origin or
    // provider correlation headers with fixture authority or remove them.
    for name in ["host", "content-length", "transfer-encoding", "connection"] {
        headers.remove(name);
    }
    let response = backend
        .client
        .request(method, &backend.url)
        .headers(headers)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let mut headers = response.headers().clone();
    for name in ["content-length", "transfer-encoding", "connection"] {
        headers.remove(name);
    }
    (status, headers, response.bytes().await.unwrap()).into_response()
}
