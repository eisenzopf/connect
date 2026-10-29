//! Actual transport selection with distinct transport-bound browser credentials.
use super::*;
use rcgen::PublicKeyData;
use rustls::pki_types::{pem::PemObject, CertificateDer};
use sha2::{Digest, Sha256};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit owned Chrome and built production browser SDK"]
async fn installed_chrome_resolves_transport_bound_enrollment_for_each_contact() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let h = Hosted::with_origins(&[origin.clone()]).await;
    let before = counts(&h);
    let page = enrollment_browser::page(listener, &h, origin.clone()).await;
    let key = rcgen::KeyPair::from_pem(&std::fs::read_to_string(&h.f.config.private_key).unwrap())
        .unwrap();
    let spki = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(key.subject_public_key_info()));
    let certificate = CertificateDer::from_pem_file(&h.f.config.certificate).unwrap();
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(certificate.as_ref()));
    let blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let profile = h.f.temp.path().join("owned-profile-fallback");
    std::fs::create_dir(&profile).unwrap();
    let mut chrome =
        enrollment_browser::launch(&profile, &h.f.temp.path().join("profile-chrome.log"), &spki);
    let port = enrollment_browser::debug_port(&mut chrome, &profile).await;
    let c = &h.f.client;
    let config = json!({"actors":[],"suffix":uuid::Uuid::new_v4().simple().to_string(),"identity_id":c.identity_id,"recipient_id":h.f.peer.identity_id,"api_origin":h.url(HttpSurface::Application,""),"wt_url":h.f.server.webtransport_url,"wss_url":h.f.server.websocket_url,"unreachable_wt_url":format!("https://{}/connect",blackhole.local_addr().unwrap()),"certificate_sha256":hash,"server_keyid":c.server_key_id,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()});
    let result = enrollment_browser::run(
        &h,
        port,
        &format!("{origin}/profile-fallback"),
        config,
        "profile-fallback",
    )
    .await;
    for name in [
        "direct_wt",
        "bounded_wss_fallback",
        "explicit_wss_reconnect",
        "reused_preferred_credential_refusal",
        "wrong_profile_authentication_refusal",
        "revoked_profile_refusal",
        "revoked_cached_peer_refusal",
    ] {
        assert_eq!(result["result"][name], 1, "{name}");
    }
    assert_eq!(
        counts(&h),
        (before.0 + 2, before.1 + 1, before.2 + 2, before.3 + 2)
    );
    drop(chrome);
    drop(blackhole);
    page.stop.cancel();
    page.task.await.unwrap();
    h.f.server.shutdown().await.unwrap();
}
