//! Public connection entry points refuse incompatible peers before an SDK owner exists.
use super::*;
use crate::sdk::{self, Client, ClientConfig, RequestOptions};
#[derive(Clone, Copy, Debug)]
enum Case {
    Current,
    LegacyAlpn,
    FutureAlpn,
    LegacyEnvelope,
    FutureEnvelope,
    ForeignChallengeProfile,
    MissingChallengeProfile,
    ForeignChallengeReply,
    InvalidChallengeSignature,
    ForeignSessionProfile,
    FutureSessionEnvelope,
    ForeignSessionOwner,
    ForeignSessionReply,
}
async fn read(reader: &mut quinn::RecvStream) -> std::result::Result<Value, ()> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix).await.map_err(|_| ())?;
    let length = u32::from_be_bytes(prefix) as usize;
    assert!((1..=MAX_ENVELOPE_BYTES).contains(&length));
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes).await.map_err(|_| ())?;
    Ok(serde_json::from_slice(&bytes).unwrap())
}
async fn send(writer: &mut quinn::SendStream, value: Value) {
    let bytes = serde_json::to_vec(&value).unwrap();
    writer
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    writer.write_all(&bytes).await.unwrap();
}
async fn run(case: Case, algorithm: &str, configured: bool) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let directory = tempfile::tempdir().unwrap();
    let (cert, key) = rvoip_uctp::substrate::self_signed_for_dev(&["localhost".into()]).unwrap();
    std::fs::write(directory.path().join("tls.der"), cert.as_ref()).unwrap();
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    tls.alpn_protocols = vec![match case {
        Case::LegacyAlpn => b"uctp/1".to_vec(),
        Case::FutureAlpn => b"uctp/3".to_vec(),
        _ => RAW_QUIC_ALPN.to_vec(),
    }];
    let server = quinn::Endpoint::server(
        quinn::ServerConfig::with_crypto(Arc::new(
            quinn::crypto::rustls::QuicServerConfig::try_from(tls).unwrap(),
        )),
        "127.0.0.1:0".parse().unwrap(),
    )
    .unwrap();
    let endpoint = server.local_addr().unwrap();
    let key_algorithm = if algorithm == "EdDSA" {
        &rcgen::PKCS_ED25519
    } else {
        &rcgen::PKCS_ECDSA_P256_SHA256
    };
    let signing = rcgen::KeyPair::generate_for(key_algorithm).unwrap();
    let signer = if algorithm == "EdDSA" {
        Sig9421Signer::ed25519("server", &signing.serialize_der()).unwrap()
    } else {
        Sig9421Signer::es256("server", &signing.serialize_der()).unwrap()
    };
    std::fs::write(
        directory.path().join("server.pub"),
        signing.public_key_raw(),
    )
    .unwrap();
    let client_signing = rcgen::KeyPair::generate_for(key_algorithm).unwrap();
    std::fs::write(
        directory.path().join("client.key"),
        client_signing.serialize_der(),
    )
    .unwrap();
    let config = CanonicalClientConfig {
        network_profile: NativeNetworkProfile::Local,
        endpoint,
        server_name: "localhost".into(),
        trust_certificate: directory.path().join("tls.der"),
        credential: "private-credential-canary".into(),
        identity_id: "identity".into(),
        device_id: "device".into(),
        device_kind: "test".into(),
        device_platform: "isolated".into(),
        signing_key_id: "client".into(),
        signing_algorithm: algorithm.into(),
        signing_private_key: directory.path().join("client.key"),
        server_key_id: "server".into(),
        server_algorithm: algorithm.into(),
        server_public_key: directory.path().join("server.pub"),
        server_key_not_before: chrono::Utc::now() - chrono::Duration::minutes(1),
        server_key_expires_at: chrono::Utc::now() + chrono::Duration::minutes(2),
        timeout_ms: 3000,
    };
    // Explicit application configuration must never become a fallback for a
    // refused native handshake. No HTTP connection is accepted in any scene.
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let application = crate::application::ApplicationClientConfig {
        endpoint: http.local_addr().unwrap(),
        credential: config.credential.clone(),
        expires_at: config.server_key_expires_at,
        timeout_ms: 3000,
    };
    let host = tokio::spawn(async move {
        let incoming = server.accept().await.unwrap().await;
        if matches!(case, Case::LegacyAlpn | Case::FutureAlpn) {
            assert!(incoming.is_err(), "incompatible ALPN admitted");
        } else {
            let connection = incoming.unwrap();
            let (mut writer, mut reader) = connection.accept_bi().await.unwrap();
            let hello = read(&mut reader).await.unwrap();
            assert_eq!(hello["type"], "auth.hello");
            assert_eq!(hello["v"], 2);
            assert_eq!(
                hello["payload"]["application_profiles"],
                json!(["connect/2"])
            );
            assert_eq!(hello["payload"]["auth_methods"], json!(["bearer"]));
            assert!(!serde_json::to_string(&hello)
                .unwrap()
                .contains("private-credential-canary"));
            let mut challenge = envelope(
                "auth.challenge",
                json!({"nonce":"challenge","accepted_methods":["bearer"],"server_capabilities":{"application_profiles":["connect/2"]}}),
            );
            challenge["in_reply_to"] = hello["id"].clone();
            match case {
                Case::LegacyEnvelope => challenge["v"] = json!(1),
                Case::FutureEnvelope => challenge["v"] = json!(3),
                Case::ForeignChallengeProfile => {
                    challenge["payload"]["server_capabilities"]["application_profiles"] =
                        json!(["foreign/9"])
                }
                Case::MissingChallengeProfile => {
                    challenge["payload"]["server_capabilities"] = json!({})
                }
                Case::ForeignChallengeReply => challenge["in_reply_to"] = json!("foreign"),
                _ => (),
            }
            let mut signed = signer.sign(&challenge).unwrap();
            if matches!(case, Case::InvalidChallengeSignature) {
                signed["payload"]["nonce"] = json!("tampered");
            }
            send(&mut writer, signed).await;
            if matches!(
                case,
                Case::Current
                    | Case::ForeignSessionProfile
                    | Case::FutureSessionEnvelope
                    | Case::ForeignSessionOwner
                    | Case::ForeignSessionReply
            ) {
                let response = read(&mut reader).await.unwrap();
                assert_eq!(response["type"], "auth.response");
                assert_eq!(response["in_reply_to"], challenge["id"]);
                assert_eq!(
                    response["payload"]["credential"],
                    "private-credential-canary"
                );
                let mut session = envelope(
                    "auth.session",
                    json!({"identity_id":"identity","session_token":"private-session-canary","expires_at":chrono::Utc::now()+chrono::Duration::minutes(1),"assurance":"identified","application_profile":"connect/2"}),
                );
                session["in_reply_to"] = response["id"].clone();
                match case {
                    Case::ForeignSessionProfile => {
                        session["payload"]["application_profile"] = json!("foreign/9")
                    }
                    Case::FutureSessionEnvelope => session["v"] = json!(3),
                    Case::ForeignSessionOwner => {
                        session["payload"]["identity_id"] = json!("foreign")
                    }
                    Case::ForeignSessionReply => session["in_reply_to"] = json!("foreign"),
                    _ => (),
                }
                send(&mut writer, signer.sign(&session).unwrap()).await;
            }
            // The connection must close with no application command or leaked
            // credential response after a refused challenge; EOF is required.
            assert!(
                tokio::time::timeout(Duration::from_secs(2), read(&mut reader))
                    .await
                    .unwrap()
                    .is_err()
            );
            tokio::time::timeout(Duration::from_secs(2), connection.closed())
                .await
                .unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(30), server.accept())
                .await
                .is_err(),
            "unexpected reconnect/downgrade"
        );
        server.close(0u32.into(), b"compatibility fixture complete");
        server.wait_idle().await;
    });
    let result = if configured {
        Client::connect_configured(ClientConfig {
            control: config,
            tenant_id: "tenant".parse().unwrap(),
            application: Some(application),
        })
        .await
    } else {
        Client::connect(config, "tenant".parse().unwrap()).await
    };
    if matches!(case, Case::Current) {
        let client = result.unwrap();
        let options = RequestOptions::new(Duration::from_secs(2)).unwrap();
        assert_eq!(
            client
                .capabilities()
                .refresh(options.clone())
                .await
                .unwrap()
                .profile,
            "connect/2"
        );
        client.close(options).await.unwrap();
    } else {
        let error = result.err().expect("incompatible server admitted");
        let expected = match case {
            Case::LegacyAlpn | Case::FutureAlpn => sdk::ErrorKind::TransportUncertain,
            Case::LegacyEnvelope
            | Case::FutureEnvelope
            | Case::ForeignSessionProfile
            | Case::FutureSessionEnvelope => sdk::ErrorKind::Protocol,
            _ => sdk::ErrorKind::Authentication,
        };
        assert_eq!(error.kind, expected, "{algorithm} {case:?}");
        assert_eq!(error.certainty, sdk::Certainty::ReadOnly);
        assert!(error.operation.is_none() && error.problem.is_none());
        assert!(!format!("{error:?} {error}").contains("canary"));
    }
    tokio::time::timeout(Duration::from_secs(4), host)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), http.accept())
            .await
            .is_err(),
        "native refusal fell back to HTTP"
    );
}
#[tokio::test]
async fn public_sdk_refuses_incompatible_server_alpn_envelopes_profiles_and_bindings_without_fallback(
) {
    for algorithm in ["EdDSA", "ES256"] {
        for case in [
            Case::LegacyAlpn,
            Case::FutureAlpn,
            Case::LegacyEnvelope,
            Case::FutureEnvelope,
            Case::ForeignChallengeProfile,
            Case::MissingChallengeProfile,
            Case::ForeignChallengeReply,
            Case::InvalidChallengeSignature,
            Case::ForeignSessionProfile,
            Case::FutureSessionEnvelope,
            Case::ForeignSessionOwner,
            Case::ForeignSessionReply,
        ] {
            run(case, algorithm, true).await;
        }
    }
}
#[tokio::test]
async fn public_sdk_current_signed_profile_connects_and_joins_through_both_configuration_entry_points(
) {
    for algorithm in ["EdDSA", "ES256"] {
        for configured in [false, true] {
            run(Case::Current, algorithm, configured).await;
        }
    }
}
