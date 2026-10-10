//! Developer setup uses the protected public HTTP exchange; peers use public SDKs.
#![allow(clippy::unwrap_used, dead_code)]
use super::native_tasks::Harness;
use connect_client::{tasks::*, ClientError};
use connect_protocol::{Operation, Response};

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap()
}
pub fn url(h: &Harness) -> String {
    format!("http://{}", h.server.http_address.unwrap())
}
pub async fn task(h: &mut Harness, id: &str) -> TaskRecord {
    let task = h.create(id).await;
    super::native_tasks::record(
        h.owner
            .task(TaskCommand::Transition {
                request_id: format!("{id}-run"),
                task_id: task.id,
                expected_revision: task.revision,
                state: TaskState::Running,
                reason: None,
            })
            .await
            .unwrap(),
    )
}
pub async fn issue(
    h: &Harness,
    task: &TaskRecord,
    id: &str,
    permissions: Vec<TaskPermission>,
    expiry: i64,
) -> ProtectedTaskGrant {
    let request = TaskAccessRequest::new(TaskAccessCommand::Issue {
        request_id: id.into(),
        definition: GrantDefinition {
            task_id: task.id.clone(),
            binding: InvocationBinding {
                kind: InvocationKind::Native,
                adapter: "sdk-native-test".into(),
                external_id: id.into(),
            },
            operations: permissions,
            expires_at: expiry,
        },
    });
    let response: TaskAccessResponse = http()
        .post(format!("{}/v1/task-access", url(h)))
        .bearer_auth(&h.owner_config.credential)
        .json(&request)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let TaskAccessOutcome::Issued(grant) = response.result.unwrap() else {
        panic!("grant")
    };
    grant
}
pub async fn operation(h: &Harness, token: &str, operation: Operation) -> Response {
    http()
        .post(format!("{}/v1/operations", url(h)))
        .bearer_auth(token)
        .json(&operation)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}
pub fn wrapped(task: &str, id: &str, operation: Operation) -> TaskCommand {
    TaskCommand::Invoke {
        task_id: task.into(),
        request_id: id.into(),
        step_id: id.into(),
        operation: Box::new(operation),
        reply_to: None,
        wait_for_reply: false,
    }
}
pub fn code(error: ClientError) -> String {
    let ClientError::Application(error) = error else {
        panic!("application rejection: {error}")
    };
    error.code
}
