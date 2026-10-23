//! Installed listener failure and drop cleanup, using real sockets and Store keys.
use super::*;
use connect_server::canonical_telephone::{CanonicalTelephoneConfig, CanonicalTelephoneRoute};

fn listener_config(credential_id: String) -> CanonicalTelephoneConfig {
    CanonicalTelephoneConfig {
        bind: std::net::UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap(),
        media_port_start: 20100,
        media_port_end: 20163,
        outbound: None,
        initial: vec![],
        routes: vec![CanonicalTelephoneRoute {
            credential_id,
            policy: TelephoneReplacementPolicy {
                route_reference: "config_test_callback".into(),
                sip_authority: "127.0.0.1:5099".into(),
                confirmation_digits: "55#".into(),
                timeout_seconds: 30,
            },
        }],
    }
}
fn route_key(f: &Fixture) -> String {
    let actor = f.server.store.authenticate(&f.admin).unwrap();
    f.server
        .store
        .issue(
            &actor.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .id
}
async fn released(address: std::net::SocketAddr) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let (Ok(_udp), Ok(_tcp)) = (
                std::net::UdpSocket::bind(address),
                std::net::TcpListener::bind(address),
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("owned SIP sockets released after failed startup or drop");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_unknown_account_refuses_before_binding_and_releases_installation(
) {
    let f = Fixture::new("EdDSA", 12).await;
    let mut config = f.config.clone();
    let telephone = listener_config("missing-route-account".into());
    let address = telephone.bind;
    config.telephone = Some(telephone);
    f.server.shutdown().await.unwrap();
    assert!(RunningCanonicalServer::start(config.clone()).await.is_err());
    assert_sip_listener_released(address);
    config.telephone = None;
    RunningCanonicalServer::start(config)
        .await
        .unwrap()
        .shutdown()
        .await
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_later_http_bind_failure_drains_started_sip() {
    let f = Fixture::new("EdDSA", 12).await;
    let mut config = f.config.clone();
    let telephone = listener_config(route_key(&f));
    let address = telephone.bind;
    config.telephone = Some(telephone);
    f.server.shutdown().await.unwrap();
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    config.http_bind = Some(occupied.local_addr().unwrap());
    assert!(RunningCanonicalServer::start(config).await.is_err());
    released(address).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_server_drop_fences_owner_and_drains_sip() {
    let f = Fixture::new("EdDSA", 12).await;
    let mut config = f.config.clone();
    let telephone = listener_config(route_key(&f));
    let address = telephone.bind;
    config.telephone = Some(telephone);
    f.server.shutdown().await.unwrap();
    let server = RunningCanonicalServer::start(config).await.unwrap();
    assert!(std::net::UdpSocket::bind(address).is_err());
    assert!(std::net::TcpListener::bind(address).is_err());
    drop(server);
    released(address).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_voicemail_audio_without_confirmation_times_out_without_movement(
) {
    telephone_case(Stop::Voicemail, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_rejected_digits_time_out_without_movement() {
    telephone_case(Stop::RejectedConfirmation, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_telephone_configured_private_cancel_preserves_original_duplex() {
    telephone_case(Stop::PrivateCancel, true).await;
}
