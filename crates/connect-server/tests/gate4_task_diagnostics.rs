//! Native SDK and independent CLI diagnostics preserve journal/auth boundaries.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/task_diagnostics_fixture.rs"]
mod setup;
use connect_client::{
    application::ApplicationClient, task_diagnostics::*, ClientError, ConnectClient,
};
use connect_protocol::tasks::*;
use connect_store::Store;

fn available<T>(section: DiagnosticSection<T>) -> T {
    let DiagnosticSection::Available(value) = section else {
        panic!("available authorized view");
    };
    value
}
fn denied<T>(section: DiagnosticSection<T>) {
    assert!(matches!(section,DiagnosticSection::Unavailable {code} if code=="forbidden"));
}
fn forbidden(error: ClientError) {
    assert!(matches!(error,ClientError::Application(error) if error.code=="forbidden"));
}
async fn start() -> fixture::Fixture {
    fixture::Fixture::start_independent_permitted(vec![
        TaskPermission::Send,
        TaskPermission::History,
        TaskPermission::Inbox,
    ])
    .await
}
#[tokio::test]
async fn missing_notification_catches_up_exact_reply_without_consuming_or_exposing_content() {
    let f = start().await;
    let question = setup::question(&f, "PRIVATE-QUESTION-BODY").await;
    let mut client = ApplicationClient::open(setup::application(&f, setup::native(&f)))
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let view = client
        .task_diagnostics(&f.task.id, &Default::default(), 100)
        .await
        .unwrap();
    assert_eq!(view.task.state, TaskState::Waiting);
    assert_eq!(view.task.conversation_id, f.task.conversation_id);
    assert!(!view.changed_during_sample);
    let journal = available(view.journal);
    assert!(!journal.items.is_empty());
    let checkpoint = journal.next.clone();
    assert!(checkpoint.is_some());
    assert_eq!(available(view.sources).items.len(), 1);
    denied(view.unassigned);
    denied(view.server_readiness);
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    drop(client);
    let reply = setup::answer(&f, question).await;
    let mut client = ApplicationClient::open(setup::application(&f, setup::native(&f)))
        .await
        .unwrap();
    let after = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let view = client
        .task_diagnostics(
            &f.task.id,
            &DiagnosticCursors {
                journal: checkpoint.clone(),
                ..Default::default()
            },
            100,
        )
        .await
        .unwrap();
    assert_eq!(view.task.state, TaskState::Ready);
    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("PRIVATE-QUESTION-BODY") && !encoded.contains("PRIVATE-ANSWER-BODY"));
    assert!(!encoded.contains(f.issued.expose_secret()) && !encoded.contains(&f.owner.credential));
    let journal = available(view.journal);
    assert_eq!(journal.after, checkpoint);
    assert!(journal.items.iter().any(|item|matches!(&item.observation,DiagnosticObservation::Reply(value) if value.event_id==reply.event_id && !value.consumed)));
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        after
    );
    f.server
        .service
        .revoke_task_grant(f.owner.credential_id.clone(), f.issued.grant.id.clone())
        .await
        .unwrap();
    let revoked = Store::inspect_snapshot(f.server.store.path()).unwrap();
    forbidden(
        client
            .task_diagnostics(&f.task.id, &Default::default(), 100)
            .await
            .unwrap_err(),
    );
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        revoked
    );
    drop(client);
    f.close().await;
}
#[tokio::test]
async fn owner_sees_unassigned_metadata_and_explicit_paging_while_bad_cursor_fails() {
    let f = start().await;
    setup::question(&f, "PRIVATE-QUESTION-BODY").await;
    let mut human = ConnectClient::open(f.peer.clone()).await.unwrap();
    human
        .send(
            "unbound-message",
            &f.task.conversation_id,
            "PRIVATE-UNASSIGNED-BODY",
        )
        .await
        .unwrap();
    human.close().await;
    let mut owner = ApplicationClient::open(setup::application(&f, setup::owner(&f)))
        .await
        .unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let view = owner
        .task_diagnostics(&f.task.id, &Default::default(), 1)
        .await
        .unwrap();
    assert_eq!(view.task.state, TaskState::Waiting);
    let unassigned = available(view.unassigned);
    assert_eq!(unassigned.items.len(), 1);
    assert!(unassigned.items[0].task_id.is_none());
    let sources = available(view.sources);
    assert_eq!(sources.items.len(), 1);
    assert_eq!(sources.omitted, Some(1));
    assert!(sources.next.is_some());
    let readiness = available(view.server_readiness);
    assert!(readiness
        .components
        .iter()
        .any(|c| c.component == "mcp" && c.configured));
    let error = owner
        .task_diagnostics(
            &f.task.id,
            &DiagnosticCursors {
                journal: sources.next,
                ..Default::default()
            },
            1,
        )
        .await
        .unwrap_err();
    assert!(matches!(error,ClientError::Application(error) if error.code=="invalid-request"));
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    drop(owner);
    f.close().await;
}
#[tokio::test]
async fn independent_cli_uses_scoped_sdk_and_rejects_unauthorized_task_without_fallback() {
    let f = start().await;
    setup::question(&f, "PRIVATE-QUESTION-BODY").await;
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    let output = setup::cli(&f).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let view: TaskDiagnostics = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(view.task.task_id, f.task.id);
    assert_eq!(view.task.state, TaskState::Waiting);
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.contains("PRIVATE-QUESTION-BODY") && !text.contains(f.issued.expose_secret()));
    let mut client = ApplicationClient::open(setup::application(&f, setup::native(&f)))
        .await
        .unwrap();
    forbidden(
        client
            .task_diagnostics("another-task", &Default::default(), 100)
            .await
            .unwrap_err(),
    );
    assert!(client
        .task_diagnostics(&f.task.id, &Default::default(), 0)
        .await
        .is_err());
    assert_eq!(
        Store::inspect_snapshot(f.server.store.path()).unwrap(),
        before
    );
    drop(client);
    f.close().await;
}
