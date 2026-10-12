#![allow(clippy::unwrap_used)] // Fixture failures intentionally panic.
use connect_client::{ClientConfig, ClientError, ConnectClient};
use connect_protocol::{Limits, Operation};
use connect_server::{
    cli::{credentials, init},
    config::ServerConfig,
    runtime::RunningServer,
};
use connect_store::{Scope, Store};

#[tokio::test]
async fn tls_rotation_is_explicit_and_old_trust_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("demo");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server_path = directory.join("server.toml");
    let operator_path = directory.join("operator.toml");
    let old = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let server = RunningServer::start(ServerConfig::load(&server_path).unwrap())
        .await
        .unwrap();
    assert!(credentials::rotate_tls(&server_path, &operator_path)
        .unwrap_err()
        .to_string()
        .contains("already in use"));
    server.shutdown().await.unwrap();
    credentials::rotate_tls(&server_path, &operator_path).unwrap();
    let config = ServerConfig::load(&server_path).unwrap();
    assert_ne!(
        std::fs::read(&old.trust_certificate).unwrap(),
        std::fs::read(&config.certificate).unwrap()
    );
    let server = RunningServer::start(config).await.unwrap();
    let mut old = old;
    old.endpoint = server.address;
    assert!(matches!(
        ConnectClient::open(old).await,
        Err(ClientError::Transport)
    ));
    let mut current = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    current.endpoint = server.address;
    let mut client = ConnectClient::open(current).await.unwrap();
    client.request(Operation::Health {}).await.unwrap();
    client.close().await;
    server.shutdown().await.unwrap();
}
#[tokio::test]
async fn expired_tls_invalid_config_and_occupied_port_do_not_start() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("demo");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
    params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    params.not_after = rcgen::date_time_ymd(2001, 1, 1);
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let expired = temp.path().join("expired.der");
    let expired_key = temp.path().join("expired-key.der");
    std::fs::write(&expired, cert.der()).unwrap();
    std::fs::write(&expired_key, key.serialize_der()).unwrap();
    config.certificate = expired;
    config.private_key = expired_key;
    assert!(RunningServer::start(config)
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("expired"));
    let mut config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let occupied = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    config.bind = occupied.local_addr().unwrap();
    assert!(
        format!("{:#}", RunningServer::start(config).await.err().unwrap())
            .contains("Cannot bind QUIC listener")
    );
    // Failed startup released ownership; another valid start is possible.
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    server.shutdown().await.unwrap();
}
#[test]
fn minting_checks_current_operator_authority_inside_transaction() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::initialize(&temp.path().join("state.sqlite3"), Limits::default()).unwrap();
    let op = store.create_identity("operator").unwrap();
    let app = store.create_identity("app").unwrap();
    let expiry = chrono::Utc::now().timestamp() + 60;
    let operator = store.issue(&op.id, Scope::operator(), expiry).unwrap();
    let authority = store.authenticate(&operator.token).unwrap();
    let app_key = store
        .issue_as(&authority, &app.id, Scope::messaging(vec![]), expiry)
        .unwrap();
    let restricted = store.authenticate(&app_key.token).unwrap();
    assert_eq!(
        store
            .issue_as(&restricted, &app.id, Scope::operator(), expiry)
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    store.revoke(&authority, &operator.id).unwrap();
    assert_eq!(
        store
            .issue_as(&authority, &app.id, Scope::operator(), expiry)
            .err()
            .unwrap()
            .code,
        "unauthorized"
    );
}
#[tokio::test]
async fn ipv6_loopback_native_identity_path() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("demo");
    init::initialize(&directory, true, "[::1]:0".parse().unwrap()).unwrap();
    let server = RunningServer::start(ServerConfig::load(&directory.join("server.toml")).unwrap())
        .await
        .unwrap();
    let mut config = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    config.endpoint = server.address;
    let mut client = ConnectClient::open(config).await.unwrap();
    client.resolve("connect:agent@local").await.unwrap();
    client.close().await;
    server.shutdown().await.unwrap();
}
