//! Actual canonical QUIC certificate reload, separate from envelope signing,
//! physical reconnect, media reconnection or network path migration.
use super::*;

async fn certificate_witness(f: &Fixture, expected_leaf: Option<&[u8]>) -> Vec<u8> {
    use rustls::pki_types::{pem::PemObject, CertificateDer};
    let mut roots = rustls::RootCertStore::empty();
    for certificate in
        CertificateDer::pem_slice_iter(&std::fs::read(&f.client.trust_certificate).unwrap())
    {
        roots.add(certificate.unwrap()).unwrap();
    }
    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![rvoip_uctp::v2::RAW_QUIC_ALPN.to_vec()];
    tls.resumption = rustls::client::Resumption::disabled();
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap(),
    )));
    let connection = tokio::time::timeout(
        Duration::from_secs(3),
        endpoint.connect(f.server.address, NAME).unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let chain = connection
        .peer_identity()
        .unwrap()
        .downcast::<Vec<CertificateDer<'static>>>()
        .unwrap();
    let leaf = chain[0].to_vec();
    if let Some(expected) = expected_leaf {
        assert_eq!(leaf, expected);
    }
    // Certificate inspection deliberately sends no application command or
    // credential. All application/media checks below use the public client.
    connection.close(0u32.into(), b"certificate witness complete");
    endpoint.close(0u32.into(), b"certificate witness complete");
    tokio::time::timeout(Duration::from_secs(5), endpoint.wait_idle())
        .await
        .unwrap();
    leaf
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_native_certificate_reload_preserves_signed_media_and_last_good_identity() {
    for algorithm in ["EdDSA", "ES256"] {
        let (f, issuer) =
            configured_at_with_issuer(algorithm, true, "127.0.0.1:0".parse().unwrap()).await;
        let mut scene = codecs::setup_started_dtmf(f, true).await;
        {
            let (source, target) = scene.clients.split_at_mut(1);
            codecs::decoded_audio(&source[0], &mut target[0], scene.sh, 0, scene.th, 0, 100).await;
            codecs::decoded_audio(&target[0], &mut source[0], scene.th, 0, scene.sh, 0, 200).await;
        }
        let db = rusqlite::Connection::open_with_flags(
            &scene.f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let bindings = || {
            db.prepare("SELECT id,generation,observed_runtime_id,readiness_json FROM connections ORDER BY id")
                .unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,
                    r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?)))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        let before = bindings();
        assert_eq!(before.len(), 2);
        let old_leaf = certificate_witness(&scene.f, None).await;
        let mut parameters = rcgen::CertificateParams::new(vec![NAME.into()]).unwrap();
        parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = parameters.signed_by(&key, &issuer).unwrap();
        let new_leaf = certificate.der().to_vec();
        assert_ne!(old_leaf, new_leaf);
        std::fs::write(
            &scene.f.config.certificate,
            format!("{}{}", certificate.pem(), issuer.pem()),
        )
        .unwrap();
        std::fs::write(&scene.f.config.private_key, key.serialize_pem()).unwrap();
        let changed = scene.f.server.reload_tls_certificates().await.unwrap();
        assert!(changed.changed);
        assert_eq!(changed.revision, 2);
        certificate_witness(&scene.f, Some(&new_leaf)).await;
        let unchanged = scene.f.server.reload_tls_certificates().await.unwrap();
        assert!(!unchanged.changed);
        assert_eq!(unchanged.revision, 2);
        std::fs::write(&scene.f.config.private_key, b"incomplete key replacement").unwrap();
        assert!(scene.f.server.reload_tls_certificates().await.is_err());
        certificate_witness(&scene.f, Some(&new_leaf)).await;
        // Original authenticated native owners retain authority and media;
        // another full public signed opening uses the retained rotated chain.
        let mut opened = CanonicalClient::open(scene.f.client.clone()).await.unwrap();
        let id = opened.keepalive().await.unwrap();
        reply(&mut opened, &id, "ack").await;
        opened.shutdown().await.unwrap();
        for client in &mut scene.clients {
            let id = client.keepalive().await.unwrap();
            reply(client, &id, "ack").await;
        }
        {
            let (source, target) = scene.clients.split_at_mut(1);
            codecs::decoded_audio(&source[0], &mut target[0], scene.sh, 0, scene.th, 0, 300).await;
            codecs::decoded_audio(&target[0], &mut source[0], scene.th, 0, scene.sh, 0, 400).await;
        }
        assert_eq!(before, bindings());
        assert_eq!(
            db.query_row("SELECT count(*) FROM native_offer_operations", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            db.query_row("SELECT max(attempt) FROM operation_effects", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
        drop(db);
        for client in scene.clients {
            client.shutdown().await.unwrap();
        }
        scene.f.server.shutdown().await.unwrap();
        eprintln!(
            "native-certificate-rotation {}",
            json!({"algorithm":algorithm,
            "full_handshakes":3,"last_good_after_invalid_key":true,
            "decoded_directions_before_and_after":4,"original_media_bindings_preserved":true,
            "original_offer_rows":2,"maximum_effect_attempt":1,"joined":true})
        );
    }
}
