//! Provider admission checks current expiry, cancellation and backing authority.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/vapi_bindings.rs"]
mod provider;
use connect_client::ConnectClient;
use connect_mcp::{RunningMcp, VapiBindingAuth, VapiOrigin};
use connect_protocol::tasks::{TaskCommand, TaskPermission};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_requests_stop_on_expiry_task_cancel_and_parent_revocation() {
    for cause in ["expiry", "cancel", "parent"] {
        let f = fixture::Fixture::start_permitted(
            false,
            vec![TaskPermission::History, TaskPermission::Send],
        )
        .await;
        let expires = chrono::Utc::now().timestamp() + if cause == "expiry" { 3 } else { 180 };
        let (binding, grant) =
            provider::grant(&f, &f.task, VapiOrigin::Call, "lifecycle-call", expires).await;
        let listener = RunningMcp::start_local(
            f.server.service.clone(),
            Arc::new(
                VapiBindingAuth::new(f.server.service.clone(), binding, VapiOrigin::Call).unwrap(),
            ),
            "127.0.0.1:0".parse().unwrap(),
        )
        .await
        .unwrap();
        let http = provider::http(provider::headers(VapiOrigin::Call, "lifecycle-call")).await;
        let url = format!("http://{}/mcp", listener.address);
        let request = || {
            http.post(&url)
                .header("accept", "application/json, text/event-stream")
                .bearer_auth(grant.expose_secret())
        };
        let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"provider-lifecycle","version":"1"}}});
        assert_eq!(
            request().json(&initialize).send().await.unwrap().status(),
            200
        );
        match cause {
            "expiry" => {
                while chrono::Utc::now().timestamp() < expires {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }
            "cancel" => {
                let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
                owner
                    .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
                    .await
                    .unwrap();
                owner
                    .task(TaskCommand::Cancel {
                        request_id: "cancel-provider-task".into(),
                        task_id: f.task.id.clone(),
                        reason: Some("local qualification".into()),
                    })
                    .await
                    .unwrap();
                owner.close().await;
            }
            "parent" => f.revoke_parent(),
            _ => unreachable!(),
        }
        let before = connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap();
        for body in [initialize, provider::mutation("after-authority-ended")] {
            assert_eq!(
                request().json(&body).send().await.unwrap().status(),
                401,
                "{cause}"
            );
        }
        assert_eq!(
            connect_store::Store::inspect_snapshot(f.server.store.path()).unwrap(),
            before
        );
        let mut peer = ConnectClient::open(f.peer.clone()).await.unwrap();
        assert!(peer
            .history(&f.task.conversation_id, None, 10)
            .await
            .unwrap()
            .messages
            .is_empty());
        peer.close().await;
        listener.shutdown().await.unwrap();
        f.close().await;
    }
}
