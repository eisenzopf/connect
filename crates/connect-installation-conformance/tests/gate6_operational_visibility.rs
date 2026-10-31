#![recursion_limit = "256"]
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
//! Real signed peer and application HTTP diagnostics use public client APIs.
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::{CanonicalClient, CanonicalClientConfig},
    ClientError,
};
use connect_protocol::{OperationalHealth, Outcome, ResourcePoolKind, ResourceUsage};
use connect_server::{
    canonical::{CanonicalServerConfig, RunningCanonicalServer},
    cli::canonical_init,
};
use std::time::Duration;

async fn snapshot(client: &mut ApplicationClient) -> Box<OperationalHealth> {
    let outcome = client.health().await.unwrap();
    let Outcome::Health {
        operational: Some(value),
        ..
    } = outcome
    else {
        panic!("Expected administrator operational health")
    };
    value
}
fn usage(snapshot: &OperationalHealth, kind: ResourcePoolKind) -> &ResourceUsage {
    snapshot
        .resource_pools
        .as_ref()
        .unwrap()
        .iter()
        .find(|pool| pool.kind == kind)
        .unwrap()
        .usage
        .as_ref()
        .unwrap()
}
#[tokio::test]
async fn public_health_observes_real_http_and_signed_peer_lifetimes_without_secrets() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        canonical_init::initialize_with_http(
            &root,
            true,
            "127.0.0.1:0".parse().unwrap(),
            Some(port),
        )
        .unwrap();
        let server = RunningCanonicalServer::start(
            CanonicalServerConfig::load(&root.join("server.toml")).unwrap(),
        )
        .await
        .unwrap();
        let admin_config =
            ApplicationClientConfig::load(&root.join("operator.application.toml")).unwrap();
        let mut admin = ApplicationClient::open(admin_config.clone()).await.unwrap();
        let mut participant = ApplicationClient::open(
            ApplicationClientConfig::load(&root.join("reviews.application.toml")).unwrap(),
        )
        .await
        .unwrap();
        let ordinary = participant.health().await.unwrap();
        assert!(matches!(
            ordinary,
            Outcome::Health {
                operational: None,
                ..
            }
        ));
        let initial = snapshot(&mut admin).await;
        assert_eq!(usage(&initial, ResourcePoolKind::NativeQuic).used, 0);
        assert_eq!(usage(&initial, ResourcePoolKind::QuicIngress).used, 0);
        assert!(usage(&initial, ResourcePoolKind::HttpConnections).used >= 1);
        assert!(usage(&initial, ResourcePoolKind::HttpRequests).used >= 1);
        assert!(usage(&initial, ResourcePoolKind::HttpRequests).accepted >= 2);
        let mut config = CanonicalClientConfig::load(&root.join("reviews.toml")).unwrap();
        config.endpoint = server.address;
        let peer_secret = config.credential.clone();
        let mut peer = CanonicalClient::open(config).await.unwrap();
        peer.list_conversations(Default::default()).await.unwrap();
        let active = snapshot(&mut admin).await;
        assert_eq!(usage(&active, ResourcePoolKind::NativeQuic).used, 1);
        assert_eq!(usage(&active, ResourcePoolKind::QuicIngress).used, 1);
        assert!(usage(&active, ResourcePoolKind::NativeQuic).accepted >= 1);
        assert!(active.runtime.as_ref().unwrap().physical_owners >= 1);
        let encoded = serde_json::to_string(&active).unwrap();
        assert!(!encoded.contains(&peer_secret));
        assert!(!encoded.contains(&admin_config.credential));
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("intent_json"));
        peer.shutdown().await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let current = snapshot(&mut admin).await;
                if usage(&current, ResourcePoolKind::NativeQuic).used == 0
                    && usage(&current, ResourcePoolKind::QuicIngress).used == 0
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let actor = server.store.authenticate(&admin_config.credential).unwrap();
        server.store.revoke(&actor, &actor.credential_id).unwrap();
        assert!(matches!(admin.health().await.unwrap_err(),
            ClientError::Application(ref error) if error.code == "unauthorized"));
        drop(admin);
        drop(participant);
        server.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn abandoned_http_header_expires_reclaims_capacity_and_joins_listener_drain() {
    use connect_protocol::Operation;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::time::timeout(Duration::from_secs(65), async {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        canonical_init::initialize_with_http(
            &root,
            false,
            "127.0.0.1:0".parse().unwrap(),
            Some(port),
        )
        .unwrap();
        let mut config = CanonicalServerConfig::load(&root.join("server.toml")).unwrap();
        config.limits.connections = 1;
        let server = RunningCanonicalServer::start(config).await.unwrap();
        let application =
            ApplicationClientConfig::load(&root.join("operator.application.toml")).unwrap();
        let actor = server.store.authenticate(&application.credential).unwrap();
        let read = || async {
            let Outcome::Health {
                operational: Some(value),
                ..
            } = server
                .service
                .execute(actor.credential_id.clone(), Operation::Health {})
                .await
                .unwrap()
            else {
                panic!("Expected operator snapshot")
            };
            value
        };
        let mut abandoned = tokio::net::TcpStream::connect(port).await.unwrap();
        abandoned.write_all(b"G").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if usage(&read().await, ResourcePoolKind::HttpConnections).used == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut refused = tokio::net::TcpStream::connect(port).await.unwrap();
        let mut byte = [0u8; 1];
        let result = tokio::time::timeout(Duration::from_secs(3), refused.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
        assert_eq!(
            usage(&read().await, ResourcePoolKind::HttpConnections).rejected,
            1
        );
        let result = tokio::time::timeout(Duration::from_secs(35), abandoned.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if usage(&read().await, ResourcePoolKind::HttpConnections).used == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut client = ApplicationClient::open(application).await.unwrap();
        assert_eq!(
            usage(
                &snapshot(&mut client).await,
                ResourcePoolKind::HttpConnections
            )
            .limit,
            1
        );
        drop(client);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if usage(&read().await, ResourcePoolKind::HttpConnections).used == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let mut draining = tokio::net::TcpStream::connect(port).await.unwrap();
        draining.write_all(b"G").await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if usage(&read().await, ResourcePoolKind::HttpConnections).used == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(6), server.shutdown())
            .await
            .unwrap()
            .unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), draining.read(&mut byte))
            .await
            .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn trickling_http_headers_cannot_extend_the_absolute_connection_lifetime() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::time::timeout(Duration::from_secs(130), async {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("state");
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        canonical_init::initialize_with_http(
            &root,
            false,
            "127.0.0.1:0".parse().unwrap(),
            Some(port),
        )
        .unwrap();
        let mut config = CanonicalServerConfig::load(&root.join("server.toml")).unwrap();
        config.limits.connections = 1;
        let server = RunningCanonicalServer::start(config).await.unwrap();
        let mut socket = tokio::net::TcpStream::connect(port).await.unwrap();
        socket
            .write_all(b"GET /healthz HTTP/1.1\r\nX-Slow: ")
            .await
            .unwrap();
        let (mut input, mut output) = socket.into_split();
        let start = tokio::time::Instant::now();
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        let mut byte = [0u8; 1];
        loop {
            tokio::select! {
                _result = input.read(&mut byte) => {
                    // No complete request has been submitted. A server timeout
                    // response or EOF/reset all retire the incomplete exchange.
                    break;
                }
                _ = tick.tick() => {
                    if output.write_all(b"x").await.is_err() { break; }
                }
            }
        }
        assert!(start.elapsed() <= Duration::from_secs(125));
        drop(input);
        drop(output);
        let mut admin = ApplicationClient::open(
            ApplicationClientConfig::load(&root.join("operator.application.toml")).unwrap(),
        )
        .await
        .unwrap();
        // A fresh complete authenticated request can use the retired slot.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match admin.health().await {
                    Ok(Outcome::Health {
                        operational: Some(_),
                        ..
                    }) => break,
                    _ => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
        })
        .await
        .unwrap();
        drop(admin);
        server.shutdown().await.unwrap();
    })
    .await
    .unwrap();
}
