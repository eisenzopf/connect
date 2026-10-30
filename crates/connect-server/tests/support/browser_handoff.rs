//! Actual owned browser handoff from an independently prepared synthetic target.
use super::*;
use rcgen::PublicKeyData;
use rustls::pki_types::{pem::PemObject, CertificateDer};
use sha2::{Digest, Sha256};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit owned Chrome and built production browser SDK"]
async fn installed_chrome_prepared_handoff_switches_decoded_audio_on_wt_and_wss() {
    run_handoff(false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit owned Chrome and built production browser SDK"]
async fn installed_chrome_fallback_decodes_public_audio() {
    run_handoff(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit owned Chrome and built production browser SDK"]
async fn installed_chrome_prepared_subscription_refusals_preserve_public_audio() {
    run_handoff(false, true).await;
}
async fn run_handoff(audio_probe: bool, privacy_probe: bool) {
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
    let config = json!({"wss_audio_probe":audio_probe,"privacy_probe":privacy_probe,"actors":[],"suffix":uuid::Uuid::new_v4().simple().to_string(),"identity_id":c.identity_id,"recipient_id":h.f.peer.identity_id,"handoff_identity":target,"api_origin":h.url(HttpSurface::Application,""),"wt_url":h.f.server.webtransport_url,"wss_url":h.f.server.websocket_url,"unreachable_wt_url":format!("https://{}/connect",blackhole.local_addr().unwrap()),"certificate_sha256":hash,"server_keyid":c.server_key_id,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()});
    let result = enrollment_browser::run(
        &h,
        port,
        &format!("{origin}/handoff-browser"),
        config,
        if audio_probe {
            "audio-browser"
        } else if privacy_probe {
            "privacy-browser"
        } else {
            "handoff-browser"
        },
    )
    .await;
    if audio_probe {
        assert_eq!(result["result"]["reports"].as_array().unwrap().len(), 1);
        assert_eq!(result["result"]["original_mutations"], 9);
        let db = rusqlite::Connection::open(&h.f.config.database).unwrap();
        let count = |sql| db.query_row(sql, [], |r| r.get::<_, u64>(0)).unwrap();
        assert_eq!(
            count("SELECT COUNT(*) FROM operations WHERE command_type='session.handoff'"),
            0
        );
        assert_eq!(count("SELECT COUNT(*) FROM operations WHERE command_type='connection.offer' AND state='completed'"),2);
    } else if privacy_probe {
        assert_eq!(result["result"]["reports"].as_array().unwrap().len(), 2);
        assert_eq!(result["result"]["original_mutations"], 24);
        let db = rusqlite::Connection::open(&h.f.config.database).unwrap();
        let count = |sql| db.query_row(sql, [], |r| r.get::<_, u64>(0)).unwrap();
        assert_eq!(
            count("SELECT COUNT(*) FROM operations WHERE command_type='session.handoff'"),
            0
        );
        assert_eq!(count("SELECT COUNT(*) FROM operations WHERE command_type='connection.offer' AND state='completed'"), 6);
        assert_eq!(count("SELECT COUNT(*) FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id WHERE (o.operation_id LIKE 'handoff-public-to-prepared-%' OR o.operation_id LIKE 'handoff-prepared-to-public-%') AND o.state='failed' AND json_extract(o.problem_json,'$.code')='forbidden' AND n.effect_started=0 AND n.effect_id IS NULL AND n.attempt_id IS NULL"), 4);
    } else {
        assert_eq!(result["result"]["reports"].as_array().unwrap().len(), 2);
        assert_eq!(result["result"]["original_mutations"], 28);
        {
            let db = rusqlite::Connection::open(&h.f.config.database).unwrap();
            let count = |sql| db.query_row(sql, [], |r| r.get::<_, u64>(0)).unwrap();
            assert_eq!(
                count("SELECT COUNT(*) FROM operations WHERE command_type='session.handoff'"),
                4
            );
            assert_eq!(count("SELECT COUNT(*) FROM operations WHERE command_type='session.handoff' AND state='completed'"), 2);
            assert_eq!(
            count(
                "SELECT COUNT(*) FROM native_handoffs WHERE started=1 AND applied_json IS NOT NULL"
            ),
            2
        );
            assert_eq!(count("SELECT COUNT(*) FROM operation_effects e JOIN operations o ON o.record_id=e.operation_record_id WHERE o.command_type='session.handoff' AND e.state='confirmed' AND e.attempt=1"), 2);
            assert_eq!(count("SELECT COUNT(*) FROM connection_offers WHERE json_extract(offer_json,'$.handoff_preparation.profile')='connect-handoff-preparation-v1'"), 2);
            assert_eq!(count("SELECT COUNT(*) FROM native_consultation_legs"), 0);
            assert_eq!(count("SELECT COUNT(*) FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id WHERE (o.operation_id LIKE 'handoff-public-to-prepared-%' OR o.operation_id LIKE 'handoff-prepared-to-public-%') AND o.state='failed' AND json_extract(o.problem_json,'$.code')='forbidden' AND n.effect_started=0 AND n.effect_id IS NULL AND n.attempt_id IS NULL"),4);
            assert_eq!(
                count("SELECT COUNT(*) FROM operations WHERE command_type='connection.offer'"),
                6
            );
            assert_eq!(count("SELECT COUNT(*) FROM operations o WHERE o.command_type='session.handoff' AND o.state='failed' AND json_extract(o.problem_json,'$.code')='forbidden' AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.operation_record_id=o.record_id) AND NOT EXISTS(SELECT 1 FROM native_handoffs n WHERE n.operation_record_id=o.record_id)"),2);
        }
    }
    drop(chrome);
    drop(blackhole);
    page.stop.cancel();
    page.task.await.unwrap();
    h.f.server.shutdown().await.unwrap();
}
