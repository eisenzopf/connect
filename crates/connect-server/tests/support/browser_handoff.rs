//! Actual owned browser handoff from an independently prepared synthetic target.
use super::*;
use rcgen::PublicKeyData;
use rustls::pki_types::{pem::PemObject, CertificateDer};
use sha2::{Digest, Sha256};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit owned Chrome and built production browser SDK"]
async fn installed_chrome_prepared_handoff_switches_wt_audio_and_refuses_unsupported_wss_preparation(
) {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .try_init();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let h = Hosted::with_rtc_origins(
        &[origin.clone()],
        Some((
            connect_server::canonical_webrtc::CanonicalWebRtcProfile::Opus,
            3,
        )),
    )
    .await;
    // Isolated host provisioning creates only the synthetic target Identity.
    // All room/session/offer/handoff behavior below uses public browser clients.
    let target =
        h.f.server
            .store
            .create_identity("owned-browser-handoff-target")
            .unwrap()
            .id;
    let page =
        enrollment_browser::page_with_handoff(listener, &h, origin.clone(), Some(target.clone()))
            .await;
    let key = rcgen::KeyPair::from_pem(&std::fs::read_to_string(&h.f.config.private_key).unwrap())
        .unwrap();
    let spki = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(key.subject_public_key_info()));
    let certificate = CertificateDer::from_pem_file(&h.f.config.certificate).unwrap();
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(certificate.as_ref()));
    let blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let profile = h.f.temp.path().join("owned-handoff-browser");
    std::fs::create_dir(&profile).unwrap();
    let mut chrome =
        enrollment_browser::launch(&profile, &h.f.temp.path().join("handoff-chrome.log"), &spki);
    let port = enrollment_browser::debug_port(&mut chrome, &profile).await;
    let c = &h.f.client;
    let config = json!({"actors":[],"suffix":uuid::Uuid::new_v4().simple().to_string(),"identity_id":c.identity_id,"recipient_id":h.f.peer.identity_id,"handoff_identity":target,"api_origin":h.url(HttpSurface::Application,""),"wt_url":h.f.server.webtransport_url,"wss_url":h.f.server.websocket_url,"unreachable_wt_url":format!("https://{}/connect",blackhole.local_addr().unwrap()),"certificate_sha256":hash,"server_keyid":c.server_key_id,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()});
    let result = enrollment_browser::run(
        &h,
        port,
        &format!("{origin}/handoff-browser"),
        config,
        "handoff-browser",
    )
    .await;
    assert_eq!(result["result"]["reports"].as_array().unwrap().len(), 2);
    assert_eq!(result["result"]["original_mutations"], 17);
    {
        let db = rusqlite::Connection::open(&h.f.config.database).unwrap();
        let count = |sql| db.query_row(sql, [], |r| r.get::<_, u64>(0)).unwrap();
        assert_eq!(
            count("SELECT COUNT(*) FROM operations WHERE command_type='session.handoff'"),
            2
        );
        assert_eq!(count("SELECT COUNT(*) FROM operations WHERE command_type='session.handoff' AND state='completed'"), 1);
        assert_eq!(
            count(
                "SELECT COUNT(*) FROM native_handoffs WHERE started=1 AND applied_json IS NOT NULL"
            ),
            1
        );
        assert_eq!(count("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.command_type='session.handoff' AND e.state='confirmed' AND e.attempt=1"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM connection_offers WHERE json_extract(offer_json,'$.handoff_preparation.profile')='connect-handoff-preparation-v1'"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM native_consultation_legs"), 0);
        assert_eq!(
            count("SELECT COUNT(*) FROM operations WHERE command_type='connection.offer'"),
            3
        );
        assert_eq!(count("SELECT COUNT(*) FROM operations o WHERE o.command_type='session.handoff' AND o.state='failed' AND json_extract(o.problem_json,'$.code')='forbidden' AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.operation_record_id=o.record_id) AND NOT EXISTS(SELECT 1 FROM native_handoffs n WHERE n.operation_record_id=o.record_id)"),1);
    }
    drop(chrome);
    drop(blackhole);
    page.stop.cancel();
    page.task.await.unwrap();
    h.f.server.shutdown().await.unwrap();
}
