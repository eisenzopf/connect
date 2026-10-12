//! A verified callback or recovered receipt still needs current task authority.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_bootstrap.rs"]
mod bootstrap;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/paused_vapi_lookup.rs"]
mod paused;
use connect_client::ConnectClient;
use connect_mcp::McpAuthenticator;
use connect_protocol::tasks::TaskCommand;
use connect_server::vapi_bootstrap::{self, BootstrapAuth};
use connect_store::Store;
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_and_parent_revocation_while_callback_is_verified_cannot_issue() {
    for cancel in [false, true] {
        let f = fixture::Fixture::start().await;
        let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
        setup.uncertain(&f).await;
        let (barrier, sdk) = paused::PausedLookup::with_response("/session/session-one", json!({"id":"session-one","assistantId":"assistant-one","status":"active","metadata":{"variableValues":{"connect_invocation_id":setup.state["nonce"]}}})).await;
        let auth = BootstrapAuth::new(
            f.server.service.clone(),
            Arc::new(sdk),
            setup.directory.clone(),
        );
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", setup.secret()).parse().unwrap(),
        );
        headers.insert("x-session-id", "session-one".parse().unwrap());
        headers.insert("x-chat-id", "first-chat".parse().unwrap());
        let job = tokio::spawn(async move { auth.authenticate(&headers).await });
        barrier.wait_for_lookup().await;
        change_authority(&f, cancel).await;
        let after = Store::inspect_snapshot(f.server.store.path()).unwrap();
        assert!(!job.is_finished());
        barrier.release();
        assert!(job.await.unwrap().is_err());
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            after
        );
        // The private observation can remain saved; it is never itself authority.
        assert_eq!(
            bootstrap::read(&setup.directory)["external_id"],
            "session-one"
        );
        barrier.close().await;
        setup.close().await;
        f.close().await;
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovered_resource_return_rechecks_authority_after_private_lookup() {
    for cancel in [false, true] {
        let f = fixture::Fixture::start().await;
        let setup = bootstrap::Setup::start(&f, 200, Duration::ZERO).await;
        assert_eq!(
            vapi_bootstrap::create_session(
                f.server.service.clone(),
                &setup.provider,
                &setup.directory
            )
            .await
            .unwrap(),
            "session-one"
        );
        let state = bootstrap::read(&setup.directory);
        let (barrier, sdk) = paused::PausedLookup::with_response("/session/session-one", json!({"id":"session-one","assistantId":"assistant-one","status":"active","metadata":{"variableValues":{"connect_invocation_id":state["nonce"]}}})).await;
        let directory = setup.directory.clone();
        let service = f.server.service.clone();
        let job =
            tokio::spawn(
                async move { vapi_bootstrap::create_session(service, &sdk, &directory).await },
            );
        barrier.wait_for_lookup().await;
        change_authority(&f, cancel).await;
        let after = Store::inspect_snapshot(f.server.store.path()).unwrap();
        assert!(!job.is_finished());
        barrier.release();
        assert!(job.await.unwrap().is_err());
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            after
        );
        assert_eq!(bootstrap::read(&setup.directory), state);
        barrier.close().await;
        setup.close().await;
        f.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overlapping_callbacks_wait_and_recheck_authority_instead_of_denying_contention() {
    for cancel in [false, true] {
        let f = fixture::Fixture::start().await;
        let setup = bootstrap::Setup::start(&f, 500, Duration::ZERO).await;
        setup.uncertain(&f).await;
        let (barrier, sdk) = paused::PausedLookup::with_response(
            "/session/session-one",
            json!({"id":"session-one","assistantId":"assistant-one","status":"active",
                "metadata":{"variableValues":{"connect_invocation_id":setup.state["nonce"]}}}),
        )
        .await;
        let auth = Arc::new(BootstrapAuth::new(
            f.server.service.clone(),
            Arc::new(sdk),
            setup.directory.clone(),
        ));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "authorization",
            format!("Bearer {}", setup.secret()).parse().unwrap(),
        );
        headers.insert("x-session-id", "session-one".parse().unwrap());
        headers.insert("x-chat-id", "first-chat".parse().unwrap());
        headers.insert("x-call-id", "first-chat".parse().unwrap());
        let first_auth = auth.clone();
        let first_headers = headers.clone();
        let first = tokio::spawn(async move { first_auth.authenticate(&first_headers).await });
        barrier.wait_for_lookup().await;
        // Vapi opens an idle GET while listing or invoking a tool. Both requests
        // authenticate through this one endpoint, with the same protected origin.
        let second = auth.authenticate(&headers);
        tokio::pin!(second);
        let initial = tokio::time::timeout(Duration::from_millis(100), &mut second).await;
        let waited = initial.is_err();
        barrier.release();
        assert!(first.await.unwrap().is_ok());
        if cancel {
            change_authority(&f, true).await;
        }
        let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
        let result = match initial {
            Ok(result) => result,
            Err(_) => {
                barrier.release();
                tokio::time::timeout(Duration::from_secs(5), second)
                    .await
                    .unwrap()
            }
        };
        let unchanged = Store::inspect_snapshot(f.server.store.path()).unwrap() == before;
        barrier.close().await;
        setup.close().await;
        f.close().await;
        assert!(
            waited,
            "Overlapping authenticated requests must wait for verification"
        );
        assert_eq!(result.is_ok(), !cancel);
        assert!(
            unchanged,
            "Replayed admission must not create another grant or effect"
        );
    }
}
async fn change_authority(f: &fixture::Fixture, cancel: bool) {
    if cancel {
        let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
        owner
            .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
            .await
            .unwrap();
        owner
            .task(TaskCommand::Cancel {
                task_id: f.task.id.clone(),
                request_id: "cancel-paused-bootstrap".into(),
                reason: None,
            })
            .await
            .unwrap();
        owner.close().await;
    } else {
        f.revoke_parent();
    }
}
