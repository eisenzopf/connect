#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use connect_client::ClientConfig;
use connect_protocol::{Operation, Outcome};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::{Scope, Store};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SUBSCRIBE: u8 = 0x01;
const AUDIO: u8 = 0x02;
const CLOSE: u8 = 0x03;
const READY: u8 = 0x81;
const RECEIVED: u8 = 0x82;

async fn write_frame(writer: &mut tokio::process::ChildStdin, kind: u8, payload: &[u8]) {
    writer.write_u8(kind).await.unwrap();
    writer.write_u32(payload.len() as u32).await.unwrap();
    writer.write_all(payload).await.unwrap();
    writer.flush().await.unwrap();
}

async fn read_frame(reader: &mut tokio::process::ChildStdout) -> (u8, Vec<u8>) {
    let kind = reader.read_u8().await.unwrap();
    let length = reader.read_u32().await.unwrap();
    assert!(length <= 4096, "media helper response must remain bounded");
    let mut payload = vec![0; length as usize];
    reader.read_exact(&mut payload).await.unwrap();
    (kind, payload)
}

fn save_config(path: &Path, config: &ClientConfig) {
    std::fs::write(path, toml::to_string(config).unwrap()).unwrap();
}

fn spawn_helper(
    config: &Path,
    conversation_id: &str,
    session_id: &str,
    remote_identity_id: &str,
) -> tokio::process::Child {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .arg("media-stdio")
        .arg("--config")
        .arg(config)
        .arg("--conversation-id")
        .arg(conversation_id)
        .arg("--session-id")
        .arg(session_id)
        .arg("--remote-identity-id")
        .arg(remote_identity_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn native_language_helper_carries_raw_audio_over_real_quic() {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let mut agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let store = Store::open(&config.database, config.limits.clone()).unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let app_key = store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = store
        .issue(
            &agent.identity_id,
            Scope::communications(vec![app.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let conversation = match store
        .execute(
            &app_key.id,
            &Operation::Create {
                request_id: "helper-create".into(),
                request: connect_protocol::conversations::CreateRequest::persistent(vec![agent
                    .identity_id
                    .clone()]),
            },
        )
        .unwrap()
    {
        Outcome::Conversation(value) => value,
        other => panic!("expected conversation, got {other:?}"),
    };
    let session = match store
        .execute(
            &app_key.id,
            &Operation::SessionInvite {
                request_id: "helper-invite".into(),
                conversation_id: conversation.id.clone(),
                participant_id: agent.identity_id.clone(),
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    {
        Outcome::Session(value) => value,
        other => panic!("expected session, got {other:?}"),
    };
    store
        .execute(
            &agent_key.id,
            &Operation::SessionAccept {
                request_id: "helper-accept".into(),
                conversation_id: conversation.id.clone(),
                session_id: session.id.clone(),
            },
        )
        .unwrap();
    drop(store);

    let server = RunningServer::start(config).await.unwrap();
    app.endpoint = server.address;
    app.credential = app_key.token;
    app.credential_id = app_key.id;
    agent.endpoint = server.address;
    agent.credential = agent_key.token;
    agent.credential_id = agent_key.id;
    let app_path = temporary.path().join("app.toml");
    let agent_path = temporary.path().join("agent.toml");
    save_config(&app_path, &app);
    save_config(&agent_path, &agent);

    let mut app_helper = spawn_helper(&app_path, &conversation.id, &session.id, &agent.identity_id);
    let mut agent_helper =
        spawn_helper(&agent_path, &conversation.id, &session.id, &app.identity_id);
    let mut app_in = app_helper.stdin.take().unwrap();
    let mut app_out = app_helper.stdout.take().unwrap();
    let mut agent_in = agent_helper.stdin.take().unwrap();
    let mut agent_out = agent_helper.stdout.take().unwrap();
    let (app_kind, app_stream) = read_frame(&mut app_out).await;
    let (agent_kind, agent_stream) = read_frame(&mut agent_out).await;
    assert_eq!((app_kind, agent_kind), (READY, READY));
    write_frame(&mut app_in, SUBSCRIBE, &agent_stream).await;
    write_frame(&mut agent_in, SUBSCRIBE, &app_stream).await;
    let audio = [0x5a; 160];
    write_frame(&mut app_in, AUDIO, &audio).await;
    tokio::time::sleep(Duration::from_millis(25)).await;
    write_frame(&mut app_in, AUDIO, &audio).await;
    let received = tokio::time::timeout(Duration::from_secs(3), read_frame(&mut agent_out))
        .await
        .unwrap();
    assert_eq!(received, (RECEIVED, audio.to_vec()));

    write_frame(&mut app_in, CLOSE, &[]).await;
    write_frame(&mut agent_in, CLOSE, &[]).await;
    tokio::time::timeout(Duration::from_secs(3), app_helper.wait())
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), agent_helper.wait())
        .await
        .unwrap()
        .unwrap();
    server.shutdown().await.unwrap();
}
