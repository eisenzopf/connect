//! Local trusted-composition fixture; no Vapi account or webhook is contacted.
#![allow(clippy::unwrap_used, dead_code)]
use super::fixture::Fixture;
use connect_client::ConnectClient;
use connect_mcp::VapiOrigin;
use connect_protocol::tasks::*;
use connect_service::tasks::IssuedTaskGrant;
use std::time::Duration;

pub fn headers(origin: VapiOrigin, id: &str) -> reqwest_mcp::header::HeaderMap {
    let mut headers = reqwest_mcp::header::HeaderMap::new();
    match origin {
        VapiOrigin::Call => {
            headers.insert("x-call-id", id.parse().unwrap());
        }
        VapiOrigin::Chat => {
            headers.insert("x-chat-id", id.parse().unwrap());
        }
        VapiOrigin::ChatSession => {
            headers.insert("x-session-id", id.parse().unwrap());
            headers.insert("x-chat-id", "chat-in-this-session".parse().unwrap());
            headers.insert("x-call-id", "chat-in-this-session".parse().unwrap());
        }
    }
    headers
}

pub async fn grant(
    f: &Fixture,
    task: &TaskRecord,
    origin: VapiOrigin,
    id: &str,
    expires_at: i64,
) -> (InvocationBinding, IssuedTaskGrant) {
    let binding = InvocationBinding {
        kind: if origin == VapiOrigin::Call {
            InvocationKind::ProviderCall
        } else {
            InvocationKind::ProviderChat
        },
        adapter: "vapi-local-fixture".into(),
        external_id: id.into(),
    };
    f.server
        .service
        .register_task_binding(
            f.owner.credential_id.clone(),
            task.id.clone(),
            binding.clone(),
        )
        .await
        .unwrap();
    let grant = f
        .server
        .service
        .issue_task_grant(
            f.owner.credential_id.clone(),
            format!("grant-{id}"),
            GrantDefinition {
                task_id: task.id.clone(),
                binding: binding.clone(),
                operations: task.scope.operations.clone(),
                expires_at,
            },
        )
        .await
        .unwrap();
    (binding, grant)
}

pub async fn second_task(f: &Fixture) -> TaskRecord {
    let mut client = ConnectClient::open(f.owner.clone()).await.unwrap();
    client
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    let TaskOutcome::Record(task) = client
        .task(TaskCommand::Create {
            request_id: "second-provider-task".into(),
            definition: TaskDefinition {
                conversation_id: f.task.conversation_id.clone(),
                scope: f.task.scope.clone(),
                expires_at: f.task.expires_at,
                initiating_event_id: None,
            },
        })
        .await
        .unwrap()
    else {
        panic!("task")
    };
    client.close().await;
    *task
}

pub async fn http(headers: reqwest_mcp::header::HeaderMap) -> reqwest_mcp::Client {
    tokio::task::spawn_blocking(move || {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .default_headers(headers)
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap()
}

pub fn mutation(id: &str) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
        "name":"connect_send","arguments":{"request_id":id,"step_id":"draft","message": connect_protocol::messages::MessageDraft::text_all(("Provider-scoped draft").to_string())}}})
}
