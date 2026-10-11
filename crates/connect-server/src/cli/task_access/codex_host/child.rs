//! Own the launched child until exit or cancellation. Unix failure cleanup also
//! stops descendants that remain in its inherited process group. This is process
//! ownership, not a sandbox: detached sessions and a killed launcher need an
//! external supervisor. Never signal a group after reaping its leader.
use anyhow::{Context, Result};
use tokio::process::{Child, Command};

pub(super) struct OwnedChild(Option<Child>);

impl OwnedChild {
    pub(super) fn spawn(command: &mut Command) -> Result<Self> {
        #[cfg(unix)]
        command.process_group(0);
        command.kill_on_drop(true);
        Ok(Self(Some(command.spawn().context(
            "Codex could not start; retained intent needs inspection",
        )?)))
    }

    pub(super) fn child(&mut self) -> Result<&mut Child> {
        self.0.as_mut().context("Host child was already released")
    }

    /// Observe exit without reaping before stopping the inherited group. Keeping
    /// the leader waitable reserves its PID until group cleanup has finished.
    /// Other platforms retain direct-child waiting; only macOS is qualified.
    pub(super) async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        let child = self.child()?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if let Some(raw) = child.id() {
            use rustix::process::{waitid, Pid, WaitId, WaitIdOptions};
            let raw = i32::try_from(raw).context("Invalid host process ID")?;
            anyhow::ensure!(raw > 1, "Invalid owned host process ID");
            let pid = Pid::from_raw(raw).context("Missing host process ID")?;
            let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
            loop {
                match waitid(WaitId::Pid(pid), options) {
                    Ok(Some(_)) => break,
                    Ok(None) | Err(rustix::io::Errno::INTR) => {}
                    Err(error) => return Err(error).context("Could not observe owned host exit"),
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Self::stop_for_wait(child).await?;
        }
        child
            .wait()
            .await
            .context("Could not reap owned host child")
    }

    /// Signal before waiting: an unreaped leader reserves its PID/group identity.
    /// A group may already have exited; that is successful cleanup, not an error.
    fn stop_group(child: &Child) -> Result<()> {
        #[cfg(unix)]
        if let Some(raw) = child.id() {
            use rustix::process::{kill_process_group, Pid, Signal};
            let raw = i32::try_from(raw).context("Invalid host process ID")?;
            // PID 1 has special group-signal semantics; it is never our child.
            anyhow::ensure!(raw > 1, "Invalid owned host process group");
            let group = Pid::from_raw(raw).context("Missing host process group")?;
            match kill_process_group(group, Signal::KILL) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(error).context("Could not stop owned host process group"),
            }
        }
        #[cfg(not(unix))]
        let _ = child;
        Ok(())
    }

    async fn stop_for_wait(child: &Child) -> Result<()> {
        let result = Self::stop_group(child);
        #[cfg(target_os = "macos")]
        if result.as_ref().err().is_some_and(|error| {
            error.downcast_ref::<rustix::io::Errno>() == Some(&rustix::io::Errno::PERM)
        }) {
            // Darwin skips zombies during group signalling. Do not accept
            // EPERM for a live or unobservable group: require a bounded native
            // snapshot containing the unreaped leader and only zombie members.
            if let Some(pid) = child.id() {
                if super::group_status::has_only_zombies(pid).await? {
                    return Ok(());
                }
            }
        }
        result
    }

    pub(super) async fn stop_and_reap(&mut self) -> Result<()> {
        let child = self.child()?;
        let group_result = Self::stop_for_wait(child).await;
        // Still stop/reap the direct child if group signalling failed.
        let direct_result = child.kill().await;
        group_result?;
        direct_result.context("Could not reap owned host child")
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.id().is_none() {
                return;
            }
            if let Err(error) = Self::stop_group(&child) {
                tracing::error!(%error, "Host cancellation cleanup failed");
            }
            // Future cancellation cannot await. Signal synchronously, then let
            // the active runtime reap the direct child; kill_on_drop is retained
            // as a fallback if runtime shutdown prevents this waiter running.
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if let Err(error) = child.wait().await {
                        tracing::error!(%error, "Could not reap cancelled host child");
                    }
                });
            }
        }
    }
}
