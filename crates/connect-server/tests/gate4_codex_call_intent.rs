//! Actual runtime intent persistence/replay with a scripted local Responses peer.
#![recursion_limit = "256"]
#![allow(clippy::unwrap_used)]
#[path = "support/codex_call_barrier.rs"]
mod barrier;
#[path = "support/codex_exec.rs"]
mod exec;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_install.rs"]
mod install;
#[path = "support/codex_responses.rs"]
mod responses;
#[path = "support/codex_intent.rs"]
mod scenario;
use connect_client::ConnectClient;
use connect_protocol::tasks::TaskState;
use connect_store::Store;
use scenario::Case;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly selected installed Codex; scripted peer executes no inference"]
async fn persisted_call_precedes_dispatch_and_exact_resume_reconciles_one_effect() {
    let c = Case::start(
        "codex-intent-send",
        "Please review the original local draft.",
    )
    .await;
    let before = Store::inspect_snapshot(c.f.server.store.path()).unwrap();
    let mut first = c.launch(None);
    let saved = c.checkpoint(&first).await;
    assert_eq!(
        Store::inspect_snapshot(c.f.server.store.path()).unwrap(),
        before,
        "No operation may reach Connect while admission is paused"
    );
    let session = first.session();
    c.barrier.resume();
    c.returned().await;
    let after = Store::inspect_snapshot(c.f.server.store.path()).unwrap();
    assert_ne!(after, before);
    let mut owner = ConnectClient::open(c.f.owner.clone()).await.unwrap();
    assert_eq!(
        owner.negotiate_task(&c.f.task.id).await.unwrap().state,
        TaskState::Waiting
    );
    owner.close().await;
    let mut peer = ConnectClient::open(c.f.peer.clone()).await.unwrap();
    let history = peer
        .history(&c.f.task.conversation_id, None, 100)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].body,
        c.arguments["body"].as_str().unwrap()
    );
    peer.close().await;
    first.kill().await;
    c.responses.replay();
    let mut resumed = c.launch(Some(&session));
    resumed.finish().await;
    assert_eq!(resumed.session(), session);
    assert_eq!(
        Store::inspect_snapshot(c.f.server.store.path()).unwrap(),
        after,
        "Same-intent replay must not duplicate accepted effects"
    );
    let requests = c.responses.requests();
    assert_eq!(requests.len(), 4);
    let skill = include_str!("../../../skills/connect/SKILL.md");
    let body = skill.split_once("\n---\n").unwrap().1.trim();
    assert!(requests[0]["input"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .any(|content| content["text"]
            .as_str()
            .is_some_and(|text| text.contains(body))));
    assert!(requests[2]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "custom_tool_call"
            && item["call_id"] == "intent-call"
            && item["input"] == saved["input"]));
    c.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly selected installed Codex; scripted peer executes no inference"]
async fn interrupted_before_dispatch_replays_original_program_after_listener_restart() {
    let mut c = Case::start(
        "before-dispatch-send",
        "This original draft was not dispatched before interruption.",
    )
    .await;
    let before = Store::inspect_snapshot(c.f.server.store.path()).unwrap();
    let mut first = c.launch(None);
    let saved = c.checkpoint(&first).await;
    let session = first.session();
    first.kill().await;
    c.restart_mcp().await;
    assert_eq!(
        Store::inspect_snapshot(c.f.server.store.path()).unwrap(),
        before
    );
    c.responses.replay();
    let mut resumed = c.launch(Some(&session));
    resumed.finish().await;
    assert_eq!(resumed.session(), session);
    let mut peer = ConnectClient::open(c.f.peer.clone()).await.unwrap();
    let history = peer
        .history(&c.f.task.conversation_id, None, 100)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    assert_eq!(
        history.messages[0].body,
        c.arguments["body"].as_str().unwrap()
    );
    peer.close().await;
    let requests = c.responses.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[1]["input"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["type"] == "custom_tool_call"
            && item["call_id"] == "intent-call"
            && item["input"] == saved["input"]));
    c.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires explicitly selected installed Codex; scripted peer executes no inference"]
async fn current_revocation_after_intent_persistence_denies_dispatch_and_required_startup() {
    let c = Case::start(
        "revoked-send",
        "This revoked draft must never be dispatched.",
    )
    .await;
    let mut first = c.launch(None);
    c.checkpoint(&first).await;
    let session = first.session();
    c.f.server
        .service
        .revoke_task_grant(c.f.owner.credential_id.clone(), c.f.issued.grant.id.clone())
        .await
        .unwrap();
    let before = Store::inspect_snapshot(c.f.server.store.path()).unwrap();
    c.barrier.resume();
    c.returned().await;
    assert_eq!(
        Store::inspect_snapshot(c.f.server.store.path()).unwrap(),
        before
    );
    first.kill().await;
    c.responses.replay();
    let mut resumed = c.launch(Some(&session));
    resumed.rejected().await;
    assert_eq!(
        c.responses.requests().len(),
        2,
        "Required revoked MCP must stop before another scripted turn"
    );
    assert_eq!(
        Store::inspect_snapshot(c.f.server.store.path()).unwrap(),
        before
    );
    c.close().await;
}
