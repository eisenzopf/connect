//! Actual protected issuance, independently enrolled child and original recovery.
use super::*;
use connect_client::application::{
    credential_issuance::*, ApplicationClient, ApplicationClientConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn config(f: &Fixture, credential: &str) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: credential.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 5000,
    }
}
fn intent(f: &Fixture, id: &str) -> CredentialIssueRequest {
    CredentialIssueRequest {
        operation_id: id.into(),
        identity_id: f.client.identity_id.clone(),
        operations: vec!["operation.get".into()],
        recipients: vec![f.peer.identity_id.clone()],
        expires_at: chrono::Utc::now().timestamp() + 240,
    }
}
async fn raw(f: &Fixture, token: Option<&str>, extra: &str, body: &str) -> (u16, Value, String) {
    let address = f.server.http_address.unwrap();
    let auth = token
        .map(|v| format!("Authorization: Bearer {v}\r\n"))
        .unwrap_or_default();
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket.write_all(format!("POST /v2/credentials HTTP/1.1\r\nHost: {address}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",body.len()).as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(7), socket.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    let index = bytes.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
    let headers = String::from_utf8(bytes[..index].to_vec()).unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    let value = serde_json::from_slice(&bytes[index..]).unwrap();
    (status, value, headers)
}
#[tokio::test]
async fn protected_issuance_recovers_original_after_disconnect_restart_and_native_enrollment() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let request = intent(&f, "issue-http-original");
        let mut admin = ApplicationClient::open(config(&f, &f.admin)).await.unwrap();
        // Send the complete original, then lose observation without consuming any response.
        let body = serde_json::to_vec(&request).unwrap();
        let address = f.server.http_address.unwrap();
        let mut lost = tokio::net::TcpStream::connect(address).await.unwrap();
        lost.write_all(format!("POST /v2/credentials HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",f.admin,body.len()).as_bytes()).await.unwrap();
        lost.write_all(&body).await.unwrap();
        drop(lost);
        let issued = admin
            .issue_scoped_credential(request.clone())
            .await
            .unwrap();
        assert_eq!(
            issued.installation_id,
            f.server.store.installation_id().unwrap()
        );
        assert_eq!(
            issued.issuer_credential_id,
            f.server.store.authenticate(&f.admin).unwrap().credential_id
        );
        let mut equivalent = request.clone();
        equivalent.operations.push("operation.get".into());
        assert_eq!(
            admin.issue_scoped_credential(equivalent).await.unwrap(),
            issued
        );
        let mut changed = request.clone();
        changed.expires_at += 1;
        assert!(
            matches!(admin.issue_scoped_credential(changed).await,Err(ClientError::Application(e)) if e.code=="conflict")
        );
        let mut child = ApplicationClient::open(config(&f, &issued.credential))
            .await
            .unwrap();
        assert!(
            matches!(child.issue_scoped_credential(request.clone()).await,Err(ClientError::Application(e)) if e.code=="forbidden")
        );
        let mut native = f.client.clone();
        native.credential = issued.credential.clone();
        native.device_id = "issued-fresh-device".into();
        native.signing_key_id = "issued-fresh-key".into();
        native.device_platform = "issuance-fixture".into();
        native.signing_private_key = f.temp.path().join("issued-fresh-key.der");
        let public = signing(&native.signing_private_key, algorithm);
        let endpoint = ProvisioningCommand::Endpoint {
            operation_id: "issued-enroll-device".into(),
            identity_id: issued.identity_id.clone(),
            device: DeviceSpec {
                id: native.device_id.clone(),
                kind: "desktop".into(),
                platform: "issuance-fixture".into(),
            },
            transport: EndpointTransport::Quic,
            address: format!("quic://{}/{}", f.server.address, native.device_id),
            capabilities: vec!["data".into()],
            secret_ref: None,
        };
        let key = ProvisioningCommand::SigningKey {
            operation_id: "issued-enroll-key".into(),
            identity_id: issued.identity_id.clone(),
            credential_id: issued.credential_id.clone(),
            device_id: native.device_id.clone(),
            key_id: native.signing_key_id.clone(),
            algorithm: if algorithm == "ES256" {
                SigningAlgorithm::ES256
            } else {
                SigningAlgorithm::EdDSA
            },
            public_key: URL_SAFE_NO_PAD.encode(public),
            not_before: issued.issued_at,
            expires_at: issued.expires_at,
            expected_revision: None,
        };
        for command in [endpoint, key] {
            let expected = admin.provision(command.clone()).await.unwrap();
            assert_eq!(admin.provision(command).await.unwrap(), expected);
        }
        let connected = CanonicalClient::open(native.clone()).await.unwrap();
        connected.shutdown().await.unwrap();
        f.config.bind = f.server.address;
        f.config.http_bind = f.server.http_address;
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        let mut fresh = ApplicationClient::open(config(&f, &f.admin)).await.unwrap();
        assert_eq!(
            fresh
                .issue_scoped_credential(request.clone())
                .await
                .unwrap(),
            issued
        );
        let connected = CanonicalClient::open(native.clone()).await.unwrap();
        connected.shutdown().await.unwrap();
        fresh
            .provision(ProvisioningCommand::RevokeCredential {
                operation_id: "revoke-issued".into(),
                credential_id: issued.credential_id,
            })
            .await
            .unwrap();
        assert!(fresh.issue_scoped_credential(request).await.is_err());
        assert!(CanonicalClient::open(native).await.is_err());
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn protected_issuance_authenticates_before_body_and_refuses_header_shape_and_authority_changes(
) {
    let f = Fixture::new("EdDSA", 8).await;
    let valid = serde_json::to_string(&intent(&f, "http-refusals")).unwrap();
    for (token, extra, body, status) in [
        (None, "", "{", 401),
        (Some("ctg1.model-grant"), "", "{", 401),
        (Some(f.client.credential.as_str()), "", "{", 403),
        (
            Some(f.admin.as_str()),
            "Origin: http://localhost\r\n",
            valid.as_str(),
            403,
        ),
        (
            Some(f.admin.as_str()),
            "X-Call-ID: guessed\r\n",
            valid.as_str(),
            403,
        ),
        (
            Some(f.admin.as_str()),
            "X-Chat-ID: guessed\r\n",
            valid.as_str(),
            403,
        ),
        (
            Some(f.admin.as_str()),
            "Host: other.example\r\n",
            valid.as_str(),
            403,
        ),
        (
            Some(f.admin.as_str()),
            "Upgrade: h2c\r\n",
            valid.as_str(),
            400,
        ),
        (Some(f.admin.as_str()), "", "{", 400),
    ] {
        let (actual, value, headers) = raw(&f, token, extra, body).await;
        assert_eq!(actual, status);
        assert_eq!(value["profile"], PROFILE);
        assert!(value["result"]["Err"].is_object());
        assert!(headers
            .to_ascii_lowercase()
            .contains("cache-control: no-store"));
        assert!(headers.to_ascii_lowercase().contains("connection: close"));
    }
    let oversized = " ".repeat(65_537);
    assert_eq!(raw(&f, Some(&f.admin), "", &oversized).await.0, 413);
    let mut value = serde_json::from_str::<Value>(&valid).unwrap();
    value["user_authorization"] = json!({"user_id":"forged"});
    assert_eq!(raw(&f, Some(&f.admin), "", &value.to_string()).await.0, 400);
    let (status, value, headers) = raw(&f, Some(&f.admin), "", &valid).await;
    assert_eq!(status, 200);
    assert!(value["result"]["Ok"]["credential"].is_string());
    assert!(headers.to_ascii_lowercase().contains("pragma: no-cache"));
    let mut admin = ApplicationClient::open(config(&f, &f.admin)).await.unwrap();
    let issuer = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    admin
        .provision(ProvisioningCommand::RevokeCredential {
            operation_id: "revoke-issuer-http".into(),
            credential_id: issuer,
        })
        .await
        .unwrap();
    assert_eq!(raw(&f, Some(&f.admin), "", &valid).await.0, 401);
    f.server.shutdown().await.unwrap();
}
