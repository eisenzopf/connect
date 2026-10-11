//! Darwin skips zombies in killpg and can return EPERM for an exited group.
//! Accept that result only when a bounded OS snapshot proves the reserved leader
//! is present and every remaining member is a zombie. This never signals a PID.
use anyhow::{Context, Result};
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

const MAX_SNAPSHOT: u64 = 64 * 1024;

pub(super) async fn has_only_zombies(group: u32) -> Result<bool> {
    anyhow::ensure!(group > 1, "Invalid owned host process group");
    let mut child = Command::new("/bin/ps")
        .args(["-g", &group.to_string(), "-o", "pid=,pgid=,stat="])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("Could not inspect owned host process group")?;
    let mut stream = child
        .stdout
        .take()
        .context("Missing group snapshot")?
        .take(MAX_SNAPSHOT + 1);
    let (bytes, status) = tokio::time::timeout(Duration::from_secs(1), async {
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).await?;
        Ok::<_, anyhow::Error>((bytes, child.wait().await?))
    })
    .await
    .context("Owned host group inspection timed out")??;
    anyhow::ensure!(
        bytes.len() <= MAX_SNAPSHOT as usize,
        "Owned host group snapshot exceeded its bound"
    );
    anyhow::ensure!(
        status.success(),
        "Owned host process group could not be observed"
    );
    let snapshot = std::str::from_utf8(&bytes).context("Invalid owned host group snapshot")?;
    let mut leader = false;
    for row in snapshot.lines() {
        let mut fields = row.split_whitespace();
        let pid: u32 = fields.next().context("Missing group member PID")?.parse()?;
        let pgid: u32 = fields
            .next()
            .context("Missing group member group")?
            .parse()?;
        let state = fields.next().context("Missing group member state")?;
        anyhow::ensure!(
            fields.next().is_none() && pgid == group && pid > 1,
            "Invalid owned group member"
        );
        if !state.starts_with('Z') {
            return Ok(false);
        }
        leader |= pid == group;
    }
    Ok(leader)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_snapshot_rejects_live_group_and_accepts_its_unreaped_zombie() -> Result<()> {
        use rustix::process::{waitid, Pid, WaitId, WaitIdOptions};
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut child = Command::new("python3")
            .args([
                "-c",
                "import sys; print('ready',flush=True); sys.stdin.read()",
            ])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let pid = child.id().context("Missing fixture PID")?;
        let mut stdout = BufReader::new(child.stdout.take().context("Missing fixture stdout")?);
        let mut ready = String::new();
        tokio::time::timeout(Duration::from_secs(1), stdout.read_line(&mut ready)).await??;
        assert_eq!(ready, "ready\n");
        assert!(!has_only_zombies(pid).await?);
        drop(child.stdin.take());
        let pid_value = Pid::from_raw(i32::try_from(pid)?).context("Invalid fixture PID")?;
        tokio::time::timeout(Duration::from_secs(1), async {
            while waitid(
                WaitId::Pid(pid_value),
                WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
            )?
            .is_none()
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        assert!(has_only_zombies(pid).await?);
        assert!(child.wait().await?.success());
        Ok(())
    }
}
