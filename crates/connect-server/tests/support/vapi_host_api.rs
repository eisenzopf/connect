//! Dynamic loopback private API retains the exact creation nonce for verification.
#![allow(clippy::unwrap_used)]
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, ResponseTemplate,
};

pub struct Api {
    pub server: MockServer,
    pub posts: Arc<Mutex<Vec<Value>>>,
}
impl Api {
    pub async fn start(status: u16) -> Self {
        Self::with_delay(status, std::time::Duration::ZERO).await
    }
    pub async fn with_delay(status: u16, delay: std::time::Duration) -> Self {
        let server = MockServer::start().await;
        let posts = Arc::new(Mutex::new(Vec::new()));
        let created = posts.clone();
        Mock::given(method("POST"))
            .and(path("/session"))
            .and(header("Authorization", "Bearer fake-private-key"))
            .respond_with(move |request: &Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                let value = resource(&body);
                created.lock().unwrap().push(body);
                ResponseTemplate::new(status)
                    .set_body_json(value)
                    .set_delay(delay)
            })
            .mount(&server)
            .await;
        let observed = posts.clone();
        Mock::given(method("GET"))
            .and(path("/session/installed-session"))
            .and(header("Authorization", "Bearer fake-private-key"))
            .respond_with(
                move |_request: &Request| match observed.lock().unwrap().last() {
                    Some(body) => ResponseTemplate::new(200).set_body_json(resource(body)),
                    None => ResponseTemplate::new(404),
                },
            )
            .mount(&server)
            .await;
        Self { server, posts }
    }
    pub fn body(&self) -> Value {
        self.posts.lock().unwrap().last().unwrap().clone()
    }
    pub fn count(&self) -> usize {
        self.posts.lock().unwrap().len()
    }
}
fn resource(body: &Value) -> Value {
    json!({"id":"installed-session","assistantId":body["assistantId"],"status":"active",
        "metadata":{"variableValues":body["assistantOverrides"]["variableValues"]}})
}
