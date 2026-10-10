//! Independent local installations share a listener without sharing task authority.
#![allow(clippy::unwrap_used)]
#[path = "support/task_access.rs"]
mod access;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use connect_client::ConnectClient;
use connect_protocol::tasks::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_publicly_installed_grants_stay_separate_and_revoke_independently() {
    let f = fixture::Fixture::start_independent().await;
    let mut native = ConnectClient::open(f.owner.clone()).await.unwrap();
    native
        .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    let TaskOutcome::Record(second) = native
        .task(TaskCommand::Create {
            request_id: "second-local-task".into(),
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
        panic!("second task")
    };
    native.close().await;
    let first_request = access::request(&f.task.id, "first-install", InvocationKind::Native);
    let a = access::issued(access::send(&f, &first_request).await);
    let replay = access::issued(access::send(&f, &first_request).await);
    assert_eq!(a.grant, replay.grant);
    assert_eq!(a.expose_secret(), replay.expose_secret());
    let b = access::issued(
        access::send(
            &f,
            &access::request(&second.id, "second-install", InvocationKind::HostSession),
        )
        .await,
    );
    let ca = mcp::client_from_grant(f.server.mcp_address.unwrap(), a.expose_secret()).await;
    let cb = mcp::client_from_grant(f.server.mcp_address.unwrap(), b.expose_secret()).await;
    for (client, task) in [(&ca, &f.task.id), (&cb, &second.id)] {
        let status =
            mcp::success(mcp::call(client, "connect_task_status", serde_json::json!({})).await);
        assert_eq!(status["data"]["id"], *task);
        let tools = client.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|t| t.name == "connect_history"));
        assert!(!tools.iter().any(|t| t.name.contains("grant")));
        let forged = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("connect_task_status").with_arguments(
                    serde_json::json!({"task_id":second.id})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await;
        assert!(
            forged.is_err(),
            "closed schema accepted a model-selected task"
        );
    }
    let revoke = TaskAccessRequest::new(TaskAccessCommand::Revoke {
        grant_id: a.grant.id,
    });
    assert!(matches!(
        access::send(&f, &revoke).await.result.unwrap(),
        TaskAccessOutcome::Revoked { .. }
    ));
    assert!(ca
        .call_tool(rmcp::model::CallToolRequestParams::new(
            "connect_task_status"
        ))
        .await
        .is_err());
    assert_eq!(
        mcp::success(mcp::call(&cb, "connect_task_status", serde_json::json!({})).await)["data"]
            ["id"],
        second.id
    );
    f.revoke_parent();
    assert!(cb
        .call_tool(rmcp::model::CallToolRequestParams::new(
            "connect_task_status"
        ))
        .await
        .is_err());
    ca.cancel().await.unwrap();
    cb.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protected_api_denies_amplification_providers_and_forged_admission() {
    let f = fixture::Fixture::start_independent().await;
    let request = access::request(&f.task.id, "denial-install", InvocationKind::Native);
    let token = access::issued(access::send(&f, &request).await);
    let response = access::http()
        .post(access::url(&f))
        .bearer_auth(token.expose_secret())
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let response = access::http()
        .post(access::url(&f))
        .bearer_auth(&f.peer.credential)
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    for kind in [InvocationKind::ProviderCall, InvocationKind::ProviderChat] {
        assert_eq!(
            access::send(&f, &access::request(&f.task.id, "provider", kind))
                .await
                .result
                .unwrap_err()
                .code,
            "forbidden"
        );
    }
    for (name, value) in [
        ("origin", "http://localhost:8192"),
        ("x-call-id", "call"),
        ("x-chat-id", "chat"),
        ("host", "evil.example"),
    ] {
        let response = access::http()
            .post(access::url(&f))
            .bearer_auth(&f.owner.credential)
            .header(name, value)
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    }
    let response = access::http()
        .post(access::url(&f))
        .bearer_auth(&f.owner.credential)
        .header("authorization", format!("Bearer {}", f.peer.credential))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let mut unsupported = access::request(&f.task.id, "unsupported", InvocationKind::Native);
    unsupported.profile = "connect-task-access/2".into();
    assert_eq!(
        access::send(&f, &unsupported)
            .await
            .result
            .unwrap_err()
            .code,
        "unsupported-profile"
    );
    let mut conflict = request;
    let TaskAccessCommand::Issue { definition, .. } = &mut conflict.command else {
        unreachable!()
    };
    definition.binding.external_id = "another-origin".into();
    assert_eq!(
        access::send(&f, &conflict).await.result.unwrap_err().code,
        "request-conflict"
    );
    let response = access::http()
        .post(format!(
            "http://{}/v1/task-access",
            f.server.webhook_address.unwrap()
        ))
        .bearer_auth(&f.owner.credential)
        .json(&conflict)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    f.close().await;
}
