//! Current owner diagnostics inspect retained work without restoring delegation.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
#[path = "support/agent_task_fixture.rs"]
mod tracked;
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    task_diagnostics::DiagnosticSection,
    tasks::*,
    ClientError,
};
use serde_json::json;
use std::time::Duration;

fn application(
    f: &agent::Fixture,
    source: &connect_client::ClientConfig,
) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: source.credential.clone(),
        expires_at: chrono::DateTime::from_timestamp(f.expires_at, 0).unwrap(),
        timeout_ms: 10_000,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fresh_owner_inspection_discovers_effects_and_runs_documented_cli_after_revoke() {
    let f = agent::Fixture::start().await;
    let mut native = f.agent_client().await;
    let cid = native
        .create("retained-inspection-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let original = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let task = tracked::create(&original, &cid, "retained-inspection-task").await;
    tracked::question(&original, &task, "retained-inspection-question").await;
    f.revoke(&f.agent.credential_id);

    let inspection = tracked::owner(&f);
    let client = mcp::client(f.server.mcp_address.unwrap(), &inspection.credential).await;
    // Discover the exact effect from retained journal metadata after revocation,
    // rather than relying on an effect ID cached before the security action.
    let events = tracked::journal(&client, &task).await;
    let effect = events
        .iter()
        .find_map(|event| match &event.data {
            TaskEventData::Effect(effect) => Some(effect.id.clone()),
            _ => None,
        })
        .unwrap();
    assert!(matches!(tracked::task(mcp::outcome(mcp::call(&client,
        "connect_effect_status", json!({"task_id":task.id,"effect_id":effect})).await)),
        TaskOutcome::Effect(effect) if effect.outcome.is_some()));
    let mut current = ApplicationClient::open(application(&f, &inspection))
        .await
        .unwrap();
    current.negotiate_task(&task.id).await.unwrap();
    let diagnostics = current
        .task_diagnostics(&task.id, &Default::default(), 100)
        .await
        .unwrap();
    assert!(
        matches!(diagnostics.journal, DiagnosticSection::Available(ref page)
        if page.items.iter().any(|event| matches!(event.observation,
            connect_client::task_diagnostics::DiagnosticObservation::Effect {..})))
    );
    assert!(matches!(
        diagnostics.sources,
        DiagnosticSection::Available(_)
    ));

    let path = f
        .server_config_path()
        .with_file_name("retained-inspection.toml");
    connect_server::cli::init::private_write(
        &path,
        toml::to_string_pretty(&application(&f, &inspection))
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
    command
        .args(["task-diagnostics", "--config"])
        .arg(&path)
        .args(["--task-id", &task.id])
        .env_remove("TELNYX_API_KEY")
        .env_remove("TELNYX_PUBLIC_KEY")
        .env_remove("VAPI_PRIVATE_KEY")
        .env_remove("VAPI_API_KEY")
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "current owner diagnostic command succeeds"
    );
    let cli: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(cli["journal"]["availability"], "available");
    assert_eq!(cli["sources"]["availability"], "available");

    // Current inspection does not revive the original task's mutations.
    let denied = mcp::call(
        &client,
        "connect_task_send",
        json!({"task_id":task.id,
        "conversation_id":cid,"request_id":"revoked-task-effect","step_id":"revoked-task-effect",
        "message": connect_protocol::messages::MessageDraft::text_all(("Must remain denied.").to_string())}),
    )
    .await;
    assert_eq!(mcp::error(denied).code, "unauthorized");
    tracked::record(mcp::outcome(
        mcp::call(
            &client,
            "connect_task_cancel",
            json!({"request_id":"retained-inspection-cancel","task_id":task.id}),
        )
        .await,
    ));
    assert!(!tracked::journal(&client, &task).await.is_empty());
    let diagnostics = current
        .task_diagnostics(&task.id, &Default::default(), 100)
        .await
        .unwrap();
    assert_eq!(diagnostics.task.state, TaskState::Cancelled);
    assert!(matches!(
        diagnostics.journal,
        DiagnosticSection::Available(_)
    ));
    drop(current);
    native.close().await;
    client.cancel().await.unwrap();
    original.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retained_diagnostics_do_not_mask_revocation_of_the_current_reader() {
    let f = agent::Fixture::start().await;
    let mut native = f.agent_client().await;
    let cid = native
        .create("reader-revoke-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let original = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let task = tracked::create(&original, &cid, "reader-revoke-task").await;
    let inspection = tracked::owner(&f);
    let mut reader = ApplicationClient::open(application(&f, &inspection))
        .await
        .unwrap();
    reader.negotiate_task(&task.id).await.unwrap();
    f.revoke(&f.agent.credential_id);
    assert!(reader
        .task_diagnostics(&task.id, &Default::default(), 100)
        .await
        .is_ok());
    f.revoke(&inspection.credential_id);
    assert!(
        matches!(reader.task_diagnostics(&task.id, &Default::default(), 100).await,
        Err(ClientError::Application(error)) if error.code == "unauthorized")
    );
    drop(reader);
    native.close().await;
    original.cancel().await.unwrap();
    f.close().await;
}
