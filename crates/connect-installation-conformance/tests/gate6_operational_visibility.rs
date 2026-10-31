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
                if usage(&snapshot(&mut admin).await, ResourcePoolKind::NativeQuic).used == 0 {
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
