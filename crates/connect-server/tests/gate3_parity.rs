#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{AvailabilityState, Operation, Outcome, Response};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use sha2::{Digest, Sha256};

async fn http_operation(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    token: &str,
    operation: &Operation,
) -> Outcome {
    let response = client
        .post(format!("http://{address}/v1/operations"))
        .bearer_auth(token)
        .json(operation)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    response.json::<Response>().await.unwrap().result.unwrap()
}

#[tokio::test]
async fn gate_three_state_is_shared_by_native_quic_and_http() {
    let _ = tracing_subscriber::fmt::try_init();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    // TLS/proxy initialization can block on host services. Keep it off the
    // executor and complete it before opening live QUIC peers; only loopback
    // HTTP participates in this account-free fixture.
    let http_client = tokio::task::spawn_blocking(|| {
        reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
    })
    .await
    .unwrap()
    .unwrap();
    let server = RunningServer::start_local(config, Some("127.0.0.1:0".parse().unwrap()), false)
        .await
        .unwrap();
    let http = server.http_address.unwrap();
    let expiry = chrono::Utc::now().timestamp() + 3600;
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let mut agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let app_key = server
        .store
        .issue(
            &app.identity_id,
            Scope::communications(vec![agent.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    let agent_key = server
        .store
        .issue(
            &agent.identity_id,
            Scope::communications(vec![app.identity_id.clone()]),
            expiry,
        )
        .unwrap();
    app.endpoint = server.address;
    app.credential = app_key.token.clone();
    app.credential_id = app_key.id;
    agent.endpoint = server.address;
    agent.credential = agent_key.token.clone();
    agent.credential_id = agent_key.id;
    let mut app_native = ConnectClient::open(app.clone()).await.unwrap();
    let mut agent_native = ConnectClient::open(agent).await.unwrap();

    let conversation = app_native
        .create(
            "g3-parity-create",
            vec![
                agent_native
                    .resolve("connect:agent@local")
                    .await
                    .unwrap()
                    .id,
            ],
        )
        .await
        .unwrap();
    app_native
        .send(
            "g3-parity-send",
            &conversation.id,
            "Keep this context when the audio session starts.",
        )
        .await
        .unwrap();
    let invited = http_operation(
        &http_client,
        http,
        &app_key.token,
        &Operation::SessionInvite {
            request_id: "g3-parity-invite".into(),
            conversation_id: conversation.id.clone(),
            participant_id: agent_native
                .resolve("connect:agent@local")
                .await
                .unwrap()
                .id,
            media: vec!["audio".into()],
        },
    )
    .await;
    let session = match invited {
        Outcome::Session(value) => value,
        _ => panic!("session outcome"),
    };
    let active = agent_native
        .session_accept("g3-parity-accept", &conversation.id, &session.id)
        .await
        .unwrap();
    assert_eq!(active.state, "accepted");
    assert_eq!(active.conversation_id, conversation.id);
    let mut app_audio = app_native
        .open_native_audio(&conversation.id, &session.id, &active.participant_id)
        .await
        .unwrap();
    let mut agent_audio = agent_native
        .open_native_audio(&conversation.id, &session.id, &active.initiator_id)
        .await
        .unwrap();
    app_native
        .subscribe_native_audio(&app_audio, &agent_audio.publisher_stream_id)
        .await
        .unwrap();
    agent_native
        .subscribe_native_audio(&agent_audio, &app_audio.publisher_stream_id)
        .await
        .unwrap();
    for value in 0u8..4 {
        app_audio.send_pcmu(&[value; 160]).await.unwrap();
        if value == 0 {
            // The first best-effort datagram causes the server to allocate and
            // announce the subscriber stream. It may precede that reliable
            // announcement, just like an initial RTP packet may be lost.
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            app_audio.send_pcmu(&[value; 160]).await.unwrap();
        }
        let received = agent_audio
            .receive(std::time::Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(received.payload, vec![value; 160]);
        agent_audio
            .send_pcmu(&[value.saturating_add(10); 160])
            .await
            .unwrap();
        if value == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            agent_audio
                .send_pcmu(&[value.saturating_add(10); 160])
                .await
                .unwrap();
        }
        let returned = app_audio
            .receive(std::time::Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(returned.payload, vec![value.saturating_add(10); 160]);
    }
    let ready = app_native
        .session_get(&conversation.id, &session.id)
        .await
        .unwrap();
    assert_eq!(ready.state, "ready");

    let availability = agent_native
        .availability_set(
            "g3-parity-available",
            AvailabilityState::Available,
            Some("native-quic".into()),
            expiry,
        )
        .await
        .unwrap();
    assert_eq!(availability.state, "available");
    let observed = http_operation(
        &http_client,
        http,
        &app_key.token,
        &Operation::AvailabilityGet {
            identity_id: availability.identity_id,
        },
    )
    .await;
    assert!(
        matches!(observed, Outcome::Availability(value) if value.preferred_route.as_deref() == Some("native-quic"))
    );

    let bytes = b"shared over the reliable artifact surface";
    let artifact = match http_operation(
        &http_client,
        http,
        &app_key.token,
        &Operation::ArtifactBegin {
            request_id: "g3-parity-artifact-begin".into(),
            conversation_id: conversation.id.clone(),
            name: "review.txt".into(),
            media_type: "text/plain".into(),
            size: bytes.len() as u64,
            sha256: hex::encode(Sha256::digest(bytes)),
            expires_at: None,
        },
    )
    .await
    {
        Outcome::Artifact(value) => value,
        _ => panic!("artifact outcome"),
    };
    let appended_response = http_client
        .put(format!(
            "http://{http}/v1/conversations/{}/artifacts/{}",
            conversation.id, artifact.id
        ))
        .bearer_auth(&app_key.token)
        .query(&[("request_id", "g3-parity-artifact-append"), ("offset", "0")])
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert!(appended_response.status().is_success());
    let appended = match appended_response
        .json::<Response>()
        .await
        .unwrap()
        .result
        .unwrap()
    {
        Outcome::Artifact(value) => value,
        other => panic!("artifact append outcome, got {other:?}"),
    };
    assert_eq!(appended.received, bytes.len() as u64);
    http_operation(
        &http_client,
        http,
        &app_key.token,
        &Operation::ArtifactFinish {
            request_id: "g3-parity-artifact-finish".into(),
            conversation_id: conversation.id.clone(),
            artifact_id: artifact.id.clone(),
        },
    )
    .await;
    let chunk = agent_native
        .artifact_get(&conversation.id, &artifact.id, 0, 32768)
        .await
        .unwrap();
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), bytes);
    assert!(chunk.eof);
    let binary = http_client
        .get(format!(
            "http://{http}/v1/conversations/{}/artifacts/{}",
            conversation.id, artifact.id
        ))
        .bearer_auth(&agent_key.token)
        .query(&[("offset", "0"), ("limit", "32768")])
        .send()
        .await
        .unwrap();
    assert!(binary.status().is_success());
    assert_eq!(binary.headers()["x-connect-eof"], "true");
    assert_eq!(binary.bytes().await.unwrap(), bytes.as_slice());
    let denied = http_client
        .get(format!(
            "http://{http}/v1/conversations/{}/artifacts/{}?offset=0&limit=32768",
            conversation.id, artifact.id
        ))
        .bearer_auth("invalid")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);

    let context = app_native
        .context_view(&conversation.id, 100)
        .await
        .unwrap();
    assert_eq!(context.conversation_id, conversation.id);
    assert!(context
        .items
        .iter()
        .any(|item| { item.body == "Keep this context when the audio session starts." }));
    assert!(context
        .items
        .iter()
        .any(|item| item.kind == "session_accepted"));
    assert!(context
        .items
        .iter()
        .any(|item| item.kind == "session_ready"));
    assert!(context
        .items
        .iter()
        .any(|item| item.kind == "artifact_ready"));

    app_native.close_native_audio(&app_audio).await.unwrap();
    agent_native.close_native_audio(&agent_audio).await.unwrap();
    let interrupted = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let current = app_native
                .session_get(&conversation.id, &session.id)
                .await
                .unwrap();
            if current.state == "accepted" {
                break current;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(interrupted.state, "accepted");
    app_native
        .session_end(
            "g3-parity-end",
            &conversation.id,
            &session.id,
            Some("completed".into()),
        )
        .await
        .unwrap();
    app_native.close().await;
    agent_native.close().await;
    server.shutdown().await.unwrap();
}
