//! Durable application storage. SQLite is the only application-state authority.
mod conversations;
mod cursor;
mod identity;
mod recovery;

use connect_protocol::{Error, Limits, Result};
use rand::RngCore;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub use identity::{Actor, IssuedCredential, Scope};
pub use recovery::Snapshot;
pub const SCHEMA_VERSION: u32 = 2;

pub struct Store {
    connection: Mutex<Connection>,
    pub(crate) limits: Limits,
    path: PathBuf,
}
impl Store {
    /// Create a new database. Refuses to reuse an existing path.
    pub fn initialize(path: &Path, limits: Limits) -> Result<Self> {
        if path.exists() {
            return Err(Error::new(
                "conflict",
                "State already exists; initialization preserves it",
            ));
        }
        recovery::create_private_file(path)?;
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| Error::storage())?;
        Self::configure(&connection)?;
        let mut connection = connection;
        let tx = connection.transaction().map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/001_identity.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/002_conversations.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute(
            "INSERT INTO installation VALUES(1,?1,?2)",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), random_secret()],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
            path: path.into(),
        })
    }
    /// Open initialized state without creating a new database or resetting data.
    pub fn open(path: &Path, limits: Limits) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| Error::storage())?;
        let schema: u32 = connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if schema != SCHEMA_VERSION {
            return Err(Error::new(
                "unsupported-schema",
                "Use a compatible binary and verified backup; schema is unsupported",
            ));
        }
        Self::configure(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            limits,
            path: path.into(),
        })
    }
    fn configure(c: &Connection) -> Result<()> {
        c.busy_timeout(Duration::from_secs(2))
            .map_err(|_| Error::storage())?;
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(|_| Error::storage())
    }
    pub(crate) fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| Error::storage())
    }
    pub fn installation_id(&self) -> Result<String> {
        self.connection()?
            .query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
                r.get(0)
            })
            .map_err(|_| Error::storage())
    }
    pub fn integrity_check(&self) -> Result<()> {
        let result: String = self
            .connection()?
            .query_row("PRAGMA quick_check", [], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        if result != "ok" {
            return Err(Error::storage());
        };
        Ok(())
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
}
pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub(crate) fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
pub(crate) fn verifier(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Abrupt process termination exists only in explicitly featured test binaries.
#[cfg(feature = "test-support")]
pub fn fault_point(stage: &str, request_id: &str) {
    if std::env::var("CONNECT_TEST_FAULT_STAGE").as_deref() == Ok(stage)
        && std::env::var("CONNECT_TEST_FAULT_REQUEST").as_deref() == Ok(request_id)
    {
        std::process::exit(86);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_protocol::{Operation, Outcome};
    #[test]
    fn actual_sqlite_full_rolls_back_without_acceptance() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let app = store.create_identity("app").unwrap();
        let key = store.issue(&app.id, Scope::operator(), now() + 60).unwrap();
        let id = match store
            .execute(
                &key.id,
                &Operation::Create {
                    request_id: "create".into(),
                    participants: vec![],
                },
            )
            .unwrap()
        {
            Outcome::Conversation(c) => c.id,
            _ => panic!("create"),
        };
        {
            let c = store.connection().unwrap();
            let pages: u32 = c
                .pragma_query_value(None, "page_count", |r| r.get(0))
                .unwrap();
            c.pragma_update(None, "max_page_count", pages).unwrap();
        }
        let send = Operation::Send {
            request_id: "full".into(),
            conversation_id: id,
            body: "x".repeat(16_384),
        };
        assert_eq!(
            store.execute(&key.id, &send).unwrap_err().code,
            "storage-unavailable"
        );
        let c = store.connection().unwrap();
        for (table, expected) in [("messages", 0), ("journal", 1), ("requests", 1)] {
            assert_eq!(
                c.query_row::<i64, _, _>(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get(0))
                    .unwrap(),
                expected
            );
        }
    }
}
