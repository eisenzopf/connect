//! Actual installed runtime continuation from a human reply, without inference.
#![allow(clippy::unwrap_used)]
#[allow(dead_code)] // The shared interruption helper also exposes unrelated probes.
#[path = "support/codex_exec.rs"]
mod exec;
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/codex_workflow_install.rs"]
mod install;
#[path = "support/codex_workflow_peer.rs"]
mod peer;
#[path = "support/codex_workflow_program.rs"]
mod program;
use connect_client::ConnectClient;
use connect_protocol::tasks::*;
use connect_server::{
    config::{McpConfig, ServerConfig},
    runtime::RunningServer,
};
use connect_store::Store;
use install::Host;
use std::{net::SocketAddr, path::Path};

fn launch(
    binary: &Path,
    host: &Host,
    peer: &peer::Peer,
    address: SocketAddr,
    session: Option<&str>,
) -> exec::Exec {
    exec::Exec::start(
        binary,
        &host.directory,
        &peer.url,
        address,
        host.grant.expose_secret(),
        session,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires selected installed Codex; scripted loopback peer executes no inference"]
async fn two_installed_sessions_resume_bound_reply_after_server_restart_and_replay_one_follow_up() {
    let binary = std::path::PathBuf::from(
        std::env::var_os("CONNECT_TEST_CODEX_BINARY").expect("Select actual installed Codex"),
    );
    let f = fixture::Fixture::start_independent().await;
    let [a, b] = install::pair(&f).await;
    let pa = peer::Peer::start("Please choose the draft's release color.").await;
    let pb = peer::Peer::start("SEPARATE-CONVERSATION-CANARY: choose a meeting time.").await;
    let address = f.server.mcp_address.unwrap();
    let mut first_a = launch(&binary, &a, &pa, address, None);
    let mut first_b = launch(&binary, &b, &pb, address, None);
    tokio::join!(first_a.finish(), first_b.finish());
    let session_a = first_a.session();
    let session_b = first_b.session();
    assert_ne!(session_a, session_b);
    let mut human = ConnectClient::open(f.peer.clone()).await.unwrap();
    let question = human
        .history(&a.task.conversation_id, None, 100)
        .await
        .unwrap()
        .messages;
    assert_eq!(question.len(), 1);
    human
        .negotiate_tasks("connect:reviews@local", Some(&a.task.conversation_id))
        .await
        .unwrap();
    let answer = "Use cerulean-583 for this release.";
    let reply = human
        .task(TaskCommand::Reply {
            request_id: "human-bound-answer".into(),
            conversation_id: a.task.conversation_id.clone(),
            reply_to: question[0].id.clone(),
            body: answer.into(),
        })
        .await
        .unwrap();
    let TaskOutcome::ReplyAccepted {
        message_id, reply, ..
    } = reply
    else {
        panic!("reply receipt");
    };
    assert_eq!(reply.task_id.as_deref(), Some(a.task.id.as_str()));
    assert_eq!(reply.reply_to.as_deref(), Some(question[0].id.as_str()));
    human.close().await;

    // Both actual host processes have exited. Reopen the same Connect database
    // and endpoint, retaining grants, questions, human reply and saved sessions.
    let directory = f.directory().join("state");
    let native_address = f.server.address;
    let peer_config = f.peer.clone();
    let owner_config = f.owner.clone();
    let path = f.server.store.path().to_owned();
    let before = Store::inspect_snapshot(&path).unwrap();
    f.server.shutdown().await.unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.bind = native_address;
    config.mcp = Some(McpConfig {
        bind: address,
        binding: None,
    });
    let server = RunningServer::start_local(config, None, false)
        .await
        .unwrap();
    assert_eq!(Store::inspect_snapshot(&path).unwrap(), before);
    pa.resume();
    pb.resume();
    let mut resumed_a = launch(&binary, &a, &pa, address, Some(&session_a));
    let mut resumed_b = launch(&binary, &b, &pb, address, Some(&session_b));
    tokio::join!(resumed_a.finish(), resumed_b.finish());
    assert_eq!(resumed_a.session(), session_a);
    assert_eq!(resumed_b.session(), session_b);
    let mut human = ConnectClient::open(peer_config).await.unwrap();
    let history = human
        .history(&a.task.conversation_id, None, 100)
        .await
        .unwrap()
        .messages;
    assert_eq!(history.len(), 3);
    assert!(history
        .iter()
        .any(|message| message.body == format!("Confirmed from {message_id}: {answer}")));
    assert_eq!(
        human
            .history(&b.task.conversation_id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    human.close().await;
    let mut owner = ConnectClient::open(owner_config).await.unwrap();
    assert_eq!(
        owner.negotiate_task(&b.task.id).await.unwrap().state,
        TaskState::Waiting
    );
    owner.close().await;
    let after = Store::inspect_snapshot(&path).unwrap();
    a.archive_resume();
    pa.replay();
    let mut replay = launch(&binary, &a, &pa, address, Some(&session_a));
    replay.finish().await;
    assert_eq!(replay.session(), session_a);
    assert_eq!(
        Store::inspect_snapshot(&path).unwrap(),
        after,
        "Exact follow-up replay cannot create another communication effect"
    );
    let ar = pa.requests();
    let br = pb.requests();
    assert_eq!(ar.len(), 7);
    assert_eq!(br.len(), 4);
    assert_eq!(
        program::question_result(&ar[1]),
        program::question_result(&ar[2])
    );
    assert_eq!(
        program::question_result(&br[1]),
        program::question_result(&br[2])
    );
    assert_eq!(
        program::saved_follow_up(&ar[4]),
        program::saved_follow_up(&ar[5])
    );
    assert!(!serde_json::to_string(&ar)
        .unwrap()
        .contains("SEPARATE-CONVERSATION-CANARY"));
    assert!(!serde_json::to_string(&br).unwrap().contains(answer));
    for requests in [&ar, &br] {
        let wire = serde_json::to_string(requests).unwrap();
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
