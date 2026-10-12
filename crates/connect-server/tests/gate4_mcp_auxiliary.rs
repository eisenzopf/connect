//! Task endpoint teardown cannot overwrite the primary MCP readiness.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
use connect_mcp::{LocalGrantAuth, RunningMcp};
use connect_protocol::ReadinessState;
use std::sync::Arc;

#[tokio::test]
async fn auxiliary_listener_shutdown_preserves_primary_listener_readiness() {
    let f = fixture::Fixture::start_independent().await;
    let primary = f.server.mcp_address.unwrap();
    let before = f
        .server
        .service
        .readiness
        .snapshot()
        .components
        .into_iter()
        .find(|c| c.component == "mcp")
        .unwrap();
    assert_eq!(before.state, ReadinessState::Ready);
    let mut endpoints = Vec::new();
    for _ in 0..2 {
        endpoints.push(
            RunningMcp::start_local_auxiliary(
                f.server.service.clone(),
                Arc::new(LocalGrantAuth::new(f.server.service.clone())),
                "127.0.0.1:0".parse().unwrap(),
            )
            .await
            .unwrap(),
        );
    }
    for endpoint in endpoints {
        let address = endpoint.address;
        endpoint.shutdown().await.unwrap();
        assert!(tokio::net::TcpStream::connect(address).await.is_err());
        assert_eq!(
            f.server
                .service
                .readiness
                .snapshot()
                .components
                .into_iter()
                .find(|c| c.component == "mcp")
                .unwrap(),
            before
        );
        assert!(tokio::net::TcpStream::connect(primary).await.is_ok());
    }
    f.close().await;
}
