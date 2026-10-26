//! Capability support never negotiates, grants or claims actual media readiness.
use super::*;
use crate::sdk::{self, features, Client, RequestOptions, RuntimeRole, Transport};
fn options() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(2)).unwrap()
}
async fn control(client: &Client) {
    let value = json!({"v":2,"id":"caller-attempt","ts":"2026-01-01T00:00:00Z","type":"operation.get","cid":"conv_one","payload":{"operation_id":"original-send","vendor":{"private":"payload-secret"}}});
    client
        .raw()
        .request(client.raw().validate(value).unwrap(), options())
        .await
        .unwrap();
}
async fn finish(client: &Client, host: tokio::task::JoinHandle<()>) {
    client.close(options()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), host)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn capabilities_describe_actual_owned_role_without_wire_effects_or_media_grants() {
    let (peer, host, _) = controlled(Fault::SdkRaw).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let cap = client.capabilities();
    let snapshot = cap.refresh(options()).await.unwrap();
    assert_eq!(snapshot.profile, "connect/2");
    assert_eq!(snapshot.runtime_role, RuntimeRole::Rust);
    assert_eq!(snapshot.selected_transport, Transport::Quic);
    assert!(snapshot.fallback_reason.is_none());
    assert_eq!(snapshot.features.len(), 5);
    cap.require(features::NATIVE_PROGRAMMATIC_MEDIA, options())
        .await
        .unwrap();
    assert!(snapshot.features.windows(2).all(|w| w[0] < w[1]));
    assert!(client.media_stats().is_ok());
    for name in [
        features::MESSAGE_RECEIPTS,
        "desktop-capture",
        "provider-delivered",
        "decoded-media-ready",
        "unknown-feature",
    ] {
        let e = cap.require(name, options()).await.unwrap_err();
        assert_eq!(
            (e.kind, e.certainty),
            (sdk::ErrorKind::Unsupported, sdk::Certainty::ReadOnly)
        );
        assert!(e.operation.is_none());
    }
    control(&client).await;
    finish(&client, host).await;
    assert_eq!(
        cap.refresh(options()).await.unwrap_err().kind,
        sdk::ErrorKind::Cancelled
    );
}
#[tokio::test]
async fn capability_wait_options_and_invalid_names_never_consume_control_capacity() {
    let (peer, host, _) = controlled(Fault::SdkRaw).await;
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    let cap = client.capabilities();
    let cancelled = options();
    cancelled.signal.cancel();
    assert_eq!(
        cap.refresh(cancelled).await.unwrap_err().kind,
        sdk::ErrorKind::Cancelled
    );
    let e = cap
        .refresh(RequestOptions {
            timeout: Duration::ZERO,
            ..options()
        })
        .await
        .unwrap_err();
    assert_eq!(
        (e.kind, e.certainty),
        (sdk::ErrorKind::Configuration, sdk::Certainty::ReadOnly)
    );
    for name in ["", "private invalid feature", "CAPTURE"] {
        assert_eq!(
            cap.require(name, options()).await.unwrap_err().kind,
            sdk::ErrorKind::Validation
        );
    }
    control(&client).await;
    finish(&client, host).await;
}
#[tokio::test]
async fn expired_authenticated_profile_cannot_return_a_live_capability_snapshot() {
    let (mut peer, host, _) = controlled(Fault::SdkRaw).await;
    peer.authenticated_until = chrono::Utc::now() + chrono::Duration::milliseconds(100);
    let client = Client::from_authenticated_peer(peer, "tenant".parse().unwrap()).unwrap();
    control(&client).await;
    tokio::time::sleep(Duration::from_millis(120)).await;
    let e = client.capabilities().refresh(options()).await.unwrap_err();
    // The native owner may already have observed expiry and retired by this
    // local check. Both observations refuse; neither fabricates a live profile.
    assert!(matches!(
        e.kind,
        sdk::ErrorKind::Authentication | sdk::ErrorKind::Cancelled
    ));
    assert_eq!(e.certainty, sdk::Certainty::ReadOnly);
    finish(&client, host).await;
}
