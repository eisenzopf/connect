//! Public task reads plus a consistent, private application checkpoint snapshot.
use super::state::{statistics_at, Binding, Statistics};
use anyhow::Result;
use connect_client::{task_diagnostics::TaskDiagnostics, ClientConfig, ConnectClient};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{fs::File, io, path::Path, time::Duration};

/// An OS lock is an ownership observation, never provider/model liveness.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkerState {
    Unconfigured,
    NotStarted,
    LockHeld,
    Stopped,
    StoppedWithPendingWork,
}

/// Safe local snapshot. No stored commands, outcomes, message bytes or secrets.
#[derive(Debug, Serialize)]
pub struct WorkerObservation {
    pub state: WorkerState,
    pub checkpoint_sequence: Option<u64>,
    pub statistics: Option<Statistics>,
}

/// Reference-client report; Connect data comes exclusively from the public SDK.
#[derive(Debug, Serialize)]
pub struct Report {
    pub task: TaskDiagnostics,
    pub runner: WorkerObservation,
}

/// Inspect only the selected application's state and authenticated task. Never
/// creates/migrates a database, acquires writer ownership or resumes any work.
pub async fn inspect(
    config: &Path,
    task_id: &str,
    state: Option<&Path>,
    limit: u32,
) -> Result<Report> {
    let mut client = ConnectClient::open(ClientConfig::load(config)?).await?;
    let result = async {
        let first = client.negotiate_task(task_id).await?;
        let binding = Binding {
            task_id: first.id,
            owner_id: first.owner_id,
            conversation_id: first.conversation_id,
        };
        let path = state.map(Path::to_path_buf);
        let selected = binding.clone();
        let runner =
            tokio::task::spawn_blocking(move || local(path.as_deref(), &selected)).await??;
        let mut task = client
            .task_diagnostics(task_id, &Default::default(), limit)
            .await?;
        anyhow::ensure!(
            task.task.conversation_id == binding.conversation_id,
            "Task binding changed"
        );
        task.changed_during_sample |= first.revision != task.task.revision;
        Ok(Report { task, runner })
    }
    .await;
    client.close().await;
    result
}

fn local(path: Option<&Path>, binding: &Binding) -> Result<WorkerObservation> {
    let empty = |state| WorkerObservation {
        state,
        checkpoint_sequence: None,
        statistics: None,
    };
    let Some(path) = path else {
        return Ok(empty(WorkerState::Unconfigured));
    };
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(empty(WorkerState::NotStarted))
        }
        other => {
            other?;
        }
    }
    private_file(path)?;
    let lock_path = path.with_extension("task.lock");
    private_file(&lock_path)?;
    let lock = File::open(lock_path)?;
    let held = match FileExt::try_lock_shared(&lock) {
        Ok(()) => {
            FileExt::unlock(&lock)?;
            false
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => true,
        Err(error) => return Err(error.into()),
    };
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(Duration::from_secs(2))?;
    let tx = connection.unchecked_transaction()?;
    let schema: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(schema == 1, "Unsupported runner state; preserve it");
    let saved: String = tx.query_row(
        "SELECT substr(value,1,65537) FROM runner_metadata WHERE key='binding'",
        [],
        |r| r.get(0),
    )?;
    anyhow::ensure!(saved.len() <= 65536, "Runner binding exceeds its bound");
    let saved = serde_json::from_str::<Binding>(&saved)
        .map_err(|_| anyhow::anyhow!("Invalid runner binding"))?;
    anyhow::ensure!(
        saved == *binding,
        "Runner binding does not match the authenticated task"
    );
    let statistics = statistics_at(&tx)?;
    let sequence: Option<String> = tx
        .query_row(
            "SELECT substr(value,1,21) FROM runner_metadata WHERE key='sequence'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let checkpoint_sequence = sequence
        .map(|value| {
            anyhow::ensure!(value.len() <= 20, "Invalid runner checkpoint");
            let parsed = value
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("Invalid runner checkpoint"))?;
            anyhow::ensure!(parsed.to_string() == value, "Invalid runner checkpoint");
            Ok::<_, anyhow::Error>(parsed)
        })
        .transpose()?;
    let maximum: Option<u64> =
        tx.query_row("SELECT MAX(sequence) FROM runner_events", [], |r| r.get(0))?;
    anyhow::ensure!(
        checkpoint_sequence.unwrap_or(0) == maximum.unwrap_or(0),
        "Runner checkpoint does not match committed events"
    );
    tx.commit()?;
    let pending = statistics.pending_commands > 0 || statistics.page_pending;
    Ok(WorkerObservation {
        state: if held {
            WorkerState::LockHeld
        } else if pending {
            WorkerState::StoppedWithPendingWork
        } else {
            WorkerState::Stopped
        },
        checkpoint_sequence,
        statistics: Some(statistics),
    })
}

fn private_file(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        meta.file_type().is_file(),
        "Runner state must be a regular private file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "Runner state must have private permissions"
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
