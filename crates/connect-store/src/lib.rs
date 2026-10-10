//! Durable application storage. SQLite is the only application-state authority.
mod artifacts;
mod availability;
mod connector_registry;
mod context;
mod discovery;
pub use discovery::{BrowserRoute, DiscoveryRuntime};
mod conversation_queries;
mod conversations;
mod cursor;
mod identity;
mod media_end;
mod operations;
mod provider_chat_context;
mod provider_context;
mod providers;
mod sms_eligibility;
pub use sms_eligibility::{
    control_kind as sms_control_kind, SmsGrant, SmsGrantSnapshot, SMS_LEASE_SECONDS,
};
mod recovery;
mod session_routes;
mod sessions;
mod sip_legs;
mod tasks;
mod voice;
pub use providers::{event_digest as provider_event_digest, ProviderJob, Route};
pub use voice::{InboundCall, VoiceAction, VoiceIngress, VoiceJob, VoiceRoute};

use connect_protocol::{Error, Limits, Result};
use rand::RngCore;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub use identity::{Actor, IssuedCredential, Scope};
pub use recovery::Snapshot;
pub use sessions::{RoutedInboundSession, VapiSessionRoute};
pub use sip_legs::{SipLegAction, SipLegJob};
pub use tasks::{IssuedTaskGrant, NativeTaskAuthority, TaskInvocation};
pub const SCHEMA_VERSION: u32 = 15;
const STARTUP_BLOB_CLEANUP_LIMIT: usize = 100;

pub struct Store {
    connectors: connector_registry::Registry,
    connection: Mutex<Connection>,
    pub(crate) limits: Limits,
    path: PathBuf,
    blob_directory: PathBuf,
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
        tx.execute_batch(include_str!("../migrations/003_providers.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/004_voice.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/005_inbound_voice.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/006_voice_recovery.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!(
            "../migrations/007_sessions_context_artifacts.sql"
        ))
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/008_artifact_expiry.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/009_sip_legs.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/010_sms_eligibility.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/011_tasks.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/012_task_effect_authority.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/013_task_sms_policy.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/014_media_end_authority.sql"))
            .map_err(|_| Error::storage())?;
        tx.execute(
            "INSERT INTO installation VALUES(1,?1,?2)",
            rusqlite::params![uuid::Uuid::new_v4().to_string(), random_secret()],
        )
        .map_err(|_| Error::storage())?;
        tx.execute_batch(include_str!("../migrations/015_native_media_authority.sql"))
            .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        let blob_directory = Self::create_blob_directory(path)?;
        let store = Self {
            connectors: Default::default(),
            connection: Mutex::new(connection),
            limits,
            path: path.into(),
            blob_directory,
        };
        store.purge_orphan_artifact_blobs(STARTUP_BLOB_CLEANUP_LIMIT)?;
        Ok(store)
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
        let blob_directory = Self::create_blob_directory(path)?;
        let store = Self {
            connectors: Default::default(),
            connection: Mutex::new(connection),
            limits,
            path: path.into(),
            blob_directory,
        };
        // A previous process may have committed deletion or crashed after an
        // atomic blob rename but before publishing its database reference.
        // Keep startup work bounded while ensuring those bytes are eventually
        // erased without ever touching a referenced ready artifact.
        store.purge_orphan_artifact_blobs(STARTUP_BLOB_CLEANUP_LIMIT)?;
        Ok(store)
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

    pub fn blob_directory_for(database: &Path) -> PathBuf {
        database.with_extension("blobs")
    }

    pub fn blob_directory(&self) -> &Path {
        &self.blob_directory
    }

    pub fn artifact_blob_names(database: &Path) -> Result<Vec<String>> {
        let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::storage())?;
        let schema: u32 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| Error::storage())?;
        if schema < 7 {
            return Ok(Vec::new());
        }
        let query = if schema >= 8 {
            "SELECT blob_name FROM artifacts WHERE state='ready' AND blob_name IS NOT NULL AND (expires_at IS NULL OR expires_at>strftime('%s','now')) ORDER BY blob_name"
        } else {
            "SELECT blob_name FROM artifacts WHERE state='ready' AND blob_name IS NOT NULL ORDER BY blob_name"
        };
        let mut statement = connection.prepare(query).map_err(|_| Error::storage())?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        if names.iter().any(|name| !artifacts::valid_blob_name(name)) {
            return Err(Error::storage());
        }
        Ok(names)
    }

    /// Remove at most `limit` completed or temporary blob files which are not
    /// referenced by a ready artifact. Holding the database mutex makes the
    /// scan mutually exclusive with finalize, so cleanup cannot race the
    /// database transition which publishes a blob.
    pub fn purge_orphan_artifact_blobs(&self, limit: usize) -> Result<usize> {
        if limit == 0 {
            return Ok(0);
        }
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT blob_name FROM artifacts WHERE state='ready' AND blob_name IS NOT NULL AND (expires_at IS NULL OR expires_at>strftime('%s','now'))",
            )
            .map_err(|_| Error::storage())?;
        let live = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<HashSet<_>>>()
            .map_err(|_| Error::storage())?;
        if live.iter().any(|name| !artifacts::valid_blob_name(name)) {
            return Err(Error::storage());
        }

        let mut removed = 0;
        let entries = std::fs::read_dir(&self.blob_directory).map_err(|_| Error::storage())?;
        for entry in entries {
            if removed == limit {
                break;
            }
            let entry = entry.map_err(|_| Error::storage())?;
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(_) => continue,
            };
            let is_final_orphan =
                artifacts::valid_blob_name(&name) && !live.contains(name.as_str());
            let is_stale_temporary = artifacts::valid_temporary_blob_name(&name);
            if !is_final_orphan && !is_stale_temporary {
                continue;
            }
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|_| Error::storage())?;
            if metadata.file_type().is_file() {
                std::fs::remove_file(entry.path()).map_err(|_| Error::storage())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn create_blob_directory(database: &Path) -> Result<PathBuf> {
        let directory = Self::blob_directory_for(database);
        match std::fs::create_dir(&directory) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                        .map_err(|_| Error::storage())?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if !std::fs::symlink_metadata(&directory)
                    .map_err(|_| Error::storage())?
                    .file_type()
                    .is_dir()
                {
                    return Err(Error::storage());
                }
            }
            Err(_) => return Err(Error::storage()),
        }
        Ok(directory)
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
#[allow(clippy::unwrap_used)] // Panics are deliberate assertions in these fixtures.
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
