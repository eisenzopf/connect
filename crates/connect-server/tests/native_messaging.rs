#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{Operation, Outcome};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};

#[tokio::test]
async fn two_native_clients_recover_messages_and_receive_wait_boundary_insert() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("demo");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    let config = |name: &str| {
        let mut c = ClientConfig::load(&directory.join(format!("{name}.toml"))).unwrap();
        c.endpoint = server.address;
        c
    };
    let mut app = ConnectClient::open(config("reviews")).await.unwrap();
    let mut agent = ConnectClient::open(config("agent")).await.unwrap();
    let peer = app.resolve("connect:agent@local").await.unwrap();
    let id = match app
        .request(Operation::Create {
            request_id: "create-1".into(),
            participants: vec![peer.id],
        })
        .await
        .unwrap()
    {
        Outcome::Conversation(c) => c.id,
        _ => panic!("conversation"),
    };
    let cursor = match agent
        .request(Operation::Inbox {
            cursor: None,
            limit: 100,
            wait_ms: 0,
        })
        .await
        .unwrap()
    {
        Outcome::Inbox { cursor, .. } => cursor,
        _ => panic!("inbox"),
    };
    let send = Operation::Send {
        request_id: "send-1".into(),
        conversation_id: id.clone(),
        body: "hello".into(),
    };
    let wait = agent.request(Operation::Inbox {
        cursor,
        limit: 100,
        wait_ms: 2000,
    });
    let sending = app.request(send.clone());
    let (events, receipt) = tokio::join!(wait, sending);
    let receipt = receipt.unwrap();
    assert!(
        matches!(events.unwrap(),Outcome::Inbox {events,..} if events.len()==1 && events[0].message.as_ref().unwrap().body=="hello")
    );
    assert_eq!(receipt, app.request(send.clone()).await.unwrap());
    let changed = Operation::Send {
        request_id: "send-1".into(),
        conversation_id: id.clone(),
        body: "changed".into(),
    };
    assert!(
        matches!(app.request(changed).await,Err(ClientError::Application(e)) if e.code=="conflict")
    );
    app.close().await;
    agent.close().await;
    server.shutdown().await.unwrap();
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    let mut c = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    c.endpoint = server.address;
    let mut app = ConnectClient::open(c).await.unwrap();
    assert_eq!(receipt, app.request(send).await.unwrap());
    assert!(
        matches!(app.request(Operation::History {conversation_id:id,after:None,limit:100}).await.unwrap(),Outcome::History {messages,..} if messages.len()==1)
    );
    app.close().await;
    server.shutdown().await.unwrap();
}
