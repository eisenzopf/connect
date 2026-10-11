//! Actual exec/resume against private fixture endpoints, without model accounts.
#![allow(clippy::unwrap_used)]
use connect_server::cli::init::{private_directory, private_write};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

pub struct Exec {
    pub child: Child,
    pub home: PathBuf,
    output: PathBuf,
    errors: PathBuf,
}
impl Exec {
    pub fn start(
        binary: &Path,
        directory: &Path,
        provider: &str,
        mcp: std::net::SocketAddr,
        bearer: &str,
        session: Option<&str>,
    ) -> Self {
        let home = directory.join("exec-codex-home");
        if !home.exists() {
            private_directory(&home).unwrap();
        }
        let user_home = directory.join("exec-user-home");
        if !user_home.exists() {
            private_directory(&user_home).unwrap();
        }
        let suffix = if session.is_some() {
            "resumed"
        } else {
            "first"
        };
        let output = directory.join(format!("exec-{suffix}.jsonl"));
        let errors = directory.join(format!("exec-{suffix}.stderr"));
        private_write(&output, b"").unwrap();
        private_write(&errors, b"").unwrap();
        let mut command = Command::new(binary);
        command.arg("exec");
        if let Some(id) = session {
            command.args(["resume", id]);
        }
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(directory.join("install.json")).unwrap())
                .unwrap();
        let original = manifest["mcp_uri"].as_str().unwrap();
        let saved = std::fs::read_to_string(directory.join("mcp-override.txt")).unwrap();
        assert_eq!(saved.matches(original).count(), 1);
        // Only this isolated fixture selects the controlled admission listener;
        // the saved installation/grant and production launcher are unchanged.
        let config = saved.replace(original, &format!("http://{mcp}/mcp"));
        command.args(["--json","--ignore-user-config","--skip-git-repo-check","-c",&config,
            "-c","model_provider=\"connect_intent_fixture\"","-c",
            &format!("model_providers.connect_intent_fixture={{name=\"Connect scripted fixture\",base_url=\"{provider}\",wire_api=\"responses\",requires_openai_auth=false,supports_websockets=false,request_max_retries=0,stream_max_retries=0}}"),
            "Use $connect to continue this existing delegated task."])
            .current_dir(directory).env_clear().env("HOME",user_home).env("CODEX_HOME",&home).env("CONNECT_TASK_GRANT",bearer)
            .stdin(Stdio::null()).stdout(Stdio::from(std::fs::OpenOptions::new().append(true).open(&output).unwrap()))
            .stderr(Stdio::from(std::fs::OpenOptions::new().append(true).open(&errors).unwrap())).kill_on_drop(true);
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        Self {
            child: command.spawn().unwrap(),
            home,
            output,
            errors,
        }
    }
    pub fn events(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    pub fn session(&self) -> String {
        self.events()
            .iter()
            .find(|event| event["type"] == "thread.started")
            .unwrap()["thread_id"]
            .as_str()
            .unwrap()
            .into()
    }
    pub async fn kill(&mut self) {
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
    }
    pub async fn finish(&mut self) {
        let status = tokio::time::timeout(Duration::from_secs(20), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(
            status.success(),
            "Actual Codex failed: {}",
            std::fs::read_to_string(&self.errors).unwrap()
        );
        assert!(self
            .events()
            .iter()
            .any(|event| event["type"] == "turn.completed"));
    }
    pub async fn rejected(&mut self) {
        let status = tokio::time::timeout(Duration::from_secs(20), self.child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(
            !status.success(),
            "Revoked runtime must fail its required MCP startup"
        );
    }
}

/// Read canonical rollout bytes from disk while MCP admission is still paused.
pub fn saved_call(home: &Path) -> Value {
    fn visit(path: &Path, found: &mut Vec<Value>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, found);
            } else if path.extension().is_some_and(|s| s == "jsonl") {
                let text = std::fs::read_to_string(path).unwrap();
                assert!(text.len() <= 1024 * 1024);
                for line in text.lines() {
                    let event: Value = serde_json::from_str(line).unwrap();
                    let payload = &event["payload"];
                    let item = payload.get("item").unwrap_or(payload);
                    if event["type"] == "response_item"
                        && item["type"] == "custom_tool_call"
                        && item["call_id"] == "intent-call"
                    {
                        found.push(item.clone());
                    }
                }
            }
        }
    }
    let mut found = vec![];
    visit(&home.join("sessions"), &mut found);
    assert_eq!(found.len(), 1);
    found.pop().unwrap()
}
