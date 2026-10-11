//! Packaged exact-session renewal and reply continuation in the real local host.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[allow(dead_code)] // Shared helper also archives direct-runtime fixture output.
#[path = "support/codex_workflow_install.rs"]
mod install;
#[path = "support/codex_packaged.rs"]
mod packaged;
#[path = "support/codex_workflow_peer.rs"]
mod peer;
#[path = "support/codex_workflow_program.rs"]
mod program;
use connect_client::ConnectClient;
use connect_protocol::tasks::*;
use connect_server::{
    cli::{
        init::private_write,
        task_access::{self, codex_host, GrantFile},
    },
    config::{McpConfig, ServerConfig},
    runtime::RunningServer,
};
use connect_store::Store;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires selected installed Codex; packaged launcher uses scripted loopback Responses without inference"]
async fn packaged_host_renews_original_session_and_continues_reply_after_restart_without_repeating_effect(
) {
    let binary = std::path::PathBuf::from(
        std::env::var_os("CONNECT_TEST_CODEX_BINARY").expect("Select actual installed Codex"),
    );
    let f = fixture::Fixture::start_independent().await;
    let [a, b] = install::pair(&f).await;
    let peer = peer::Peer::start("Choose the release color for this draft.").await;
    let runtime = packaged::runtime(&a.directory, &binary, &peer.url);
    let first = packaged::succeeded(&packaged::run(&a.directory, &runtime, true).await);
    let saved = packaged::state(&a.directory);
    assert_eq!(first["session_id"], saved["session_id"]);
    assert_eq!(saved["task_id"], a.task.id);
    assert_eq!(peer.requests().len(), 2);

    // Trusted composition issues a replacement through the public protected
    // HTTP exchange. Neither that credential nor its config enters the host.
    let owner = f.directory().join("renewal-owner.toml");
    private_write(&owner, toml::to_string(&f.owner).unwrap().as_bytes()).unwrap();
    let input = f.directory().join("renewal-request.json");
    let grant_file = f.directory().join("renewed-grant.json");
    let request = TaskAccessRequest::new(TaskAccessCommand::Issue {
        request_id: "packaged-runtime-renewal".into(),
        definition: GrantDefinition {
            task_id: a.task.id.clone(),
            binding: a.grant.grant.binding.clone(),
            operations: a.grant.grant.operations.clone(),
            expires_at: chrono::Utc::now().timestamp() + 180,
        },
    });
    private_write(&input, &serde_json::to_vec(&request).unwrap()).unwrap();
    let http = f.server.http_address.unwrap();
    task_access::issue(&owner, http, &input, &grant_file)
        .await
        .unwrap();
    task_access::revoke(&owner, http, &a.grant.grant.id)
        .await
        .unwrap();
    let original_state = std::fs::read(a.directory.join("host-state.json")).unwrap();
    codex_host::renew(&a.directory, &grant_file).unwrap();
    assert_eq!(
        std::fs::read(a.directory.join("host-state.json")).unwrap(),
        original_state
    );
    let renewed: GrantFile = serde_json::from_slice(&std::fs::read(&grant_file).unwrap()).unwrap();
    assert_eq!(
        renewed.grant.as_ref().unwrap().grant.binding.kind,
        InvocationKind::HostSession
    );

    let directory = f.directory().join("state");
    let address = f.server.address;
    let mcp = f.server.mcp_address.unwrap();
    let peer_config = f.peer.clone();
    let parent_id = f.owner.credential_id.clone();
    let before = Store::inspect_snapshot(f.server.store.path()).unwrap();
    f.server.shutdown().await.unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
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

    let mut human = ConnectClient::open(peer_config).await.unwrap();
    let history = human
        .history(&a.task.conversation_id, None, 100)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 1);
    human
        .negotiate_tasks("connect:reviews@local", Some(&a.task.conversation_id))
        .await
        .unwrap();
    let answer = "Use indigo-927 for the release.";
    let reply = human
        .task(TaskCommand::Reply {
            request_id: "reply-after-restart".into(),
            conversation_id: a.task.conversation_id.clone(),
            reply_to: history.messages[0].id.clone(),
            body: answer.into(),
        })
        .await
        .unwrap();
    let TaskOutcome::ReplyAccepted {
        message_id, reply, ..
    } = reply
    else {
        panic!("reply");
    };
    assert_eq!(reply.task_id.as_deref(), Some(a.task.id.as_str()));
    peer.resume();
    let resumed = packaged::succeeded(&packaged::run(&a.directory, &runtime, false).await);
    assert_eq!(resumed["session_id"], saved["session_id"]);
    assert_eq!(resumed["task_id"], saved["task_id"]);
    let history = human
        .history(&a.task.conversation_id, None, 100)
        .await
        .unwrap();
    assert_eq!(history.messages.len(), 3);
    assert!(history
        .messages
        .iter()
        .any(|m| m.body == format!("Confirmed from {message_id}: {answer}")));
    assert!(human
        .history(&b.task.conversation_id, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    human.close().await;
    let before = Store::inspect_snapshot(server.store.path()).unwrap();
    peer.replay();
    let replay = packaged::succeeded(&packaged::run(&a.directory, &runtime, false).await);
    assert_eq!(replay["session_id"], saved["session_id"]);
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    let requests = peer.requests();
    assert_eq!(requests.len(), 7);
    let wire = serde_json::to_string(&requests).unwrap();
    for secret in [
        a.grant.expose_secret(),
        renewed.grant.as_ref().unwrap().expose_secret(),
        f.owner.credential.as_str(),
    ] {
        assert!(!wire.contains(secret));
    }
    let operator = connect_client::ClientConfig::load(&directory.join("operator.toml")).unwrap();
    let admin = server.store.authenticate(&operator.credential).unwrap();
    server.store.revoke(&admin, &parent_id).unwrap();
    let before = Store::inspect_snapshot(server.store.path()).unwrap();
    assert!(!packaged::run(&a.directory, &runtime, false)
        .await
        .status
        .success());
    assert_eq!(
        packaged::state(&a.directory)["session_id"],
        saved["session_id"]
    );
    assert_eq!(
        peer.requests().len(),
        7,
        "Required MCP denial precedes model dispatch"
    );
    assert_eq!(
        Store::inspect_snapshot(server.store.path()).unwrap(),
        before
    );
    server.shutdown().await.unwrap();
}
