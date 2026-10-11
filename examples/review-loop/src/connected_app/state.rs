//! Application database, independent of Connect's journal and server Store.
use super::{domain::digest, Policy};
use anyhow::{ensure, Context, Result};
use connect_client::ClientConfig;
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

/// Owns one exclusively locked, credential/policy-bound app database.
pub struct AppStore {
    pub(super) connection: Connection,
    _lock: File,
}
/// Content-free operational inspection. No tokens, draft bodies or notes.
#[derive(Debug, Serialize)]
pub struct Statistics {
    pub reviews: u64,
    pub tickets: u64,
    pub pending_commands: u64,
    pub rejected_commands: u64,
    pub unresolved_effects: u64,
    pub waiting_questions: u64,
    pub outgoing_state: Option<String>,
    pub outgoing_task_id: Option<String>,
    pub outgoing_conversation_id: Option<String>,
    pub results_omitted: u64,
    pub queued_events: u64,
    pub denied_events: u64,
    pub conflict_events: u64,
    pub unassigned_events: u64,
    pub results: Vec<BusinessResult>,
}
/// Attributed result metadata; the full business record remains private.
#[derive(Debug, Serialize)]
pub struct BusinessResult {
    pub key: String,
    pub kind: String,
    pub task_id: String,
    pub conversation_id: String,
    pub source_event: String,
    pub input_digest: String,
    pub status: String,
}
impl AppStore {
    /// Validate existing state before changing its schema or journal mode.
    pub fn open(path: &Path, config: &ClientConfig, policy: &Policy) -> Result<Self> {
        let lock_path = path.with_extension("connected-app.lock");
        if lock_path.exists() {
            protected(&lock_path)?;
        }
        let lock = options().create(true).truncate(false).open(&lock_path)?;
        lock.try_lock_exclusive()
            .context("Another connected app owns this state")?;
        if path.exists() {
            protected(path)?;
        } else {
            options().create_new(true).open(path)?;
        }
        let mut c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        c.busy_timeout(Duration::from_secs(2))?;
        let binding = digest(&(
            "connected-review-app/1",
            &config.identity_id,
            &config.credential_id,
            digest(&config.credential)?,
            digest(&std::fs::read(&config.trust_certificate)?)?,
            config.endpoint,
            &config.server_name,
            &config.trust_certificate,
            policy,
        ))?;
        let schema: u32 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(schema <= 1, "Unsupported app schema; preserve state");
        if schema == 0 {
            let n: u64 = c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            ensure!(n == 0, "Not connected-app state; preserve database");
            let tx = c.transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.execute("INSERT INTO app_metadata VALUES('binding',?1)", [&binding])?;
            tx.commit()?;
        }
        let saved: String = c.query_row(
            "SELECT value FROM app_metadata WHERE key='binding'",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            saved == binding,
            "App credential, endpoint or policy changed; preserve matching state"
        );
        c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        Ok(Self {
            connection: c,
            _lock: lock,
        })
    }
    /// Consistent read-only snapshot; does not take over the running worker.
    pub fn inspect(path: &Path) -> Result<Statistics> {
        protected(path)?;
        let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let tx = c.unchecked_transaction()?;
        let schema: u32 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(
            schema == 1,
            "Unsupported connected-app snapshot; preserve state"
        );
        let count = |sql| -> Result<u64> { Ok(tx.query_row(sql, [], |r| r.get(0))?) };
        let mut statement = tx.prepare("SELECT key,kind,task_id,conversation_id,source_event,input_digest,COALESCE(json_extract(payload,'$.decision'),json_extract(payload,'$.status')) FROM app_business ORDER BY key LIMIT 100")?;
        let results = statement
            .query_map([], |r| {
                Ok(BusinessResult {
                    key: r.get(0)?,
                    kind: r.get(1)?,
                    task_id: r.get(2)?,
                    conversation_id: r.get(3)?,
                    source_event: r.get(4)?,
                    input_digest: r.get(5)?,
                    status: r.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let result = Statistics {
            reviews: count("SELECT COUNT(*) FROM app_business WHERE kind='review'")?,
            tickets: count("SELECT COUNT(*) FROM app_business WHERE kind='intake'")?,
            pending_commands: count(
                "SELECT COUNT(*) FROM app_commands WHERE outcome IS NULL AND rejection IS NULL",
            )?,
            rejected_commands: count(
                "SELECT COUNT(*) FROM app_commands WHERE rejection IS NOT NULL",
            )?,
            unresolved_effects: count(
                "SELECT COUNT(*) FROM app_commands WHERE effect_state IN ('pending','uncertain')",
            )?,
            waiting_questions: count(
                "SELECT COUNT(*) FROM app_commands WHERE effect_state='waiting'",
            )?,
            outgoing_state: tx
                .query_row(
                    "SELECT value FROM app_metadata WHERE key='outgoing-state'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .map(|s| serde_json::from_str(&s))
                .transpose()?,
            outgoing_task_id: tx
                .query_row(
                    "SELECT value FROM app_metadata WHERE key='outgoing-task-id'",
                    [],
                    |r| r.get(0),
                )
                .optional()?,
            outgoing_conversation_id: tx
                .query_row(
                    "SELECT value FROM app_metadata WHERE key='outgoing-conversation-id'",
                    [],
                    |r| r.get(0),
                )
                .optional()?,
            results_omitted: count("SELECT MAX(0,COUNT(*)-100) FROM app_business")?,
            queued_events: count("SELECT COUNT(*) FROM app_events WHERE result IS NULL")?,
            denied_events: count("SELECT COUNT(*) FROM app_events WHERE result='denied'")?,
            conflict_events: count("SELECT COUNT(*) FROM app_events WHERE result='conflict'")?,
            unassigned_events: count("SELECT COUNT(*) FROM app_events WHERE result='unassigned'")?,
            results,
        };
        drop(statement);
        tx.commit()?;
        Ok(result)
    }
    pub(super) fn metadata(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT value FROM app_metadata WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub(super) fn save(&self, key: &str, value: &str) -> Result<()> {
        self.connection.execute("INSERT INTO app_metadata VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[key,value])?;
        Ok(())
    }
}
fn options() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}
fn protected(path: &Path) -> Result<()> {
    let m = std::fs::symlink_metadata(path)?;
    ensure!(
        m.is_file() && !m.file_type().is_symlink(),
        "App state must be a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            m.permissions().mode() & 0o077 == 0,
            "App state must be private"
        );
    }
    Ok(())
}
