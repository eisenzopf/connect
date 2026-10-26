//! Protected fixture credentials; diagnostic operations use public clients.
#![allow(clippy::unwrap_used)]
use super::fixture::Fixture;
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{tasks::*, Operation};
pub fn native(f: &Fixture) -> ClientConfig {
    f.owner
        .clone()
        .with_task_grant(&ProtectedTaskGrant::new(
            f.issued.grant.clone(),
            f.issued.expose_secret().into(),
        ))
        .unwrap()
}
pub fn owner(f: &Fixture) -> ClientConfig {
    let credential = f
        .server
        .store
        .issue(
            &f.owner.identity_id,
            connect_store::Scope::operator(),
            f.task.expires_at,
        )
        .unwrap();
    let mut config = f.owner.clone();
    config.credential_id = credential.id;
    config.credential = credential.token;
    config
}
pub fn application(
    f: &Fixture,
    source: ClientConfig,
) -> connect_client::application::ApplicationClientConfig {
    connect_client::application::ApplicationClientConfig {
        endpoint: f
            .server
            .http_address
            .expect("explicit application listener"),
        credential: source.credential,
        expires_at: chrono::DateTime::from_timestamp(f.task.expires_at, 0).unwrap(),
        timeout_ms: 10_000,
    }
}
pub async fn question(f: &Fixture, body: &str) -> String {
    let mut client = ConnectClient::open(native(f)).await.unwrap();
    client.negotiate_task(&f.task.id).await.unwrap();
    client
        .task(TaskCommand::Invoke {
            request_id: "diagnostic-question".into(),
            task_id: f.task.id.clone(),
            step_id: "question".into(),
            operation: Box::new(Operation::Send {
                request_id: "diagnostic-question".into(),
                conversation_id: f.task.conversation_id.clone(),
                message: connect_protocol::messages::MessageDraft::text_all(body),
            }),
            reply_to: None,
            wait_for_reply: true,
        })
        .await
        .unwrap();
    client.close().await;
    let mut human = ConnectClient::open(f.peer.clone()).await.unwrap();
    let history = human
        .history(&f.task.conversation_id, None, 100)
        .await
        .unwrap();
    human.close().await;
    history
        .messages
        .iter()
        .find(|message| message.body == body)
        .unwrap()
        .id
        .clone()
}
pub async fn answer(f: &Fixture, question: String) -> TaskReply {
    let mut human = ConnectClient::open(f.peer.clone()).await.unwrap();
    human
        .negotiate_tasks("connect:reviews@local", Some(&f.task.conversation_id))
        .await
        .unwrap();
    let outcome = human
        .task(TaskCommand::Reply {
            request_id: "diagnostic-answer".into(),
            conversation_id: f.task.conversation_id.clone(),
            reply_to: question,
            body: "PRIVATE-ANSWER-BODY".into(),
        })
        .await
        .unwrap();
    human.close().await;
    let TaskOutcome::ReplyAccepted { reply, .. } = outcome else {
        panic!("reply receipt");
    };
    reply
}

/// A separate CLI process receives only the explicitly exported Native grant.
pub async fn cli(f: &Fixture) -> std::process::Output {
    let config = f.directory().join("diagnostic-native.toml");
    connect_server::cli::init::private_write(
        &config,
        toml::to_string(&application(f, native(f)))
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(20),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
            .arg("task-diagnostics")
            .arg("--config")
            .arg(&config)
            .arg("--task-id")
            .arg(&f.task.id)
            .env_clear()
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}
