//! Owned server composition, cold configuration and failed-start recovery.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_protocol::tasks::{InvocationBinding, InvocationKind};
use connect_server::{
    cli::init,
    config::{McpConfig, ServerConfig},
    runtime::RunningServer,
};
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_server_owns_mcp_listener_and_drains_it_with_native_transport() {
    let f = fixture::Fixture::start_configured(true).await;
    let address = f.server.mcp_address.unwrap();
    assert!(f
        .server
        .service
        .readiness
        .snapshot()
        .components
        .iter()
        .any(|c| c.component == "mcp" && c.state == connect_protocol::ReadinessState::Ready));
    let http = tokio::task::spawn_blocking(|| {
        reqwest_mcp::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
    .await
    .unwrap();
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(format!("http://{address}/mcp"))
            .auth_header(f.issued.expose_secret()),
    );
    let client = rmcp::model::ClientConfig::default()
        .serve(transport)
        .await
        .unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("connect_task_status"))
        .await
        .unwrap();
    assert_eq!(result.structured_content.unwrap()["data"]["id"], f.task.id);
    client.cancel().await.unwrap();
    let readiness = f.server.service.readiness.clone();
    f.close().await;
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(readiness
        .snapshot()
        .components
        .iter()
        .any(|c| c.component == "mcp" && c.reason == "listener-stopped"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn occupied_mcp_port_fails_startup_and_releases_installation_for_retry() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = ServerConfig::load(&dir.join("server.toml")).unwrap();
    // A concrete UDP address detects a leaked QUIC listener after failed startup.
    let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    config.bind = udp.local_addr().unwrap();
    drop(udp);
    config.mcp = Some(McpConfig {
        bind: occupied.local_addr().unwrap(),
        binding: Some(binding()),
    });
    assert!(RunningServer::start(config.clone()).await.is_err());
    config.mcp = None;
    let server = tokio::time::timeout(Duration::from_secs(5), RunningServer::start(config))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(server.mcp_address, None);
    server.shutdown().await.unwrap();
}

#[test]
fn mcp_config_has_no_secret_and_rejects_provider_origins_and_wildcard_listeners() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("state");
    init::initialize(&dir, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let mut config = ServerConfig::load(&dir.join("server.toml")).unwrap();
    config.mcp = Some(McpConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        binding: Some(binding()),
    });
    config.validate().unwrap();
    let serialized = toml::to_string(&config).unwrap();
    let decoded: ServerConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(decoded.mcp.unwrap().binding, Some(binding()));
    let mut independent = config.clone();
    independent.mcp.as_mut().unwrap().binding = None;
    independent.validate().unwrap();
    let independent: ServerConfig =
        toml::from_str(&toml::to_string(&independent).unwrap()).unwrap();
    assert_eq!(independent.mcp.unwrap().binding, None);
    let bad = serialized.replace(
        "external_id = \"runtime-host\"",
        "external_id = \"runtime-host\"\ntoken = \"forged\"",
    );
    assert!(toml::from_str::<ServerConfig>(&bad).is_err());
    config.mcp.as_mut().unwrap().binding.as_mut().unwrap().kind = InvocationKind::ProviderCall;
    assert!(config.validate().is_err());
    config.mcp.as_mut().unwrap().binding = Some(binding());
    config.mcp.as_mut().unwrap().bind = "0.0.0.0:0".parse().unwrap();
    assert!(config.validate().is_err());
    config.mcp.as_mut().unwrap().bind = "127.0.0.1:0".parse().unwrap();
    config
        .mcp
        .as_mut()
        .unwrap()
        .binding
        .as_mut()
        .unwrap()
        .external_id = "forged binding".into();
    assert!(config.validate().is_err());
}

fn binding() -> InvocationBinding {
    InvocationBinding {
        kind: InvocationKind::HostSession,
        adapter: "connect-runtime-test".into(),
        external_id: "runtime-host".into(),
    }
}
