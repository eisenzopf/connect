//! Fake private provider lifecycle around the actual bootstrap/MCP adapter.
#![allow(clippy::unwrap_used, dead_code)]
use connect_connectors::{vapi_origin::OriginKind, Vapi};
use connect_mcp::RunningMcp;
use connect_server::vapi_bootstrap::{self, BootstrapAuth, Definition};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

pub struct Setup {
    pub api: MockServer,
    pub provider: Arc<Vapi>,
    pub endpoint: RunningMcp,
    pub directory: PathBuf,
    pub state: Value,
    pub resource_id: String,
}
impl Setup {
    pub async fn start(f: &super::fixture::Fixture, status: u16, delay: Duration) -> Self {
        Self::for_task(f, &f.task, "bootstrap", status, delay).await
    }
    pub async fn for_task(
        f: &super::fixture::Fixture,
        task: &connect_protocol::tasks::TaskRecord,
        label: &str,
        status: u16,
        delay: Duration,
    ) -> Self {
        let api = MockServer::start().await;
        let provider = Arc::new(Vapi::new("private-bootstrap-test", &api.uri()).unwrap());
        let directory = f.directory().join(label);
        let endpoint = RunningMcp::start_local(
            f.server.service.clone(),
            Arc::new(BootstrapAuth::new(
                f.server.service.clone(),
                provider.clone(),
                directory.clone(),
            )),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let definition = Definition {
            credential_id: f.owner.credential_id.clone(),
            request_id: format!("{label}-final-grant"),
            task_id: task.id.clone(),
            assistant_id: "assistant-one".into(),
            origin: OriginKind::ChatSession,
            operations: task.scope.operations.clone(),
            expires_at: chrono::Utc::now().timestamp() + 120,
        };
        vapi_bootstrap::prepare(
            f.server.service.clone(),
            definition,
            &format!("http://{}/mcp", endpoint.address),
            &directory,
        )
        .await
        .unwrap();
        let state = read(&directory);
        let resource_id = if label == "bootstrap" {
            "session-one".into()
        } else {
            format!("{label}-session")
        };
        let resource = json!({"id":resource_id,"assistantId":"assistant-one","status":"active","metadata":{"variableValues":{"connect_invocation_id":state["nonce"]}}});
        Mock::given(method("GET"))
            .and(path(format!("/session/{resource_id}")))
            .and(header("Authorization", "Bearer private-bootstrap-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(resource.clone()))
            .mount(&api)
            .await;
        Mock::given(method("POST"))
            .and(path("/session"))
            .and(header("Authorization", "Bearer private-bootstrap-test"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(resource)
                    .set_delay(delay),
            )
            .expect(1)
            .mount(&api)
            .await;
        Self {
            api,
            provider,
            endpoint,
            directory,
            state,
            resource_id,
        }
    }
    pub async fn posted(&self) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if self
                    .api
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.method == "POST")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    pub fn secret(&self) -> &str {
        self.state["secret"].as_str().unwrap()
    }
    pub async fn uncertain(&self, f: &super::fixture::Fixture) {
        assert!(vapi_bootstrap::create_session(
            f.server.service.clone(),
            &self.provider,
            &self.directory,
        )
        .await
        .is_err());
        assert_eq!(read(&self.directory)["posted"], true);
    }
    pub async fn close(self) {
        self.endpoint.shutdown().await.unwrap();
        self.api.verify().await;
    }
}
pub fn read(directory: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(directory.join("intent.json")).unwrap()).unwrap()
}
