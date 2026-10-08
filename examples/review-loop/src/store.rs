//! App-owned checkpoints. This database is never the Connect server database.
use anyhow::{Context, Result};
use connect_client::{Event, Operation, Outcome};
use fs2::FileExt;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

pub struct WorkStore {
    connection: Connection,
    _lock: File,
}
#[derive(Serialize)]
pub struct Statistics {
    pub initiated_conversation: Option<String>,
    pub processed_events: u64,
    pub business_effects: u64,
    pub pending_commands: u64,
    pub accepted_commands: u64,
    pub cursor_saved: bool,
}
#[derive(Clone)]
pub struct Command {
    pub key: String,
    pub operation: Operation,
    pub outcome: Option<Outcome>,
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
impl WorkStore {
    pub fn open(path: &Path, identity: &str) -> Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).read(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(path.with_extension("work.lock"))?;
        lock.try_lock_exclusive()
            .context("Another example process owns this work database; stop it first")?;
        if !path.exists() {
            let mut fresh = OpenOptions::new();
            fresh.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                fresh.mode(0o600);
            }
            fresh.open(path)?;
        }
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let schema: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(
            schema <= 1,
            "Unsupported example work schema; preserve it and use a compatible example binary"
        );
        if schema == 0 {
            let tx = connection.transaction()?;
            tx.execute_batch("CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE processed_events(id TEXT PRIMARY KEY); CREATE TABLE business_effects(message_id TEXT PRIMARY KEY,kind TEXT NOT NULL); CREATE TABLE commands(sequence INTEGER PRIMARY KEY AUTOINCREMENT,key TEXT NOT NULL UNIQUE,operation TEXT NOT NULL,outcome TEXT); PRAGMA user_version=1;")?;
            tx.execute("INSERT INTO metadata VALUES('identity',?1)", [identity])?;
            tx.commit()?;
        }
        let stored: String =
            connection.query_row("SELECT value FROM metadata WHERE key='identity'", [], |r| {
                r.get(0)
            })?;
        anyhow::ensure!(
            stored == identity,
            "Work state belongs to a different identity; select a new work file"
        );
        Ok(Self {
            connection,
            _lock: lock,
        })
    }
    /// This is a public inspection of app-owned state, never server state.
    pub fn inspect(path: &Path) -> Result<Statistics> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        statistics(&connection)
    }
    pub fn statistics(&self) -> Result<Statistics> {
        statistics(&self.connection)
    }
    pub fn cursor(&self) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row("SELECT value FROM metadata WHERE key='cursor'", [], |r| {
                r.get(0)
            })
            .optional()?)
    }
    pub fn save_cursor(&self, cursor: Option<&str>) -> Result<()> {
        if let Some(cursor) = cursor {
            self.connection.execute("INSERT INTO metadata VALUES('cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[cursor])?;
        }
        Ok(())
    }
    /// INSERT OR IGNORE preserves the original logical ID and inputs on restart.
    pub fn prepare(&self, key: &str, operation: Operation) -> Result<Command> {
        self.connection.execute(
            "INSERT OR IGNORE INTO commands(key,operation) VALUES(?1,?2)",
            params![key, serde_json::to_string(&operation)?],
        )?;
        self.command(key)?.context("Prepared command is missing")
    }
    pub fn command(&self, key: &str) -> Result<Option<Command>> {
        let row: Option<(String, Option<String>)> = self
            .connection
            .query_row(
                "SELECT operation,outcome FROM commands WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(op, outcome)| {
            Ok(Command {
                key: key.into(),
                operation: serde_json::from_str(&op)?,
                outcome: outcome.map(|s| serde_json::from_str(&s)).transpose()?,
            })
        })
        .transpose()
    }
    pub fn pending(&self) -> Result<Vec<Command>> {
        let mut statement = self.connection.prepare(
            "SELECT key FROM commands WHERE outcome IS NULL ORDER BY sequence LIMIT 100",
        )?;
        let keys = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        keys.into_iter()
            .map(|key| self.command(&key)?.context("Pending command is missing"))
            .collect()
    }
    pub fn accepted(&self, key: &str, outcome: &Outcome) -> Result<()> {
        anyhow::ensure!(
            self.connection.execute(
                "UPDATE commands SET outcome=?1 WHERE key=?2",
                params![serde_json::to_string(outcome)?, key]
            )? == 1,
            "Unknown command acceptance"
        );
        Ok(())
    }
    /// Effect, reply command, handled ID and cursor commit together. A duplicate
    /// event cannot repeat the local business action even after cursor replay.
    pub fn handle(&mut self, event: &Event, identity: &str, agent: bool) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO processed_events VALUES(?1)",
            [&event.id],
        )?;
        if inserted == 1 {
            if let Some(message) = &event.message {
                if message.sender != identity {
                    let reply = if !agent && message.body == "ask_pending_draft" {
                        Some(("draft_lookup", "pending_draft:draft-001".to_owned()))
                    } else if agent {
                        message
                            .body
                            .strip_prefix("review_draft:")
                            .filter(|draft| {
                                !draft.is_empty()
                                    && draft.len() <= 64
                                    && draft
                                        .chars()
                                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                            })
                            .map(|draft| ("draft_review", format!("review_complete:{draft}")))
                    } else {
                        None
                    };
                    if let Some((kind, body)) = reply {
                        #[cfg(feature = "test-support")]
                        fault("before-effect");
                        if tx.execute(
                            "INSERT OR IGNORE INTO business_effects VALUES(?1,?2)",
                            params![message.id, kind],
                        )? == 1
                        {
                            let operation = Operation::Send {
                                request_id: id(),
                                conversation_id: event.conversation_id.clone(),
                                body,
                            };
                            tx.execute(
                                "INSERT OR IGNORE INTO commands(key,operation) VALUES(?1,?2)",
                                params![
                                    format!("reply-{}", message.id),
                                    serde_json::to_string(&operation)?
                                ],
                            )?;
                        }
                    }
                }
            }
        }
        tx.execute("INSERT INTO metadata VALUES('cursor',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[&event.cursor])?;
        tx.commit()?;
        #[cfg(feature = "test-support")]
        if inserted == 1
            && event.message.as_ref().is_some_and(|m| {
                m.sender != identity
                    && ((!agent && m.body == "ask_pending_draft")
                        || (agent && m.body.starts_with("review_draft:")))
            })
        {
            fault("after-effect");
        }
        Ok(())
    }
}
fn statistics(c: &Connection) -> Result<Statistics> {
    let created: Option<Option<String>> = c
        .query_row(
            "SELECT outcome FROM commands WHERE key='startup-create'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let initiated_conversation = created
        .flatten()
        .map(|value| serde_json::from_str::<Outcome>(&value))
        .transpose()?
        .and_then(|outcome| {
            if let Outcome::Conversation(c) = outcome {
                Some(c.id)
            } else {
                None
            }
        });
    Ok(Statistics {
        initiated_conversation,
        processed_events: c.query_row("SELECT COUNT(*) FROM processed_events", [], |r| r.get(0))?,
        business_effects: c.query_row("SELECT COUNT(*) FROM business_effects", [], |r| r.get(0))?,
        pending_commands: c.query_row(
            "SELECT COUNT(*) FROM commands WHERE outcome IS NULL",
            [],
            |r| r.get(0),
        )?,
        accepted_commands: c.query_row(
            "SELECT COUNT(*) FROM commands WHERE outcome IS NOT NULL",
            [],
            |r| r.get(0),
        )?,
        cursor_saved: c.query_row(
            "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='cursor')",
            [],
            |r| r.get(0),
        )?,
    })
}
#[cfg(feature = "test-support")]
pub fn fault(stage: &str) {
    if std::env::var("CONNECT_EXAMPLE_FAULT_STAGE").as_deref() == Ok(stage) {
        std::process::exit(86);
    }
}
