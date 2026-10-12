//! Fixtures use the same unreaped-leader ownership as the packaged launcher.
//! Killing only the CLI leaves its inherited tool workers alive after a test.
#![allow(dead_code)] // Discovery stops a live host; exec fixtures also await normal exit.
#[path = "../../src/cli/task_access/codex_host/child.rs"]
mod child;
#[cfg(target_os = "macos")]
#[path = "../../src/cli/task_access/codex_host/group_status.rs"]
mod group_status;
use anyhow::Result;
use tokio::process::{Child, Command};

pub struct OwnedRuntime {
    child: child::OwnedChild,
    group: u32,
}
impl OwnedRuntime {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        let mut child = child::OwnedChild::spawn(command)?;
        let group = child
            .child()?
            .id()
            .ok_or_else(|| anyhow::anyhow!("Missing owned group"))?;
        Ok(Self { child, group })
    }
    pub fn io(&mut self) -> Result<&mut Child> {
        self.child.child()
    }
    pub async fn stop(&mut self) -> Result<()> {
        self.child.stop_and_reap().await?;
        self.verify_stopped().await
    }
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        let status = self.child.wait().await?;
        self.verify_stopped().await?;
        Ok(status)
    }
    /// Read-only postcondition: no signals are sent after the leader is reaped.
    /// A reused group would cause a conservative test failure, never be killed.
    async fn verify_stopped(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                Command::new("/bin/ps")
                    .args(["-g", &self.group.to_string(), "-o", "stat="])
                    .env_clear()
                    .kill_on_drop(true)
                    .output(),
            )
            .await??;
            anyhow::ensure!(
                output.stdout.len() <= 65536,
                "Oversized stopped-group observation"
            );
            anyhow::ensure!(
                matches!(output.status.code(), Some(0 | 1)),
                "Failed stopped-group observation"
            );
            anyhow::ensure!(
                output.stderr.is_empty(),
                "Stopped-group observation returned diagnostics"
            );
            let states = std::str::from_utf8(&output.stdout)?;
            anyhow::ensure!(
                states
                    .split_whitespace()
                    .all(|state| state.starts_with('Z')),
                "Installed host retained live workers after owned cleanup"
            );
        }
        Ok(())
    }
}
