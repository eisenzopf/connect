//! The separate audio rehearsal uses an existing task leg through public SDKs.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/codex_child.rs"]
mod owned_runtime;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use connect_server::cli::init::private_write;
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;

fn command(
    root: &Path,
    config: &Path,
    output: &Path,
    f: &media::Fixture,
    session: Option<&str>,
    remote_end: bool,
) -> Command {
    let mut command = Command::new("python3");
    command.env_clear().env("PYTHONDONTWRITEBYTECODE", "1");
    for key in ["PATH", "HOME", "TMPDIR", "USER", "LANG"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .arg(root.join("scripts/live/native-voice.py"))
        .arg("--allow-live-provider")
        .arg("--config")
        .arg(config)
        .arg("--conversation")
        .arg(&f.h.cid)
        .arg("--assistant")
        .arg(&f.h.peer_id)
        .arg("--scenario")
        .arg(output.parent().unwrap().join("scenario.json"))
        .arg("--output")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(output.parent().unwrap().join("stdout.log")).unwrap())
        .stderr(std::fs::File::create(output.parent().unwrap().join("stderr.log")).unwrap());
    if remote_end {
        command.arg("--expect-remote-end");
    }
    if let Some(session) = session {
        command
            .arg("--task-id")
            .arg(&f.task.id)
            .arg("--session-id")
            .arg(session);
    } else {
        command
            .arg("--assistant-config")
            .arg(output.parent().unwrap().join("peer.toml"));
    }
    if let Some(runtime) = std::env::var_os("CONNECT_TEST_RUNTIME_ROOT") {
        command.arg("--runtime-root").arg(runtime);
    } else {
        command
            .arg("--executable")
            .arg(env!("CARGO_BIN_EXE_connect"));
    }
    command
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_voice_script_uses_prepared_leg_and_keeps_task_and_context_after_hangup() {
    task_capture(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_voice_script_accepts_expected_remote_end_without_cancelling_task() {
    task_capture(true).await;
}

fn scenario(root: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(root.join(".local")).unwrap();
    let temp = tempfile::Builder::new()
        .prefix("task-voice-rehearsal-")
        .tempdir_in(root.join(".local"))
        .unwrap()
        .keep();
    eprintln!("Private rehearsal evidence: {}", temp.display());
    // A tiny ordinary PCM WAV; no microphone, TTS account or model service.
    let wav = temp.as_path().join("prompt.wav");
    let samples = [0xe8_u8, 0x03].repeat(160);
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(36_u32 + samples.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    for value in [1_u16, 1_u16] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [8000_u32, 16000_u32] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    for value in [2_u16, 16_u16] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(samples.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&samples);
    private_write(&wav, &bytes).unwrap();
    private_write(
        &temp.as_path().join("scenario.json"),
        br#"{"turns":[{"wav":"prompt.wav","wait_seconds":0.1}]}"#,
    )
    .unwrap();
    temp
}

async fn task_capture(remote_end: bool) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let temp = scenario(&root);
    let mut fake = gate3_support::FakeVapi::start_rehearsal(remote_end).await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    let mut definition = f.h.definition();
    let mut permissions = media::Fixture::permissions();
    permissions.push(TaskPermission::SessionEnd);
    definition.scope.operations = permissions.clone();
    f.task = native_tasks::record(
        f.h.owner
            .task(TaskCommand::Create {
                request_id: "rehearsal-task".into(),
                definition,
            })
            .await
            .unwrap(),
    );
    call_setup::route(&f);
    f.h.owner
        .send("rehearsal-context", &f.h.cid, "TASK-VOICE-RETAINED-SOURCE")
        .await
        .unwrap();
    let (_, config) = f.grant("rehearsal-native", 240, permissions).await;
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let session = f.session(&mut client, "rehearsal-session").await;
    client.close().await;
    let path = temp.as_path().join("native.toml");
    private_write(&path, toml::to_string(&config).unwrap().as_bytes()).unwrap();
    let api = fake.api_base.clone();
    let provider =
        tokio::task::spawn_blocking(move || Arc::new(Vapi::new("fake-private-key", &api).unwrap()))
            .await
            .unwrap();
    f.h.server
        .prepare_vapi_task_session(
            &session.id,
            call_setup::definition(&f),
            &temp.as_path().join("setup"),
            provider,
        )
        .await
        .unwrap();
    let output = temp.as_path().join("capture");
    let mut process = owned_runtime::OwnedRuntime::spawn(&mut command(
        &root,
        &path,
        &output,
        &f,
        Some(&session.id),
        remote_end,
    ))
    .unwrap();
    let post = fake
        .next_within("create-call", Duration::from_secs(45))
        .await;
    let tools = call_setup::mcp(&post["body"]).await;
    let recall = tools
        .call_tool(
            CallToolRequestParams::new("connect_recall")
                .with_arguments(json!({"limit":100}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_ne!(recall.is_error, Some(true));
    assert!(recall
        .structured_content
        .unwrap()
        .to_string()
        .contains("TASK-VOICE-RETAINED-SOURCE"));
    tools.cancel().await.unwrap();
    fake.release_call();
    let status = tokio::time::timeout(Duration::from_secs(45), process.wait())
        .await
        .unwrap()
        .unwrap();
    let error = std::fs::read_to_string(temp.as_path().join("stderr.log")).unwrap();
    assert!(status.success(), "rehearsal failed: {error}");
    assert!(!error.contains(&config.credential));
    assert!(!std::fs::read_to_string(temp.join("stdout.log"))
        .unwrap()
        .contains(&config.credential));
    let result: Value =
        serde_json::from_slice(&std::fs::read(output.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["result"], "captured");
    assert_eq!(result["task_id"], f.task.id);
    assert_eq!(result["remote_end_observed"], remote_end);
    assert!(result["received_frames"].as_u64().unwrap() > 0);
    let context = std::fs::read_to_string(output.join("context.json")).unwrap();
    assert!(context.contains("TASK-VOICE-RETAINED-SOURCE"));
    assert_eq!(
        f.h.owner
            .session_get(&f.h.cid, &session.id)
            .await
            .unwrap()
            .state,
        "ended"
    );
    let task = native_tasks::record(
        f.h.owner
            .task(TaskCommand::Get {
                task_id: f.task.id.clone(),
            })
            .await
            .unwrap(),
    );
    assert_eq!(task.conversation_id, f.h.cid);
    assert!(matches!(
        task.state,
        TaskState::Ready | TaskState::Running | TaskState::Waiting
    ));
    f.h.server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ordinary_voice_script_still_invites_accepts_and_ends_its_own_leg() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let temp = scenario(&root);
    let mut fake = gate3_support::FakeVapi::start_rehearsal(false).await;
    let mut f = media::Fixture::start_with_vapi(Some(fake.adapter.clone())).await;
    call_setup::route(&f);
    let config = temp.join("human.toml");
    private_write(
        &config,
        toml::to_string(&f.h.owner_config).unwrap().as_bytes(),
    )
    .unwrap();
    private_write(
        &temp.join("peer.toml"),
        toml::to_string(&f.peer_config).unwrap().as_bytes(),
    )
    .unwrap();
    let output = temp.join("capture");
    let mut process =
        owned_runtime::OwnedRuntime::spawn(&mut command(&root, &config, &output, &f, None, false))
            .unwrap();
    fake.next_within("create-call", Duration::from_secs(45))
        .await;
    fake.release_call();
    let status = tokio::time::timeout(Duration::from_secs(45), process.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(
        status.success(),
        "Inspect retained private rehearsal evidence"
    );
    let result: Value =
        serde_json::from_slice(&std::fs::read(output.join("result.json")).unwrap()).unwrap();
    let intent: Value =
        serde_json::from_slice(&std::fs::read(output.join("intent.json")).unwrap()).unwrap();
    assert_eq!(result["result"], "captured");
    assert!(result["task_id"].is_null());
    assert!(result["received_frames"].as_u64().unwrap() > 0);
    assert_eq!(
        f.h.owner
            .session_get(&f.h.cid, intent["session_id"].as_str().unwrap())
            .await
            .unwrap()
            .state,
        "ended"
    );
    f.h.server.shutdown().await.unwrap();
}
