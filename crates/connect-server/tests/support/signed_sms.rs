//! Real loopback callback transport signed by a fixture-only Telnyx key.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_server::runtime::RunningServer;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use std::time::Duration;

pub struct Callbacks {
    address: std::net::SocketAddr,
    key: SigningKey,
    http: reqwest::Client,
}
impl Callbacks {
    pub async fn start(server: &RunningServer) -> Self {
        let key = SigningKey::from_bytes(&[29; 32]);
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            address: server.http_address.unwrap(),
            key,
            http,
        }
    }
    pub async fn post(&self, data: Value, tampered: bool) -> reqwest::StatusCode {
        let body = serde_json::to_vec(&json!({"data":data})).unwrap();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        let signature = self.key.sign(&[timestamp.as_bytes(), b"|", &body].concat());
        self.http
            .post(format!("http://{}/webhooks/telnyx", self.address))
            .header("telnyx-timestamp", timestamp)
            .header(
                "telnyx-signature-ed25519",
                STANDARD.encode(signature.to_bytes()),
            )
            .body(if tampered { b"{}".to_vec() } else { body })
            .send()
            .await
            .unwrap()
            .status()
    }
    pub async fn accepted(&self, data: Value) {
        assert_eq!(
            self.post(data, false).await,
            reqwest::StatusCode::NO_CONTENT
        );
    }
}

pub fn incoming(id: &str, text: &str, at: i64, from: &str, to: &str) -> Value {
    json!({
        "id":id, "event_type":"message.received",
        "occurred_at":chrono::DateTime::from_timestamp(at, 0).unwrap().to_rfc3339(),
        "payload":{"id":format!("sms-{id}"), "direction":"inbound",
            "from":{"phone_number":from},"to":[{"phone_number":to}],"text":text}
    })
}

/// This integration binary has one test. Set its fixture key before any server
/// starts and restore the inherited environment after all owned servers stop.
pub struct FixtureKey(Option<std::ffi::OsString>);
impl FixtureKey {
    pub fn install() -> Self {
        let previous = std::env::var_os("TELNYX_PUBLIC_KEY");
        let key = SigningKey::from_bytes(&[29; 32]);
        std::env::set_var(
            "TELNYX_PUBLIC_KEY",
            STANDARD.encode(key.verifying_key().to_bytes()),
        );
        Self(previous)
    }
}
impl Drop for FixtureKey {
    fn drop(&mut self) {
        match &self.0 {
            Some(value) => std::env::set_var("TELNYX_PUBLIC_KEY", value),
            None => std::env::remove_var("TELNYX_PUBLIC_KEY"),
        }
    }
}
