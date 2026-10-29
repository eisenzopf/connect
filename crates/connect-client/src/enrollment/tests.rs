use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::provisioning::DeviceSpec;
use ring::signature;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn offer(key: &NativeEnrollmentKey) -> EnrollmentOffer {
    let now = chrono::Utc::now().timestamp();
    EnrollmentOffer {
        profile: PROFILE.into(),
        enrollment_id: "enrollment-client-test".into(),
        endpoint_id: "endpoint-client-test".into(),
        installation_id: "installation-client-test".into(),
        issuer_identity_id: "issuer-client-test".into(),
        challenge: "ac".repeat(32),
        request: EnrollmentRequest {
            operation_id: "approve-client-test".into(),
            completion_operation_id: "complete-client-test".into(),
            identity_id: "identity-client-test".into(),
            device: DeviceSpec {
                id: "device-client-test".into(),
                kind: "desktop".into(),
                platform: "fixture".into(),
            },
            key_id: "key-client-test".into(),
            algorithm: key.algorithm(),
            public_key: key.public_key().into(),
            transport: EnrollmentTransport::Quic,
            endpoint_address: "quic://device.fixture.invalid:4443/client-test".into(),
            capabilities: vec!["control".into()],
            browser_origin: None,
            operations: vec!["operation.get".into()],
            recipients: vec![],
            credential_expires_at: now + 240,
            challenge_expires_at: now + 120,
        },
    }
}
fn saved(path: &Path) -> NativeEnrollment {
    let key = NativeEnrollmentKey::generate(SigningAlgorithm::EdDSA).unwrap();
    let offer = offer(&key);
    NativeEnrollment::create(path, offer.clone(), offer.request, key).unwrap()
}
#[test]
fn native_keys_and_saved_original_offers_sign_both_real_algorithms() {
    for algorithm in [SigningAlgorithm::EdDSA, SigningAlgorithm::ES256] {
        let temp = tempfile::tempdir().unwrap();
        let key = NativeEnrollmentKey::generate(algorithm).unwrap();
        let original = temp.path().join("original.pk8");
        key.save(&original).unwrap();
        assert!(key.save(&original).is_err());
        let key = NativeEnrollmentKey::load(&original, algorithm).unwrap();
        let mut offer = offer(&key);
        // Expired original context remains signable for completed recovery.
        offer.request.challenge_expires_at = chrono::Utc::now().timestamp() - 1;
        let enrollment = NativeEnrollment::create(
            &temp.path().join("state"),
            offer.clone(),
            offer.request.clone(),
            key,
        )
        .unwrap();
        let claim = NativeEnrollment::load(&temp.path().join("state"))
            .unwrap()
            .claim()
            .unwrap();
        assert_eq!(claim.operation_id, offer.request.completion_operation_id);
        assert_eq!(claim.challenge, offer.challenge);
        let signature = claim.check_binding(&offer, None).unwrap();
        let public = URL_SAFE_NO_PAD.decode(&offer.request.public_key).unwrap();
        let verifier: &dyn signature::VerificationAlgorithm = match algorithm {
            SigningAlgorithm::EdDSA => &signature::ED25519,
            SigningAlgorithm::ES256 => &signature::ECDSA_P256_SHA256_FIXED,
        };
        signature::UnparsedPublicKey::new(verifier, public)
            .verify(&offer.proof_message().unwrap(), &signature)
            .unwrap();
        assert!(!format!("{enrollment:?}").contains(&offer.challenge));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(temp.path().join("state"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(temp.path().join("state/client.pk8"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
#[test]
fn saved_enrollment_refuses_broader_offer_wrong_key_browser_and_unsafe_state() {
    let temp = tempfile::tempdir().unwrap();
    for case in 0..3 {
        let key = NativeEnrollmentKey::generate(SigningAlgorithm::EdDSA).unwrap();
        let mut offer = offer(&key);
        let expected = offer.request.clone();
        if case == 0 {
            offer.request.operations.push("send".into());
        }
        if case == 1 {
            offer.request.public_key = NativeEnrollmentKey::generate(SigningAlgorithm::EdDSA)
                .unwrap()
                .public_key()
                .into();
        }
        if case == 2 {
            offer.request.transport = EnrollmentTransport::Webtransport;
            offer.request.browser_origin = Some("https://ui.fixture.invalid".into());
            offer.request.endpoint_address = "https://device.fixture.invalid/endpoint".into();
        }
        let directory = temp.path().join(format!("refused-{case}"));
        assert!(NativeEnrollment::create(&directory, offer, expected, key).is_err());
        assert!(!directory.exists());
    }
    let enrollment = saved(&temp.path().join("saved"));
    let wrong = NativeEnrollmentKey::generate(SigningAlgorithm::EdDSA).unwrap();
    assert!(NativeEnrollment::create(
        &enrollment.directory,
        enrollment.offer.clone(),
        enrollment.offer.request.clone(),
        wrong
    )
    .is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let link = temp.path().join("linked");
        symlink(&enrollment.directory, &link).unwrap();
        assert!(NativeEnrollment::load(&link).is_err());
        std::fs::set_permissions(
            enrollment.directory.join("offer.json"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(NativeEnrollment::load(&enrollment.directory).is_err());
    }
}
#[test]
fn enrollment_contacts_require_exact_secure_origin_or_explicit_loopback() {
    for origin in [
        "https://connect.fixture.invalid",
        "https://connect.fixture.invalid:4443",
        "http://127.0.0.1:1234",
        "http://[::1]:1234",
    ] {
        EnrollmentClientConfig {
            origin: origin.into(),
            trust_certificate: None,
            timeout_ms: 100,
        }
        .validate()
        .unwrap();
    }
    for origin in [
        "http://remote.fixture.invalid",
        "https://user:secret@fixture.invalid",
        "https://fixture.invalid/",
        "https://fixture.invalid/path",
        "https://fixture.invalid?q=secret",
        "https://fixture.invalid#secret",
        "https://fixture.invalid:0",
        "https://fixture.invalid:443",
    ] {
        assert!(
            EnrollmentClientConfig {
                origin: origin.into(),
                trust_certificate: None,
                timeout_ms: 100
            }
            .validate()
            .is_err(),
            "{origin}"
        );
    }
}
async fn response_server(response: Vec<u8>) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut part = [0; 2048];
        loop {
            let n = socket.read(&mut part).await.unwrap();
            assert_ne!(n, 0);
            bytes.extend_from_slice(&part[..n]);
            if let Some(i) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..i]);
                let length: usize = headers
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= i + 4 + length {
                    break;
                }
            }
        }
        let _ = socket.write_all(&response).await;
        bytes
    });
    (origin, server)
}
#[tokio::test]
async fn enrollment_http_never_sends_authority_and_sanitizes_refusals() {
    let temp = tempfile::tempdir().unwrap();
    let enrollment = saved(&temp.path().join("state"));
    let secret = "do-not-display-remote-secret";
    let body = format!(
        r#"{{"profile":"connect-enrollment/1","result":{{"Err":{{"code":"forbidden","message":"{secret}"}}}}}}"#
    );
    let (origin, server) = response_server(
        format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes(),
    )
    .await;
    let client = EnrollmentClient::open(EnrollmentClientConfig {
        origin,
        trust_certificate: None,
        timeout_ms: 1000,
    })
    .await
    .unwrap();
    let error = client.claim(&enrollment).await.unwrap_err();
    assert!(matches!(&error,ClientError::Application(e) if e.code=="forbidden"));
    assert!(!error.to_string().contains(secret));
    let bytes = server.await.unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (headers, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(!headers.to_ascii_lowercase().contains("authorization:"));
    assert!(!headers.to_ascii_lowercase().contains("origin:"));
    let claim: EnrollmentClaim = serde_json::from_str(body).unwrap();
    claim.check_binding(enrollment.offer(), None).unwrap();
}
#[tokio::test]
async fn enrollment_http_refuses_redirects_oversize_and_pre_cancel_without_io() {
    let temp = tempfile::tempdir().unwrap();
    let enrollment = saved(&temp.path().join("state"));
    for response in [b"HTTP/1.1 307 Temporary Redirect\r\nLocation: http://127.0.0.1:1/stolen\r\nContent-Length: 0\r\n\r\n".to_vec(),b"HTTP/1.1 200 OK\r\nContent-Length: 65537\r\n\r\n".to_vec()] {
        let (origin,server)=response_server(response).await;let client=EnrollmentClient::open(EnrollmentClientConfig{origin,trust_certificate:None,timeout_ms:1000}).await.unwrap();assert!(matches!(client.claim(&enrollment).await,Err(ClientError::Protocol)));server.await.unwrap();
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    cancel.cancel();
    let client = EnrollmentClient::open(EnrollmentClientConfig {
        origin: "http://127.0.0.1:1".into(),
        trust_certificate: None,
        timeout_ms: 1000,
    })
    .await
    .unwrap();
    assert!(matches!(
        client.claim_with_cancel(&enrollment, cancel).await,
        Err(ClientError::Cancelled)
    ));
}

#[tokio::test]
async fn enrollment_http_deadline_and_cancellation_cover_partial_response_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let enrollment = saved(&temp.path().join("state"));
    for cancelled in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (ready, observed) = tokio::sync::oneshot::channel();
        let (finish, stop) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut input = [0; 8192];
            assert!(socket.read(&mut input).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{")
                .await
                .unwrap();
            ready.send(()).unwrap();
            let _ = stop.await;
        });
        let client = EnrollmentClient::open(EnrollmentClientConfig {
            origin,
            trust_certificate: None,
            timeout_ms: if cancelled { 1000 } else { 100 },
        })
        .await
        .unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cloned = cancel.clone();
        let control = async {
            observed.await.unwrap();
            if cancelled {
                cloned.cancel();
            }
        };
        let (result, _) = tokio::join!(client.claim_with_cancel(&enrollment, cancel), control);
        if cancelled {
            assert!(matches!(result, Err(ClientError::Cancelled)));
        } else {
            assert!(matches!(result, Err(ClientError::Timeout)));
        }
        finish.send(()).unwrap();
        server.await.unwrap();
        assert_eq!(
            enrollment.claim().unwrap().operation_id,
            enrollment.offer.request.completion_operation_id
        );
    }
}
