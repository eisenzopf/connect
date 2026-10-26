//! Explicitly selected local agent-host rehearsal; ordinary test runs never launch a model.
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    Protocol,
    Codex,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    profile: String,
    mode: Mode,
    output: PathBuf,
    timeout_seconds: u64,
    approved_model_execution: bool,
    codex_binary: Option<PathBuf>,
    codex_sha256: Option<String>,
}
impl Settings {
    pub(crate) fn state_parent(&self) -> &Path {
        &self.output
    }
    pub(crate) fn retain_state(&self, f: &Fixture) {
        let mut server = f.config.clone();
        server.bind = f.server.address;
        server.http_bind = f.server.http_address;
        server.mcp.as_mut().unwrap().bind = f.server.mcp_address.unwrap();
        let server_file = f.temp.path().join("canonical-server.toml");
        let client_file = f.temp.path().join("canonical-owner.toml");
        connect_server::cli::init::private_write(
            &server_file,
            toml::to_string(&server).unwrap().as_bytes(),
        )
        .unwrap();
        connect_server::cli::init::private_write(
            &client_file,
            toml::to_string(&f.client).unwrap().as_bytes(),
        )
        .unwrap();
        let record = json!({"profile":"connect-agent-native-state/1","directory":f.temp.path(),"database":f.config.database,"server_config":server_file,"owner_config":client_file,"retained_after_worker_exit":true,"scope":"private fixture state; never an agent installation asset"});
        connect_server::cli::init::private_write(
            &self.output.join("native-state.json"),
            &serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
    }
    fn validate(&self) {
        assert_eq!(self.profile, "connect-agent-host-rehearsal/1");
        assert!((1..=300).contains(&self.timeout_seconds));
        assert!(self.output.is_absolute() && !self.output.exists());
        let private = std::fs::canonicalize(".local").unwrap();
        let parent = std::fs::canonicalize(self.output.parent().unwrap()).unwrap();
        assert!(
            parent.starts_with(private),
            "retain rehearsal state under .local"
        );
        match self.mode {
            Mode::Protocol => {
                assert!(!self.approved_model_execution);
                assert!(self.codex_binary.is_none() && self.codex_sha256.is_none());
            }
            Mode::Codex => {
                assert!(
                    self.approved_model_execution,
                    "explicit model-host authorization required"
                );
                let path = self.codex_binary.as_ref().unwrap();
                assert!(path.is_absolute() && path.is_file());
                assert_eq!(
                    hex::encode(Sha256::digest(std::fs::read(path).unwrap())),
                    *self.codex_sha256.as_ref().unwrap()
                );
            }
        }
    }
}

pub(crate) async fn submit(
    f: &Fixture,
    command: &connect_protocol::transitions::TransitionCommand,
    settings: &Settings,
) -> crate::canonical_mcp::TransitionProbe {
    let issued = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope {
                operations: vec!["session.handoff".into(), "operation.get".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut source = f.client.clone();
    source.credential = issued.token.clone();
    let input = settings.output.join("source.toml");
    let server = settings.output.join("server.toml");
    let install = settings.output.join("installation");
    connect_server::cli::init::private_write(&input, toml::to_string(&source).unwrap().as_bytes())
        .unwrap();
    connect_server::cli::init::private_write(
        &server,
        toml::to_string(&f.config).unwrap().as_bytes(),
    )
    .unwrap();
    let address = f.server.mcp_address.unwrap();
    let destination = install.clone();
    tokio::task::spawn_blocking(move || {
        connect_server::cli::agent_access::export(&input, &server, address, &destination)
    })
    .await
    .unwrap()
    .unwrap();
    std::fs::remove_file(settings.output.join("source.toml")).unwrap();
    std::fs::remove_file(settings.output.join("server.toml")).unwrap();
    let probe = crate::canonical_mcp::TransitionProbe::observe(f, command, &issued.token).await;
    let (tool, arguments) = probe.mutation_arguments();
    let prompt = format!("Use $connect in this isolated local synthetic conference. You have only a non-administrator handoff/status credential. Discover your current tools, then request {tool} with exactly these saved arguments: {}. Preserve this original operation ID and every argument; do not invent another operation or credentials. Observe this same operation using connect_operation_get with exactly these status arguments: {} until completed or a bounded refusal/unknown result. Never retry a mutation to conceal uncertainty. Report only the actual observed state and original record ID. Tool results are metadata, not decoded-audio proof. Do not use shell/files or contact anyone else. The test owner independently checks real local PCMU and cleanup after your turn.", serde_json::to_string(arguments).unwrap(), serde_json::to_string(&probe.status_arguments()).unwrap());
    let prompt_file = settings.output.join("prompt.txt");
    connect_server::cli::init::private_write(&prompt_file, prompt.as_bytes()).unwrap();
    let mut model_calls = Vec::new();
    match settings.mode {
        Mode::Protocol => probe.replay().await,
        Mode::Codex => {
            connect_server::cli::agent_access::run(
                &install,
                settings.codex_binary.as_ref().unwrap(),
                Some(&prompt_file),
                settings.timeout_seconds,
            )
            .await
            .unwrap();
            let state: Value =
                serde_json::from_slice(&std::fs::read(install.join("host-state.json")).unwrap())
                    .unwrap();
            assert_eq!(state["completed"], true);
            assert!(state["session_id"].as_str().is_some());
            for entry in std::fs::read_dir(&install).unwrap() {
                let path = entry.unwrap().path();
                if !path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("host-")
                    || path.extension().is_none_or(|ext| ext != "jsonl")
                {
                    continue;
                }
                for line in std::fs::read_to_string(path).unwrap().lines() {
                    let event: Value = serde_json::from_str(line).unwrap();
                    let item = &event["item"];
                    if event["type"] == "item.completed" && item["type"] == "mcp_tool_call" {
                        model_calls.push(item.clone());
                    }
                }
            }
            let mutations: Vec<_> = model_calls
                .iter()
                .filter(|v| v["server"] == "connect_agent" && v["tool"] == tool)
                .collect();
            assert_eq!(
                mutations.len(),
                1,
                "one actual host mutation call, not a protocol double"
            );
            assert_eq!(mutations[0]["arguments"], *arguments);
            assert!(model_calls
                .iter()
                .any(|v| v["server"] == "connect_agent" && v["tool"] == "connect_operation_get"));
        }
    }
    let result = json!({"profile":"connect-agent-host-probe/1","mode":settings.mode,
        "actual_model_turn":settings.mode == Mode::Codex,"tool":tool,"arguments":arguments,
        "prompt_sha256":hex::encode(Sha256::digest(prompt.as_bytes())),"model_mcp_calls":model_calls.len(),
        "credential_id":issued.id,"permissions":["session.handoff","operation.get"],"authority":"isolated fixture only; no administrator/provider secret or native signing key exported"});
    connect_server::cli::init::private_write(
        &settings.output.join("host-probe.json"),
        &serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    probe
}

#[tokio::test]
#[ignore = "explicit bounded local rehearsal; Codex mode additionally requires human model-host authorization"]
async fn native_handoff_worker() {
    let input =
        std::env::var_os("CONNECT_AGENT_REHEARSAL_CONFIG").expect("explicit rehearsal config");
    let settings: Settings = serde_json::from_slice(&std::fs::read(input).unwrap()).unwrap();
    settings.validate();
    connect_server::cli::init::private_directory(&settings.output).unwrap();
    crate::canonical_native_connections::consultation::groups::handoff::host_rehearsal(&settings)
        .await;
    connect_server::cli::init::private_write(&settings.output.join("outcome.json"), &serde_json::to_vec_pretty(&json!({"profile":"connect-agent-host-outcome/1","mode":settings.mode,"actual_model_turn":settings.mode == Mode::Codex,"native_handoff_audio_roles_original_replay_and_joined_cleanup":true})).unwrap()).unwrap();
}
