use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::Operation;
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Store;
use std::{path::Path, sync::Arc};

fn client_config(directory: &Path, name: &str, address: std::net::SocketAddr) -> ClientConfig {
    let mut config = ClientConfig::load(&directory.join(format!("{name}.toml"))).unwrap();
    config.endpoint = address;
    config
}
#[tokio::test]
async fn native_identity_restart_tls_scope_and_active_revocation() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("demo");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let original = std::fs::read(directory.join("agent.toml")).unwrap();
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    assert_eq!(
        std::fs::read(directory.join("agent.toml")).unwrap(),
        original
    );
    let cfg = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let server = RunningServer::start(cfg.clone()).await.unwrap();
    let installation = server.store.installation_id().unwrap();
    let mut app = ConnectClient::open(client_config(&directory, "reviews", server.address))
        .await
        .unwrap();
    let agent = app.resolve("connect:agent@local").await.unwrap();
    assert!(
        matches!(app.resolve("connect:operator@local").await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    let mut bad = client_config(&directory, "reviews", server.address);
    bad.server_name = "not-localhost.example".into();
    assert!(matches!(
        ConnectClient::open(bad).await,
        Err(ClientError::Transport)
    ));
    let different_cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let wrong = temp.path().join("wrong.der");
    std::fs::write(&wrong, different_cert.cert.der()).unwrap();
    let mut bad = client_config(&directory, "reviews", server.address);
    bad.trust_certificate = wrong;
    assert!(matches!(
        ConnectClient::open(bad).await,
        Err(ClientError::Transport)
    ));
    app.close().await;
    server.shutdown().await.unwrap();
    let server = RunningServer::start(cfg).await.unwrap();
    assert_eq!(server.store.installation_id().unwrap(), installation);
    let config = client_config(&directory, "reviews", server.address);
    let mut app = ConnectClient::open(config.clone()).await.unwrap();
    assert_eq!(app.resolve("connect:agent@local").await.unwrap(), agent);
    let operator = client_config(&directory, "operator", server.address);
    let authority = server.store.authenticate(&operator.credential).unwrap();
    server
        .store
        .revoke(&authority, &config.credential_id)
        .unwrap();
    assert!(
        matches!(app.request(Operation::Health {}).await,Err(ClientError::Application(e)) if e.code=="unauthorized")
    );
    assert!(matches!(
        ConnectClient::open(config).await,
        Err(ClientError::Authentication)
    ));
    app.close().await;
    server.shutdown().await.unwrap();
    let store = Arc::new(
        Store::open(
            &directory.join("state.sqlite3"),
            connect_protocol::Limits::default(),
        )
        .unwrap(),
    );
    store.integrity_check().unwrap();
}
#[test]
fn local_config_rejects_public_listeners_and_unbounded_limits() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("demo");
    init::initialize(&directory, false, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    config.bind = "0.0.0.0:4433".parse().unwrap();
    assert!(config.validate().is_err());
    config.bind = "127.0.0.1:4433".parse().unwrap();
    config.limits.connections = 0;
    assert!(config.validate().is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for file in ["operator.toml", "private-key.der", "state.sqlite3"] {
            assert_eq!(
                std::fs::metadata(directory.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
