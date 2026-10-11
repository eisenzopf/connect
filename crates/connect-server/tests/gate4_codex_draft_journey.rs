//! Two actual packaged host sessions share context while retaining reply bindings.
#![allow(clippy::unwrap_used)]
#[path = "support/codex_draft.rs"]
mod draft;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[allow(dead_code)] // Shared separate-conversation and direct-runtime fixtures.
#[path = "support/codex_workflow_install.rs"]
mod install;
#[path = "support/codex_packaged.rs"]
mod packaged;
#[allow(dead_code)] // This journey supplies artifact programs instead of plain questions.
#[path = "support/codex_workflow_peer.rs"]
mod peer;
#[allow(dead_code)]
#[path = "support/codex_workflow_program.rs"]
mod program;
use connect_client::ConnectClient;
use connect_protocol::tasks::*;
use connect_server::{
    config::{McpConfig, ServerConfig},
    runtime::RunningServer,
};
use connect_store::Store;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires installed Codex; deterministic local Responses, no model inference"]
async fn overlapping_drafts_preserve_bytes_context_and_bound_reply_after_restart() {
    let binary = std::path::PathBuf::from(
        std::env::var_os("CONNECT_TEST_CODEX_BINARY").expect("Select actual installed Codex"),
    );
    let permissions = vec![
        TaskPermission::Send,
        TaskPermission::History,
        TaskPermission::Inbox,
        TaskPermission::ArtifactBegin,
        TaskPermission::ArtifactAppend,
        TaskPermission::ArtifactFinish,
    ];
    let f = fixture::Fixture::start_independent_permitted(permissions.clone()).await;
    let [a, b] = install::pair_with_scope(&f, true, permissions).await;
    assert_ne!(a.task.id, b.task.id);
    assert_eq!(a.task.conversation_id, b.task.conversation_id);
    let bytes_a =
        b"Release proposal A: indigo buttons, keyboard navigation, and a concise changelog.";
    let bytes_b = b"Release proposal B: amber buttons and an expanded accessibility report.";
    let peer_a = peer::Peer::start_program(draft::program("release-a.txt", bytes_a)).await;
    let peer_b = peer::Peer::start_program(draft::program("release-b.txt", bytes_b)).await;
    let runtime_a = packaged::runtime(&a.directory, &binary, &peer_a.url);
    let runtime_b = packaged::runtime(&b.directory, &binary, &peer_b.url);
    let (first_a, first_b) = tokio::join!(
        packaged::run(&a.directory, &runtime_a, true),
        packaged::run(&b.directory, &runtime_b, true)
    );
    let first_a = packaged::succeeded(&first_a);
    let first_b = packaged::succeeded(&first_b);
    assert_ne!(first_a["session_id"], first_b["session_id"]);
    let mut human = ConnectClient::open(f.peer.clone()).await.unwrap();
    let cid = &a.task.conversation_id;
    let history = human.history(cid, None, 100).await.unwrap();
    assert_eq!(history.messages.len(), 2);
    let question_a = draft::verify(&mut human, &history.messages, "release-a.txt", bytes_a).await;
    let question_b = draft::verify(&mut human, &history.messages, "release-b.txt", bytes_b).await;
    assert_ne!(question_a, question_b);
    human.close().await;

    let state = f.directory().join("state");
    let address = f.server.address;
    let mcp = f.server.mcp_address.unwrap();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    f.server.shutdown().await.unwrap();
    let mut config = ServerConfig::load(&state.join("server.toml")).unwrap();
    config.bind = address;
    config.mcp = Some(McpConfig {
        bind: mcp,
        binding: None,
    });
    let server = RunningServer::start_local(config, None, false)
        .await
        .unwrap();
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    let mut human = ConnectClient::open(f.peer.clone()).await.unwrap();
    let history = human.history(cid, None, 100).await.unwrap();
    assert_eq!(
        draft::verify(&mut human, &history.messages, "release-a.txt", bytes_a).await,
        question_a
    );
    draft::verify(&mut human, &history.messages, "release-b.txt", bytes_b).await;
    human
        .negotiate_tasks("connect:reviews@local", Some(cid))
        .await
        .unwrap();
    let answer = "Approve proposal A with indigo-927; proposal B still needs review.";
    let TaskOutcome::ReplyAccepted {
        message_id, reply, ..
    } = human
        .task(TaskCommand::Reply {
            request_id: "draft-human-answer".into(),
            conversation_id: cid.clone(),
            reply_to: question_a,
            body: answer.into(),
        })
        .await
        .unwrap()
    else {
        panic!("bound reply")
    };
    assert_eq!(reply.task_id.as_deref(), Some(a.task.id.as_str()));
    peer_a.resume();
    peer_b.resume();
    let (resumed_a, resumed_b) = tokio::join!(
        packaged::run(&a.directory, &runtime_a, false),
        packaged::run(&b.directory, &runtime_b, false)
    );
    assert_eq!(
        packaged::succeeded(&resumed_a)["session_id"],
        first_a["session_id"]
    );
    assert_eq!(
        packaged::succeeded(&resumed_b)["session_id"],
        first_b["session_id"]
    );
    assert_eq!(peer_a.requests().len(), 5);
    assert_eq!(
        peer_b.requests().len(),
        4,
        "Unanswered task must not follow up on the other task's reply"
    );
    let history = human.history(cid, None, 100).await.unwrap();
    assert_eq!(history.messages.len(), 4);
    assert_eq!(
        history
            .messages
            .iter()
            .filter(|m| m.body == format!("Confirmed from {message_id}: {answer}"))
            .count(),
        1
    );
    draft::verify(&mut human, &history.messages, "release-a.txt", bytes_a).await;
    draft::verify(&mut human, &history.messages, "release-b.txt", bytes_b).await;
    human.close().await;

    let mut owner = ConnectClient::open(f.owner.clone()).await.unwrap();
    owner.negotiate_task(&b.task.id).await.unwrap();
    let TaskOutcome::Record(waiting) = owner
        .task(TaskCommand::Get {
            task_id: b.task.id.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("task record")
    };
    assert_eq!(waiting.state, TaskState::Waiting);
    owner.close().await;
    let before = Store::inspect_snapshot(server.store.path()).unwrap();
    peer_a.replay();
    assert_eq!(
        packaged::succeeded(&packaged::run(&a.directory, &runtime_a, false).await)["session_id"],
        first_a["session_id"]
    );
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before,
        "Saved follow-up replay must not create another effect or upload"
    );
    for peer in [&peer_a, &peer_b] {
        let wire = serde_json::to_string(&peer.requests()).unwrap();
        for secret in [
            a.grant.expose_secret(),
            b.grant.expose_secret(),
            f.owner.credential.as_str(),
        ] {
            assert!(!wire.contains(secret));
        }
    }
    server.shutdown().await.unwrap();
}
