//! Protected developer setup helpers; model operations stay on the actual MCP wire.
#![allow(clippy::unwrap_used, dead_code)]
use super::fixture::Fixture;
use connect_protocol::tasks::*;
use std::time::Duration;

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}
pub fn url(f: &Fixture) -> String {
    format!("http://{}/v1/task-access", f.server.http_address.unwrap())
}
pub fn request(task: &str, id: &str, kind: InvocationKind) -> TaskAccessRequest {
    TaskAccessRequest::new(TaskAccessCommand::Issue {
        request_id: id.into(),
        definition: GrantDefinition {
            task_id: task.into(),
            binding: InvocationBinding {
                kind,
                adapter: "local-install-test".into(),
                external_id: id.into(),
            },
            operations: vec![TaskPermission::History],
            expires_at: chrono::Utc::now().timestamp() + 180,
        },
    })
}
pub async fn send(f: &Fixture, request: &TaskAccessRequest) -> TaskAccessResponse {
    let response = http()
        .post(url(f))
        .bearer_auth(&f.owner.credential)
        .json(request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    response.json().await.unwrap()
}
pub fn issued(response: TaskAccessResponse) -> ProtectedTaskGrant {
    assert_eq!(response.profile, TASK_ACCESS_PROFILE);
    let TaskAccessOutcome::Issued(grant) = response.result.unwrap() else {
        panic!("protected issuance")
    };
    grant
}
