//! Actual installed Codex discovery without inference or account credentials.
#![allow(clippy::unwrap_used)]
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub struct Runtime {
    child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    bytes: usize,
}
impl Runtime {
    /// Start with private empty account/config roots and a nonworking loopback
    /// model provider. No turn/start request is sent by this fixture.
    pub async fn start(binary: &Path, directory: &Path, bearer: &str) -> Result<Self> {
        let home = directory.join("runtime-home");
        let codex_home = directory.join("runtime-codex-home");
        for path in [&home, &codex_home] {
            std::fs::create_dir_all(path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
        }
        let config = std::fs::read_to_string(directory.join("mcp-override.txt"))?;
        let mut command = Command::new(binary);
        command.args(["app-server", "--stdio", "--strict-config", "-c", &config,
            "-c", "model_provider=\"connect_runtime_probe\"",
            "-c", "model_providers.connect_runtime_probe={name=\"Connect offline discovery\",base_url=\"http://127.0.0.1:9\",wire_api=\"responses\",requires_openai_auth=false}"])
            .env_clear().env("HOME", home).env("CODEX_HOME", codex_home)
            .env("CONNECT_TASK_GRANT", bearer).current_dir(directory)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        let mut child = command.spawn().context("Installed Codex could not start")?;
        let mut runtime = Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            next_id: 0,
            bytes: 0,
        };
        runtime
            .ask(
                "initialize",
                json!({
                    "clientInfo":{"name":"connect_runtime_qualification","version":"1"},
                    "capabilities":{"experimentalApi":true}
                }),
            )
            .await?;
        runtime
            .write(json!({"method":"initialized","params":{}}))
            .await?;
        Ok(runtime)
    }
    async fn write(&mut self, value: Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(&value)?;
        bytes.push(b'\n');
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        Ok(())
    }
    /// Requests are bounded; unexpected server requests cannot silently become
    /// approval answers. Notifications are ignored, never treated as receipts.
    pub async fn ask(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(json!({"id":id,"method":method,"params":params}))
            .await?;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let line = self
                    .output
                    .next_line()
                    .await?
                    .context("Codex closed JSONL")?;
                self.bytes += line.len();
                anyhow::ensure!(
                    line.len() <= 256 * 1024 && self.bytes <= 1024 * 1024,
                    "Codex discovery exceeded the fixture stream budget"
                );
                let value: Value = serde_json::from_str(&line)?;
                anyhow::ensure!(
                    value.get("method").is_none() || value.get("id").is_none(),
                    "Unexpected Codex server request during model-free discovery"
                );
                if value["id"] == id {
                    return Ok::<_, anyhow::Error>(value);
                }
            }
        })
        .await
        .context("Codex discovery request deadline exceeded")?
    }
    pub async fn stop(mut self) {
        self.child.kill().await.unwrap();
        self.child.wait().await.unwrap();
    }
}
