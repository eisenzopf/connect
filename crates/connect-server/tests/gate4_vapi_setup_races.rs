//! The final issuance checks current authority after an explicitly paused lookup.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/paused_vapi_lookup.rs"]
mod paused;
#[path = "support/vapi_setup.rs"]
mod setup;
use connect_client::ConnectClient;
use connect_connectors::vapi_origin::OriginKind;
use connect_protocol::tasks::TaskCommand;
use connect_server::vapi_task_setup;
use connect_store::Store;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parent_revocation_and_task_cancellation_during_lookup_cannot_issue() {
    for cancelled in [false, true] {
        let (api, sdk) = paused::PausedLookup::start().await;
        let f = fixture::Fixture::start().await;
        let request = setup::setup(&f, OriginKind::Call);
        let service = f.server.service.clone();
        let job = tokio::spawn(async move { vapi_task_setup::issue(service, &sdk, request).await });
        api.wait_for_lookup().await;
        if cancelled {
            let mut client = ConnectClient::open(f.owner.clone()).await.unwrap();
            client
                .negotiate_tasks("connect:agent@local", Some(&f.task.conversation_id))
                .await
                .unwrap();
            client
                .task(TaskCommand::Cancel {
                    request_id: "cancel-paused-setup".into(),
                    task_id: f.task.id.clone(),
                    reason: None,
                })
                .await
                .unwrap();
            client.close().await;
        } else {
            f.revoke_parent();
        }
        let after_change = Store::inspect_snapshot(f.server.store.path()).unwrap();
        assert!(
            !job.is_finished(),
            "lookup barrier released before authority changed"
        );
        api.release();
        let result = tokio::time::timeout(Duration::from_secs(10), job)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        assert_eq!(
            Store::inspect_snapshot(f.server.store.path()).unwrap(),
            after_change,
            "final provider setup mutated state under obsolete authority"
        );
        api.close().await;
        f.close().await;
    }
}
