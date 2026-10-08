#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
use connect_client::{ClientConfig, ClientError, ConnectClient, Operation, RequestState};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn typed_facade_and_cancellation_preserve_durable_ids_and_cursors() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    let mut config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut app = ConnectClient::open(config.clone()).await.unwrap();
    assert_eq!(app.health().await.unwrap().schema, 2);
    let agent = app.resolve("connect:agent@local").await.unwrap();
    let conversation = app.create("create", vec![agent.id.clone()]).await.unwrap();
    assert_eq!(
        conversation,
        app.create("create", vec![agent.id]).await.unwrap()
    );
    let receipt = app.send("send", &conversation.id, "hello").await.unwrap();
    assert_eq!(
        receipt,
        app.send("send", &conversation.id, "hello").await.unwrap()
    );
    assert_eq!(
        app.history(&conversation.id, None, 100)
            .await
            .unwrap()
            .messages[0]
            .id,
        receipt.message_id
    );
    assert!(matches!(
        app.request_status("send").await.unwrap(),
        RequestState::Accepted(_)
    ));
    assert!(matches!(
        app.request_status("not-found").await.unwrap(),
        RequestState::NotFound
    ));
    let cursor = app.inbox(None, 100, 0).await.unwrap().cursor;
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    let trigger = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        signal.cancel();
    });
    assert!(matches!(
        app.request_cancelled(
            Operation::Inbox {
                cursor: cursor.clone(),
                limit: 100,
                wait_ms: 5000
            },
            &cancel
        )
        .await,
        Err(ClientError::Cancelled)
    ));
    trigger.await.unwrap();
    assert!(matches!(app.health().await, Err(ClientError::Transport)));
    app.close().await;
    let mut app = ConnectClient::open(config.clone()).await.unwrap();
    assert!(app.inbox(cursor, 100, 0).await.unwrap().events.is_empty());
    assert_eq!(
        receipt,
        app.send("send", &conversation.id, "hello").await.unwrap()
    );
    assert!(
        app.close_conversation("close", &conversation.id)
            .await
            .unwrap()
            .closed
    );
    assert!(
        matches!(app.send("closed-send",&conversation.id,"new").await,Err(ClientError::Application(e)) if e.code=="conversation-closed")
    );
    assert_eq!(
        app.request_status("closed-send").await.unwrap(),
        RequestState::Rejected
    );
    assert_eq!(
        app.history(&conversation.id, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    app.close().await;
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn deadline_closes_peer_without_hidden_retries_or_new_logical_ids() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    let mut config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    config.timeout_ms = 100;
    let mut app = ConnectClient::open(config.clone()).await.unwrap();
    assert!(matches!(
        app.inbox(None, 100, 5000).await,
        Err(ClientError::Timeout)
    ));
    assert!(matches!(app.health().await, Err(ClientError::Transport)));
    app.close().await;
    let mut app = ConnectClient::open(config).await.unwrap();
    assert_eq!(
        app.request_status("never-sent").await.unwrap(),
        RequestState::NotFound
    );
    app.close().await;
    server.shutdown().await.unwrap();
}
