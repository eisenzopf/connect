#![allow(clippy::unwrap_used)] // Integration assertions deliberately panic.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use connect_client::{ClientConfig, ConnectClient, NativeAudioSession};
use connect_server::{cli::init, config::ServerConfig, runtime::RunningServer};
use connect_store::Scope;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const SEED: u64 = 0x434f_4e4e_4543_5403;

struct MediaPair {
    app: ConnectClient,
    agent: ConnectClient,
    app_audio: NativeAudioSession,
    agent_audio: NativeAudioSession,
    conversation_id: String,
    session_id: String,
}

async fn open_pair(
    server: &RunningServer,
    app_config: &ClientConfig,
    agent_config: &ClientConfig,
    index: usize,
) -> MediaPair {
    let mut app = ConnectClient::open(app_config.clone()).await.unwrap();
    let mut agent = ConnectClient::open(agent_config.clone()).await.unwrap();
    let create_request = format!("pressure-create-{index}");
    let conversation = app
        .create(&create_request, vec![agent_config.identity_id.clone()])
        .await
        .unwrap();
    let invite_request = format!("pressure-invite-{index}");
    let invited = app
        .session_invite(
            &invite_request,
            &conversation.id,
            &agent_config.identity_id,
            vec!["audio".into()],
        )
        .await
        .unwrap();
    let accept_request = format!("pressure-accept-{index}");
    let accepted = agent
        .session_accept(&accept_request, &conversation.id, &invited.id)
        .await
        .unwrap();
    let mut app_audio = app
        .open_native_audio(&conversation.id, &accepted.id, &accepted.participant_id)
        .await
        .unwrap();
    let mut agent_audio = agent
        .open_native_audio(&conversation.id, &accepted.id, &accepted.initiator_id)
        .await
        .unwrap();
    app.subscribe_native_audio(&app_audio, &agent_audio.publisher_stream_id)
        .await
        .unwrap();
    agent
        .subscribe_native_audio(&agent_audio, &app_audio.publisher_stream_id)
        .await
        .unwrap();

    // The first best-effort datagram can precede the reliable stream-open
    // announcement. Warm the route before measurements begin.
    app_audio.send_pcmu(&[index as u8; 160]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    app_audio.send_pcmu(&[index as u8; 160]).await.unwrap();
    agent_audio.receive(Duration::from_secs(2)).await.unwrap();
    agent_audio
        .send_pcmu(&[index.saturating_add(1) as u8; 160])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(25)).await;
    agent_audio
        .send_pcmu(&[index.saturating_add(1) as u8; 160])
        .await
        .unwrap();
    app_audio.receive(Duration::from_secs(2)).await.unwrap();

    assert_eq!(server.store.installation_id().unwrap().len(), 36);
    MediaPair {
        app,
        agent,
        app_audio,
        agent_audio,
        conversation_id: conversation.id,
        session_id: accepted.id,
    }
}

async fn exercise(mut pair: MediaPair, index: usize, duration: Duration) -> serde_json::Value {
    const ARTIFACT_BYTES: usize = 256 * 1024;
    const CHUNK_BYTES: usize = 32 * 1024;
    let artifact_body = vec![index as u8; ARTIFACT_BYTES];
    let artifact = pair
        .app
        .artifact_begin(
            &format!("pressure-artifact-begin-{index}"),
            &pair.conversation_id,
            &format!("pressure-{index}.bin"),
            "application/octet-stream",
            artifact_body.len() as u64,
            &hex::encode(Sha256::digest(&artifact_body)),
        )
        .await
        .unwrap();
    let mut artifact_offset = 0usize;
    let deadline = Instant::now() + duration;
    let mut sequence = 1u64;
    let mut dropped = 0u64;
    let mut delivered = 0u64;
    let mut latencies = Vec::new();
    while Instant::now() < deadline {
        let tick = Instant::now();
        // Deterministic 3% source loss and 0-100 ms delay variation. Logical
        // Reordered RTP and queue saturation are tested in gate3_media_faults.
        if sequence.is_multiple_of(33) {
            dropped += 1;
        } else {
            let delay = (sequence.wrapping_mul(SEED + index as u64)) % 101;
            if sequence.is_multiple_of(17) {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            let value = sequence as u8;
            let started = Instant::now();
            pair.app_audio.send_pcmu(&[value; 160]).await.unwrap();
            let received = pair
                .agent_audio
                .receive(Duration::from_secs(2))
                .await
                .unwrap();
            latencies.push(started.elapsed().as_micros() as u64);
            assert_eq!(received.payload, vec![value; 160]);
            pair.agent_audio
                .send_pcmu(&[value.wrapping_add(1); 160])
                .await
                .unwrap();
            assert_eq!(
                pair.app_audio
                    .receive(Duration::from_secs(2))
                    .await
                    .unwrap()
                    .payload,
                vec![value.wrapping_add(1); 160]
            );
            delivered += 1;
        }
        if sequence.is_multiple_of(50) {
            let request_id = format!("pressure-message-{index}-{sequence}");
            let body = format!("reliable-control-{sequence}");
            pair.app
                .send(&request_id, &pair.conversation_id, &body)
                .await
                .unwrap();
            if artifact_offset < artifact_body.len() {
                let end = (artifact_offset + CHUNK_BYTES).min(artifact_body.len());
                pair.app
                    .artifact_append(
                        &format!("pressure-artifact-{index}-{artifact_offset}"),
                        &pair.conversation_id,
                        &artifact.id,
                        artifact_offset as u64,
                        &STANDARD.encode(&artifact_body[artifact_offset..end]),
                    )
                    .await
                    .unwrap();
                artifact_offset = end;
            }
        }
        sequence += 1;
        if let Some(remaining) = Duration::from_millis(20).checked_sub(tick.elapsed()) {
            tokio::time::sleep(remaining).await;
        }
    }
    latencies.sort_unstable();
    let p95 = latencies[(latencies.len() * 95 / 100).min(latencies.len() - 1)];
    assert!(p95 <= 50_000, "session {index} p95 forwarding was {p95}µs");

    if artifact_offset == artifact_body.len() {
        pair.app
            .artifact_finish(
                &format!("pressure-artifact-finish-{index}"),
                &pair.conversation_id,
                &artifact.id,
            )
            .await
            .unwrap();
        let first = pair
            .agent
            .artifact_get(&pair.conversation_id, &artifact.id, 0, CHUNK_BYTES as u32)
            .await
            .unwrap();
        assert_eq!(
            STANDARD.decode(first.data).unwrap(),
            artifact_body[..CHUNK_BYTES]
        );
    }

    pair.app.close_native_audio(&pair.app_audio).await.unwrap();
    pair.agent
        .close_native_audio(&pair.agent_audio)
        .await
        .unwrap();
    let end_request = format!("pressure-end-{index}");
    pair.app
        .session_end(
            &end_request,
            &pair.conversation_id,
            &pair.session_id,
            Some("profile-complete".into()),
        )
        .await
        .unwrap();
    pair.app.close().await;
    pair.agent.close().await;
    serde_json::json!({
        "session": index,
        "duration_seconds": duration.as_secs(),
        "delivered_frames": delivered,
        "injected_source_drops": dropped,
        "p95_forwarding_microseconds": p95,
        "reliable_artifact_bytes": artifact_offset,
    })
}

async fn run_profile(concurrency: usize, duration: Duration) -> Vec<serde_json::Value> {
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("state");
    init::initialize(&directory, true, "127.0.0.1:0".parse().unwrap()).unwrap();
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let server = RunningServer::start(config).await.unwrap();
    let mut app = ClientConfig::load(&directory.join("reviews.toml")).unwrap();
    let mut agent = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    let expiry = chrono::Utc::now().timestamp() + duration.as_secs() as i64 + 300;
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
    app.credential_id = app_key.id;
    app.credential = app_key.token;
    agent.endpoint = server.address;
    agent.credential_id = agent_key.id;
    agent.credential = agent_key.token;

    let mut pairs = Vec::new();
    for index in 0..concurrency {
        pairs.push(open_pair(&server, &app, &agent, index).await);
    }
    let tasks = pairs
        .into_iter()
        .enumerate()
        .map(|(index, pair)| tokio::spawn(exercise(pair, index, duration)))
        .collect::<Vec<_>>();
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    server.shutdown().await.unwrap();
    results
}

#[tokio::test]
async fn short_media_pressure_smoke() {
    let results = run_profile(2, Duration::from_secs(2)).await;
    assert_eq!(results.len(), 2);
}

#[tokio::test]
#[ignore = "Gate 3 qualification profile runs one and four sessions for 60 seconds each"]
async fn profile_one_and_four_concurrent_sessions_for_sixty_seconds() {
    let one = run_profile(1, Duration::from_secs(60)).await;
    let four = run_profile(4, Duration::from_secs(60)).await;
    println!(
        "{}",
        serde_json::json!({
            "seed": SEED,
            "arch": std::env::consts::ARCH,
            "logical_cpus": std::thread::available_parallelism().map(usize::from).unwrap_or(0),
            "one_session": one,
            "four_sessions": four,
        })
    );
}
