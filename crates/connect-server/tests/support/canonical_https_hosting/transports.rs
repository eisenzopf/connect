//! Current owned WSS/HTTP3/TCP composition, normal TLS and explicit host policy.
use super::*;
use rustls::pki_types::{pem::PemObject, CertificateDer};
use std::sync::Arc;
const ORIGIN: &str = "https://ui.fixture.invalid";
fn tls(h: &Hosted, alpn: &[u8]) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in CertificateDer::pem_slice_iter(&std::fs::read(&h.tls.root).unwrap()) {
        roots.add(certificate.unwrap()).unwrap();
    }
    let mut tls = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![alpn.to_vec()];
    tls.resumption = rustls::client::Resumption::disabled();
    tls
}
async fn wss(h: &Hosted, expected: &[u8], origin: &str, accepted: bool) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let address = h.f.server.websocket_address.unwrap();
    let socket = tokio::net::TcpStream::connect(address).await.unwrap();
    let stream = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_rustls::TlsConnector::from(Arc::new(tls(h, b"http/1.1"))).connect(
            rustls::pki_types::ServerName::try_from("localhost").unwrap(),
            socket,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        stream.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
        expected
    );
    let mut request = format!("wss://{address}/connect")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    let upgrade = tokio::time::timeout(
        Duration::from_secs(3),
        tokio_tungstenite::client_async(request, stream),
    )
    .await
    .unwrap();
    if accepted {
        let (mut socket, response) = upgrade.unwrap();
        assert_eq!(response.status().as_u16(), 101);
        socket.close(None).await.unwrap();
    } else {
        assert!(upgrade.is_err());
    }
}
async fn wt(h: &Hosted, expected: &[u8], origin: &str, accepted: bool) {
    let mut config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls(h, b"h3")).unwrap(),
    ));
    let mut limits = quinn::TransportConfig::default();
    limits.max_concurrent_bidi_streams(8u32.into());
    limits.max_concurrent_uni_streams(8u32.into());
    limits.datagram_receive_buffer_size(Some(64 * 1024));
    limits.datagram_send_buffer_size(64 * 1024);
    config.transport_config(Arc::new(limits));
    let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    endpoint.set_default_client_config(config);
    let connection = tokio::time::timeout(
        Duration::from_secs(3),
        endpoint.connect(h.f.server.address, "localhost").unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let chain = connection
        .peer_identity()
        .unwrap()
        .downcast::<Vec<CertificateDer<'static>>>()
        .unwrap();
    assert_eq!(chain[0].as_ref(), expected);
    let negotiated = connection
        .handshake_data()
        .unwrap()
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .unwrap();
    assert_eq!(negotiated.protocol.as_deref(), Some(b"h3".as_slice()));
    let mut request = web_transport_quinn::proto::ConnectRequest::from(
        url::Url::parse(&format!(
            "https://localhost:{}/connect",
            h.f.server.address.port()
        ))
        .unwrap(),
    );
    request.headers.append("origin", origin.parse().unwrap());
    let upgrade = tokio::time::timeout(
        Duration::from_secs(3),
        web_transport_quinn::Session::connect(connection.clone(), request),
    )
    .await
    .unwrap();
    if accepted {
        upgrade.unwrap().close(0, b"routing witness complete");
    } else {
        assert!(upgrade.is_err());
    }
    connection.close(0u32.into(), b"routing witness complete");
    endpoint.close(0u32.into(), b"routing witness complete");
    tokio::time::timeout(Duration::from_secs(5), endpoint.wait_idle())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_https_wss_and_http3_share_certificate_reload_and_keep_their_origin_routes() {
    let h = Hosted::configured(vec![], 16, |config| {
        config.websocket = Some(
            connect_server::canonical_websocket::CanonicalWebSocketConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                mount_path: "/connect".into(),
                allowed_origins: vec![ORIGIN.into()],
                advertised_url: None,
            },
        );
        config.webtransport = Some(
            connect_server::canonical_webtransport::CanonicalWebTransportConfig {
                mount_path: "/connect".into(),
                allowed_origins: vec![ORIGIN.into()],
                advertised_url: None,
            },
        );
    })
    .await;
    let original = CertificateDer::pem_slice_iter(&std::fs::read(&h.tls.certificate).unwrap())
        .next()
        .unwrap()
        .unwrap()
        .to_vec();
    let app = h.url(HttpSurface::Application, "/v2/discovery");
    for phase in 0..3 {
        let expected = if phase == 0 {
            original.clone()
        } else if phase == 1 {
            let next = h.tls.replace();
            assert!(h.f.server.reload_tls_certificates().await.unwrap().changed);
            next
        } else {
            std::fs::write(&h.tls.key, b"incomplete cross-protocol replacement").unwrap();
            assert!(h.f.server.reload_tls_certificates().await.is_err());
            CertificateDer::pem_slice_iter(&std::fs::read(&h.tls.certificate).unwrap())
                .next()
                .unwrap()
                .unwrap()
                .to_vec()
        };
        wss(&h, &expected, ORIGIN, true).await;
        wt(&h, &expected, ORIGIN, true).await;
        let response = h
            .tls
            .http()
            .get(&app)
            .bearer_auth(&h.f.client.credential)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(leaf(&response), expected);
        let discovery = response
            .json::<connect_protocol::canonical_discovery::TransportDiscovery>()
            .await
            .unwrap();
        assert_eq!(
            discovery.websocket.unwrap().url,
            h.f.server.websocket_url.as_ref().unwrap().as_str()
        );
        assert_eq!(
            discovery.webtransport.unwrap().url,
            h.f.server.webtransport_url.as_ref().unwrap().as_str()
        );
        if phase == 1 {
            wss(&h, &expected, "https://foreign.fixture.invalid", false).await;
            wt(&h, &expected, "https://foreign.fixture.invalid", false).await;
        }
    }
    h.f.server.shutdown().await.unwrap();
    eprintln!(
        "hosted-cross-protocol {}",
        json!({"https_wss_http3_full_handshakes":9,"bad_origins_refused":2,"h3_negotiated":true,"current_discovery":true,"last_good_after_invalid_key":true,"joined":true})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_remote_https_origin_is_explicit_and_not_rewritten_by_host_or_proxy_headers() {
    let h = Hosted::configured(vec![], 8, |config| {
        config.network_profile = connect_client::NativeNetworkProfile::Remote;
        for listener in &mut config.https {
            listener.public_origin = Some("https://localhost".into());
        }
    })
    .await;
    let endpoint = h.endpoint(HttpSurface::Application);
    assert_eq!(endpoint.origin, "https://localhost");
    let url = format!("https://localhost:{}/v2/discovery", endpoint.address.port());
    let http = h.tls.http();
    assert_eq!(
        http.get(&url)
            .bearer_auth(&h.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = http
        .get(&url)
        .header("Host", "localhost")
        .bearer_auth(&h.f.client.credential)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        http.get(&url)
            .header("Host", "localhost")
            .header("x-forwarded-host", "foreign.fixture.invalid")
            .bearer_auth(&h.f.client.credential)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let bad = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .resolve("uncovered.fixture.invalid", endpoint.address)
        .add_root_certificate(
            reqwest::Certificate::from_pem(&std::fs::read(&h.tls.root).unwrap()).unwrap(),
        )
        .build()
        .unwrap();
    assert!(bad
        .get(format!(
            "https://uncovered.fixture.invalid:{}/v2/discovery",
            endpoint.address.port()
        ))
        .header("Host", "localhost")
        .bearer_auth(&h.f.client.credential)
        .send()
        .await
        .is_err());
    drop(response);
    drop(http);
    drop(bad);
    h.f.server.shutdown().await.unwrap();
    eprintln!(
        "hosted-remote-origin {}",
        json!({"remote_profile_on_isolated_loopback":true,"published_authority_required":true,"forged_proxy_cannot_rewrite":true,"wrong_tls_name_refused":true,"joined":true})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_https_bounds_live_raw_sockets_and_recovers_slots_after_handshake_timeout() {
    let h = Hosted::configured(vec![], 2, |config| {
        config.https.retain(|l| l.surface == HttpSurface::Health)
    })
    .await;
    let endpoint = h.endpoint(HttpSurface::Health);
    let address = endpoint.address;
    let url = h.url(HttpSurface::Health, "/healthz");
    let first = tokio::net::TcpStream::connect(address).await.unwrap();
    let second = tokio::net::TcpStream::connect(address).await.unwrap();
    let start = tokio::time::Instant::now();
    let http = h.tls.http();
    let error = http
        .get(&url)
        .timeout(Duration::from_secs(1))
        .send()
        .await
        .expect_err("unadmitted raw socket reached HTTPS");
    assert!(
        !error.is_timeout(),
        "capacity must refuse before waiting for client timeout"
    );
    tokio::time::sleep_until(start + Duration::from_millis(5500)).await;
    for mut socket in [first, second] {
        let mut bytes = [0; 1];
        let result = tokio::time::timeout(Duration::from_secs(1), socket.read(&mut bytes))
            .await
            .unwrap();
        assert!(
            matches!(result, Ok(0) | Err(_)),
            "raw handshake survived its original deadline"
        );
    }
    let response = http.get(&url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"profile":"connect/2","ready":true})
    );
    drop(http);
    h.f.server.shutdown().await.unwrap();
    eprintln!(
        "hosted-tls-capacity {}",
        json!({"raw_socket_capacity":2,"overload_refused_before_client_timeout":true,"original_handshake_deadline_seconds":5,"slots_reused_after_timeout":true,"joined":true})
    );
}
