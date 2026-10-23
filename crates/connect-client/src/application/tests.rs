//! Controlled local HTTP peers qualify client bounds, not server authorization.
#![allow(clippy::unwrap_used)]
use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn configuration(endpoint: SocketAddr) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint,
        credential: "private-application-canary".into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(1),
        timeout_ms: 2_000,
    }
}
fn health() -> ApplicationOperation {
    Operation::Health {}.try_into().unwrap()
}
fn response(result: connect_protocol::Result<Outcome>) -> Vec<u8> {
    serde_json::to_vec(&Response {
        profile: PROFILE.into(),
        result,
    })
    .unwrap()
}
fn healthy() -> Outcome {
    Outcome::Health {
        installation_id: "inst_test".into(),
        schema: 34,
        profile: PROFILE.into(),
        readiness: None,
    }
}
async fn read(stream: &mut TcpStream) -> Vec<u8> {
    read_at(stream, "/v1/operations").await
}
async fn read_at(stream: &mut TcpStream, path: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut one = [0; 1];
    loop {
        stream.read_exact(&mut one).await.unwrap();
        bytes.push(one[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
        assert!(bytes.len() < 8192);
    }
    let headers = String::from_utf8(bytes).unwrap();
    assert!(headers.starts_with(&format!("POST {path} HTTP/1.1\r\n")));
    let len = headers
        .lines()
        .find_map(|v| {
            v.to_ascii_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_owned)
        })
        .unwrap()
        .parse::<usize>()
        .unwrap();
    assert!(len <= MAX_INPUT);
    let mut body = vec![0; len];
    stream.read_exact(&mut body).await.unwrap();
    body
}
async fn single(
    status: &str,
    extra: &str,
    body: Vec<u8>,
) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let header = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n{extra}\r\n",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let input = read(&mut stream).await;
        if stream.write_all(header.as_bytes()).await.is_ok() {
            let _ = stream.write_all(&body).await;
        }
        input
    });
    (address, task)
}
#[test]
fn canonical_commands_cannot_be_constructed_as_application_operations() {
    let commands = [
        Operation::Create {
            request_id: "saved".into(),
            request: crate::conversations::CreateRequest::persistent(vec![]),
        },
        Operation::History {
            conversation_id: "conv_one".into(),
            after: None,
            limit: 10,
        },
        Operation::Conversations {
            query: Default::default(),
        },
        Operation::RequestStatus {
            request_id: "saved".into(),
        },
        Operation::ConversationSnapshot {
            conversation_id: "conv_one".into(),
            query: Default::default(),
        },
        Operation::ConversationEvents {
            conversation_id: "conv_one".into(),
            query: Default::default(),
        },
        Operation::MessageReceipt {
            request_id: "saved".into(),
            conversation_id: "conv_one".into(),
            message_id: "msg_one".into(),
            state: connect_protocol::receipts::ClientReceiptState::Read,
        },
    ];
    for command in commands {
        assert!(
            matches!(ApplicationOperation::try_from(command),Err(ClientError::Application(e)) if e.code=="canonical-command-required")
        );
    }
}
#[tokio::test]
async fn application_responses_require_matching_profile_kind_and_success_status() {
    for (index, (status, body)) in [
        ("200 OK", response(Ok(healthy()))),
        ("503 Unavailable", response(Ok(healthy()))),
        (
            "200 OK",
            response(Ok(Outcome::Identity(connect_protocol::Identity {
                id: "id_test".into(),
                alias: "alias".into(),
                capabilities: vec![],
            }))),
        ),
        (
            "200 OK",
            serde_json::to_vec(&Response {
                profile: "other-profile".into(),
                result: Ok(healthy()),
            })
            .unwrap(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (endpoint, server) = single(status, "", body).await;
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let result = client.execute(health()).await;
        if index == 0 {
            assert!(matches!(result, Ok(Outcome::Health { .. })));
        } else {
            assert!(matches!(result, Err(ClientError::Protocol)));
        }
        server.await.unwrap();
    }
}
#[tokio::test]
async fn application_errors_never_echo_untrusted_server_reason_or_code() {
    let canary = "private-application-canary";
    let (endpoint, server) = single(
        "503 Unavailable",
        "",
        response(Err(Error::new(canary, canary))),
    )
    .await;
    let mut client = ApplicationClient::open(configuration(endpoint))
        .await
        .unwrap();
    let error = client.execute(health()).await.unwrap_err();
    assert!(!format!("{error:?} {error}").contains(canary));
    assert!(matches!(error, ClientError::Protocol));
    server.await.unwrap();
    for code in [
        "authority-busy",
        "authority-unavailable",
        "stale-revision",
        "unsupported-command",
    ] {
        let status = if code.starts_with("authority-") {
            "503 Unavailable"
        } else {
            "200 OK"
        };
        let (endpoint, server) = single(status, "", response(Err(Error::new(code, canary)))).await;
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let error = client.execute(health()).await.unwrap_err();
        assert!(!format!("{error:?} {error}").contains(canary));
        assert!(matches!(error, ClientError::Application(e) if e.code == code));
        server.await.unwrap();
    }
}
#[tokio::test]
async fn application_redirects_do_not_forward_the_credential() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (endpoint, server) = single(
        "302 Found",
        &format!(
            "location: http://{}/capture\r\n",
            target.local_addr().unwrap()
        ),
        vec![],
    )
    .await;
    let mut client = ApplicationClient::open(configuration(endpoint))
        .await
        .unwrap();
    assert!(matches!(
        client.execute(health()).await,
        Err(ClientError::Protocol)
    ));
    server.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(150), target.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn application_size_bounds_cover_announced_and_chunked_bodies() {
    let (endpoint, server) = single("200 OK", "", vec![b'x'; MAX_RESPONSE + 1]).await;
    let mut client = ApplicationClient::open(configuration(endpoint))
        .await
        .unwrap();
    assert!(matches!(
        client.execute(health()).await,
        Err(ClientError::Protocol)
    ));
    server.await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read(&mut stream).await;
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        for _ in 0..3 {
            let chunk = vec![b'x'; 750_000];
            let size = format!("{:x}\r\n", chunk.len());
            if stream.write_all(size.as_bytes()).await.is_err()
                || stream.write_all(&chunk).await.is_err()
                || stream.write_all(b"\r\n").await.is_err()
            {
                break;
            }
        }
    });
    let mut client = ApplicationClient::open(configuration(endpoint))
        .await
        .unwrap();
    assert!(matches!(
        client.execute(health()).await,
        Err(ClientError::Protocol)
    ));
    server.await.unwrap();
}
#[tokio::test]
async fn ready_http_chunks_cannot_extend_deadline_or_original_expiry() {
    for expiry in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read(&mut stream).await;
            let body = response(Ok(healthy()));
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            for chunk in body.chunks(5) {
                if stream.write_all(chunk).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        if expiry {
            client.config.expires_at = chrono::Utc::now() + chrono::Duration::milliseconds(100);
        } else {
            client.config.timeout_ms = 100;
        }
        let start = tokio::time::Instant::now();
        assert!(matches!(
            client.execute(health()).await,
            Err(ClientError::Timeout)
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
        server.await.unwrap();
    }
}
#[tokio::test]
async fn application_preflight_cancellation_expiry_and_task_discovery_have_no_network_effect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let mut client = ApplicationClient::open(configuration(endpoint))
        .await
        .unwrap();
    let token = CancellationToken::new();
    token.cancel();
    assert!(matches!(
        client.execute_with_cancel(health(), token).await,
        Err(ClientError::Cancelled)
    ));
    let task = Operation::Task {
        request: Box::new(tasks::TaskRequest::new(tasks::TaskCommand::Get {
            task_id: "task_test".into(),
        })),
    }
    .try_into()
    .unwrap();
    assert!(
        matches!(client.execute(task).await,Err(ClientError::Application(e)) if e.code=="unsupported-profile")
    );
    let large = Operation::ArtifactAppend {
        request_id: "saved_large".into(),
        conversation_id: "conv_one".into(),
        artifact_id: "art_one".into(),
        offset: 0,
        data: "x".repeat(MAX_INPUT),
    }
    .try_into()
    .unwrap();
    assert!(
        matches!(client.execute(large).await, Err(ClientError::Application(e)) if e.code=="invalid-request")
    );
    client.config.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(matches!(
        client.execute(health()).await,
        Err(ClientError::Authentication)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), listener.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn application_partial_cancellation_is_explicit_and_retry_preserves_logical_id() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let ready = std::sync::Arc::new(tokio::sync::Notify::new());
    let signal = ready.clone();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        let (mut first, _) = listener.accept().await.unwrap();
        requests.push(read(&mut first).await);
        first
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\nconnection: close\r\n\r\n{")
            .await
            .unwrap();
        signal.notify_one();
        let (mut second, _) = listener.accept().await.unwrap();
        requests.push(read(&mut second).await);
        let body = response(Err(Error::forbidden()));
        let header = format!(
            "HTTP/1.1 403 Forbidden\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        );
        second.write_all(header.as_bytes()).await.unwrap();
        second.write_all(&body).await.unwrap();
        requests
    });
    let mut client = ApplicationClient::open(configuration(endpoint))
        .await
        .unwrap();
    let token = CancellationToken::new();
    let cancel = token.clone();
    let operation = || {
        Operation::ArtifactDelete {
            request_id: "saved_application_delete".into(),
            conversation_id: "conv_one".into(),
            artifact_id: "art_one".into(),
        }
        .try_into()
        .unwrap()
    };
    let (result, ()) = tokio::join!(client.execute_with_cancel(operation(), token), async {
        ready.notified().await;
        cancel.cancel();
    });
    assert!(matches!(result, Err(ClientError::Cancelled)));
    assert!(
        matches!(client.execute(operation()).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
}

#[test]
fn protected_application_task_grant_replaces_owner_and_cannot_extend_original_expiry() {
    let config = configuration("127.0.0.1:4434".parse().unwrap());
    let original = config.expires_at;
    let grant = tasks::InvocationGrant {
        id: "grant_test".into(),
        task_id: "task_test".into(),
        binding: tasks::InvocationBinding {
            kind: tasks::InvocationKind::Native,
            adapter: "native".into(),
            external_id: "trusted-test-origin".into(),
        },
        operations: vec![],
        expires_at: original.timestamp() + 300,
        revoked: false,
    };
    let protected = tasks::ProtectedTaskGrant::new(grant.clone(), "grant-only-canary".into());
    let narrowed = config.clone().with_task_grant(&protected).unwrap();
    assert_eq!(narrowed.credential, "grant-only-canary");
    assert_eq!(narrowed.expires_at, original);
    let mut invalid = grant.clone();
    invalid.revoked = true;
    assert!(config
        .clone()
        .with_task_grant(&tasks::ProtectedTaskGrant::new(
            invalid,
            "grant-only-canary".into()
        ))
        .is_err());
    let mut invalid = grant;
    invalid.binding.kind = tasks::InvocationKind::ProviderCall;
    assert!(config
        .with_task_grant(&tasks::ProtectedTaskGrant::new(
            invalid,
            "grant-only-canary".into()
        ))
        .is_err());
}

#[test]
fn application_configuration_rejects_header_controls_and_command_input_rejects_authority_claims() {
    let mut config = configuration("127.0.0.1:4434".parse().unwrap());
    config.credential = "token\r\nInjected: value".into();
    assert!(matches!(config.validate(), Err(ClientError::Configuration)));
    let input = serde_json::json!({"command":{"type":"conversation.list","payload":{}},"scope":{"principal":{"subject":"operator"}}});
    assert!(serde_json::from_value::<crate::canonical::Command>(input).is_err());
    let input = serde_json::json!({"command":{"type":"conversation.list","payload":{}},"scope":{},"id":"chosen_envelope"});
    assert!(serde_json::from_value::<crate::canonical::Command>(input).is_err());
}

fn provision_command() -> provisioning::ProvisioningCommand {
    provisioning::ProvisioningCommand::RevokeDevice {
        operation_id: "saved-device-revocation".into(),
        device_id: "device_test".into(),
        expected_revision: 1,
    }
}
fn revoked_device() -> provisioning::ProvisioningOutcome {
    provisioning::ProvisioningOutcome::Device {
        device_id: "device_test".into(),
        identity_id: "id_test".into(),
        revision: 2,
        revoked: true,
    }
}
async fn provisioning_peer(
    status: &str,
    body: Vec<u8>,
) -> (SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let header = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let input = read_at(&mut stream, "/v2/devices/device_test/revoke").await;
        if stream.write_all(header.as_bytes()).await.is_ok() {
            let _ = stream.write_all(&body).await;
        }
        input
    });
    (address, task)
}
#[tokio::test]
async fn provisioning_correlates_target_revision_revocation_profile_kind_and_status() {
    use provisioning::*;
    let correct = serde_json::to_value(revoked_device()).unwrap();
    for (index, field, wrong) in [
        (0, "device_id", serde_json::json!("device_test")),
        (1, "device_id", serde_json::json!("foreign")),
        (2, "revision", serde_json::json!(1)),
        (3, "revoked", serde_json::json!(false)),
        (4, "kind", serde_json::json!("identity")),
        (5, "device_id", serde_json::json!("device_test")),
        (6, "device_id", serde_json::json!("device_test")),
    ] {
        let mut outcome = correct.clone();
        outcome[field] = wrong;
        let body = serde_json::to_vec(&serde_json::json!({
            "profile": if index==5 {"other"} else {PROVISIONING_PROFILE},
            "result":{"Ok":outcome}
        }))
        .unwrap();
        let (endpoint, peer) = provisioning_peer(
            if index == 6 {
                "503 Unavailable"
            } else {
                "200 OK"
            },
            body,
        )
        .await;
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let result = client.provision(provision_command()).await;
        if index == 0 {
            assert_eq!(result.unwrap(), revoked_device());
        } else {
            assert!(matches!(result, Err(ClientError::Protocol)), "case {index}");
        }
        assert_eq!(
            serde_json::from_slice::<ProvisioningCommand>(&peer.await.unwrap()).unwrap(),
            provision_command()
        );
    }
}
#[tokio::test]
async fn provisioning_preflight_rejects_path_injection_cancellation_expiry_and_oversize_without_network(
) {
    use provisioning::*;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = ApplicationClient::open(configuration(listener.local_addr().unwrap()))
        .await
        .unwrap();
    for target in ["../private-canary?token=secret", ".", ".."] {
        let mut invalid = provision_command();
        if let ProvisioningCommand::RevokeDevice { device_id, .. } = &mut invalid {
            *device_id = target.into();
        }
        let error = client.provision(invalid).await.unwrap_err();
        assert!(matches!(&error,ClientError::Application(e) if e.code=="invalid-request"));
        assert!(!format!("{error:?}").contains("canary"));
    }
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        client
            .provision_with_cancel(provision_command(), cancelled)
            .await,
        Err(ClientError::Cancelled)
    ));
    // Each capability is legal, but their serialized aggregate exceeds the body cap.
    let huge = ProvisioningCommand::Endpoint {
        operation_id: "saved-large".into(),
        identity_id: "id_test".into(),
        device: DeviceSpec {
            id: "device_test".into(),
            kind: "desktop".into(),
            platform: "macos".into(),
        },
        transport: EndpointTransport::Quic,
        address: "quic://127.0.0.1:9".into(),
        capabilities: (0..600)
            .map(|n| format!("c{n:03}{}", "x".repeat(120)))
            .collect(),
        secret_ref: None,
    };
    // Contract admission may be stricter than the aggregate wire limit.
    assert!(
        matches!(client.provision(huge).await,Err(ClientError::Application(e)) if e.code=="invalid-request")
    );
    client.config.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
    assert!(matches!(
        client.provision(provision_command()).await,
        Err(ClientError::Authentication)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn provisioning_uses_bounded_streaming_and_sanitized_errors_without_retry() {
    use provisioning::*;
    for (status, body, expected) in [
        ("200 OK", vec![b'x'; MAX_RESPONSE + 1], "protocol"),
        ("302 Found", vec![], "protocol"),
        (
            "503 Unavailable",
            serde_json::to_vec(&ProvisioningResponse {
                profile: PROVISIONING_PROFILE.into(),
                result: Err(Error::new(
                    "transport-uncertain",
                    "private-provisioning-canary",
                )),
            })
            .unwrap(),
            "transport-uncertain",
        ),
    ] {
        let (endpoint, peer) = provisioning_peer(status, body).await;
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let error = client.provision(provision_command()).await.unwrap_err();
        assert!(!format!("{error:?}").contains("canary"));
        if expected == "protocol" {
            assert!(matches!(error, ClientError::Protocol));
        } else {
            assert!(matches!(error,ClientError::Application(e) if e.code==expected));
        }
        peer.await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = configuration(listener.local_addr().unwrap());
    config.timeout_ms = 60;
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_at(&mut stream, "/v2/devices/device_test/revoke").await;
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n1\r\n{\r\n")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
    });
    let mut client = ApplicationClient::open(config).await.unwrap();
    assert!(matches!(
        client.provision(provision_command()).await,
        Err(ClientError::Timeout)
    ));
    peer.await.unwrap();
}

#[tokio::test]
async fn credential_revocation_correlates_immutable_target_and_requires_revoked_state() {
    use provisioning::*;
    for (index, target, revoked, status) in [
        (0, "credential_test", true, "200 OK"),
        (1, "foreign_credential", true, "200 OK"),
        (2, "credential_test", false, "200 OK"),
        (3, "credential_test", true, "503 Unavailable"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = listener.local_addr().unwrap();
        let body = serde_json::to_vec(&ProvisioningResponse {
            profile: PROVISIONING_PROFILE.into(),
            result: Ok(ProvisioningOutcome::Credential {
                credential_id: target.into(),
                identity_id: "id_test".into(),
                expires_at: chrono::Utc::now().timestamp() + 3600,
                revoked,
            }),
        })
        .unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_at(&mut stream, "/v2/credentials/credential_test/revoke").await;
            let header = format!(
                "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&body).await.unwrap();
            request
        });
        let mut client = ApplicationClient::open(configuration(endpoint))
            .await
            .unwrap();
        let command = ProvisioningCommand::RevokeCredential {
            operation_id: "saved-credential-revoke".into(),
            credential_id: "credential_test".into(),
        };
        let result = client.provision(command.clone()).await;
        if index == 0 {
            assert!(matches!(
                result,
                Ok(ProvisioningOutcome::Credential { revoked: true, .. })
            ));
        } else {
            assert!(matches!(result, Err(ClientError::Protocol)));
        }
        assert_eq!(
            serde_json::from_slice::<ProvisioningCommand>(&peer.await.unwrap()).unwrap(),
            command
        );
    }
}

mod incoming_contacts;
mod user_delegation;
