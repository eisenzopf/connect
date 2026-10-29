//! Typed control workflow on real WT and bounded WSS fallback.
use super::*;
use rcgen::PublicKeyData;
use rustls::pki_types::{pem::PemObject, CertificateDer};
use sha2::{Digest, Sha256};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit owned Chrome and built production browser SDK"]
async fn installed_chrome_typed_sessions_invitations_and_operation_recovery() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let h = Hosted::with_media_origins(&[origin.clone()], true).await;
    let before = counts(&h);
    let page = enrollment_browser::page(listener, &h, origin.clone()).await;
    let key = rcgen::KeyPair::from_pem(&std::fs::read_to_string(&h.f.config.private_key).unwrap())
        .unwrap();
    let spki = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(key.subject_public_key_info()));
    let certificate = CertificateDer::from_pem_file(&h.f.config.certificate).unwrap();
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(certificate.as_ref()));
    let blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let profile = h.f.temp.path().join("owned-typed-browser");
    std::fs::create_dir(&profile).unwrap();
    let mut chrome = enrollment_browser::launch_capture(
        &profile,
        &h.f.temp.path().join("typed-chrome.log"),
        &spki,
        true,
    );
    let port = enrollment_browser::debug_port(&mut chrome, &profile).await;
    let c = &h.f.client;
    let config = json!({"actors":[],"suffix":uuid::Uuid::new_v4().simple().to_string(),"identity_id":c.identity_id,"recipient_id":h.f.peer.identity_id,"api_origin":h.url(HttpSurface::Application,""),"wt_url":h.f.server.webtransport_url,"wss_url":h.f.server.websocket_url,"unreachable_wt_url":format!("https://{}/connect",blackhole.local_addr().unwrap()),"certificate_sha256":hash,"server_keyid":c.server_key_id,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()});
    let result = enrollment_browser::run(
        &h,
        port,
        &format!("{origin}/typed-browser"),
        config,
        "typed-browser",
    )
    .await;
    assert_eq!(result["result"]["direct_wt"], 1);
    assert_eq!(result["result"]["bounded_wss_fallback"], 1);
    for name in [
        "incoming_rejections",
        "incoming_accepts",
        "explicit_retry",
        "cancellations",
        "recovered_original",
        "current_authority_refusal",
        "owned_device_permission",
        "permission_denial_ui",
        "keyboard_capture_retry",
        "device_source_choice",
        "operation_progress_ui",
        "owned_tab_capture",
        "owned_tab_capture_retired",
        "screen_kind_observed",
        "screen_recovery_without_custody",
    ] {
        assert_eq!(result["result"][name], 2, "{name}");
    }
    for name in [
        "wt_native_connection",
        "wt_owned_capture_retired",
        "wss_media_connection",
        "wss_owned_capture_retired",
        "wss_owned_renderer_retired",
        "wt_screen_connection",
        "wss_screen_connection",
    ] {
        assert_eq!(result["result"][name], 1, "{name}");
    }
    assert_eq!(result["result"]["restored_media_observation_only"], 4);
    assert_eq!(result["result"]["original_mutations"], 24);
    assert_eq!(
        counts(&h),
        (before.0 + 4, before.1 + 2, before.2 + 4, before.3 + 4)
    );
    drop(chrome);
    drop(blackhole);
    page.stop.cancel();
    page.task.await.unwrap();
    h.f.server.shutdown().await.unwrap();
}
