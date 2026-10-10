//! Private application-owned SQLite, immutable task binding and process lock.
use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

/// Non-secret binding verified against both authenticated public clients.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub task_id: String,
    pub owner_id: String,
    pub conversation_id: String,
}

/// Safe counts suitable for process output; no tokens or message text.
#[derive(Debug, Serialize)]
pub struct Statistics {
    pub task_id: String,
    pub processed_events: u64,
    pub business_effects: u64,
    pub pending_commands: u64,
    pub accepted_commands: u64,
    pub rejected_commands: u64,
    pub cursor_saved: bool,
    pub page_pending: bool,
}

/// Separate from the Connect database and the older Gate 3 example schema.
pub struct TaskStore {
    pub(super) connection: Connection,
    pub(super) binding: Binding,
    pub(super) worker: String,
    _lock: File,
}

impl TaskStore {
    /// Open exclusively; reject changed task/owner/conversation without adopting it.
    pub fn open(path: &Path, binding: Binding) -> Result<Self> {
        let lock = private_options()
            .create(true)
            .truncate(false)
            .open(path.with_extension("task.lock"))?;
        lock.try_lock_exclusive()
            .context("Another runner owns this app state")?;
        if !path.exists() {
            private_options().create_new(true).open(path)?;
        }
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(Duration::from_secs(2))?;
        let schema: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(schema <= 1, "Unsupported task runner state; preserve it");
        if schema == 0 {
            let tables: u64 = connection.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                tables == 0,
                "Existing database is not task runner state; preserve it"
            );
            let tx = connection.transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.execute(
                "INSERT INTO runner_metadata VALUES('binding',?1)",
                [serde_json::to_string(&binding)?],
            )?;
            tx.execute(
                "INSERT INTO runner_metadata VALUES('worker',?1)",
                [uuid::Uuid::new_v4().to_string()],
            )?;
            tx.commit()?;
        }
        let saved: String = connection.query_row(
            "SELECT value FROM runner_metadata WHERE key='binding'",
            [],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            serde_json::from_str::<Binding>(&saved)? == binding,
            "Task runner binding changed; preserve this state and select the matching task"
        );
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let worker = connection.query_row(
            "SELECT value FROM runner_metadata WHERE key='worker'",
            [],
            |r| r.get(0),
        )?;
        Ok(Self {
            connection,
            binding,
            worker,
            _lock: lock,
        })
    }

    /// Read only the app database; the running worker keeps exclusive write ownership.
    pub fn inspect(path: &Path) -> Result<Statistics> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        statistics(&connection)
    }

    /// Consistent app-owned snapshot with counts and no secret material.
    pub fn statistics(&self) -> Result<Statistics> {
        statistics(&self.connection)
    }
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn statistics(c: &Connection) -> Result<Statistics> {
    let tx = c.unchecked_transaction()?;
    let result = statistics_at(&tx)?;
    tx.commit()?;
    Ok(result)
}

fn statistics_at(c: &Connection) -> Result<Statistics> {
    let binding: String = c.query_row(
        "SELECT value FROM runner_metadata WHERE key='binding'",
        [],
        |r| r.get(0),
    )?;
    let binding: Binding = serde_json::from_str(&binding)?;
    Ok(Statistics {
        task_id: binding.task_id,
        processed_events: c.query_row("SELECT COUNT(*) FROM runner_events", [], |r| r.get(0))?,
        business_effects: c.query_row("SELECT COUNT(*) FROM runner_actions", [], |r| r.get(0))?,
        pending_commands: c.query_row(
            "SELECT COUNT(*) FROM runner_commands c WHERE outcome IS NULL AND NOT EXISTS(SELECT 1 FROM runner_rejections r WHERE r.command_key=c.key)",
            [],
            |r| r.get(0),
        )?,
        accepted_commands: c.query_row(
            "SELECT COUNT(*) FROM runner_commands WHERE outcome IS NOT NULL",
            [],
            |r| r.get(0),
        )?,
        rejected_commands: c.query_row(
            "SELECT COUNT(DISTINCT command_key) FROM runner_rejections",
            [],
            |r| r.get(0),
        )?,
        cursor_saved: c.query_row(
            "SELECT EXISTS(SELECT 1 FROM runner_metadata WHERE key='cursor')",
            [],
            |r| r.get(0),
        )?,
        page_pending: c.query_row(
            "SELECT EXISTS(SELECT 1 FROM runner_metadata WHERE key='page')",
            [],
            |r| r.get(0),
        )?,
    })
}
