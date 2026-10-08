//! Consistent SQLite snapshots and explicit, backup-gated schema transitions.
use crate::{Store, SCHEMA_VERSION};
use connect_protocol::{Error, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub installation_id: String,
    pub schema: u32,
    /// Logical database digest; includes credential verifiers and cursor key,
    /// never plaintext credentials. Do not log row contents.
    pub digest: String,
}

pub(crate) fn create_private_file(path: &Path) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| Error::new("conflict", "Destination must be a new writable file"))?;
    Ok(())
}

fn inspect(connection: &Connection) -> Result<Snapshot> {
    let schema: u32 = connection
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if !(1..=SCHEMA_VERSION).contains(&schema) {
        return Err(Error::new(
            "unsupported-schema",
            "Unsupported schema; use its matching binary and snapshot",
        ));
    }
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(|_| Error::storage())?;
    if integrity != "ok" {
        return Err(Error::storage());
    }
    let installation_id = connection
        .query_row("SELECT id FROM installation WHERE singleton=1", [], |r| {
            r.get(0)
        })
        .map_err(|_| Error::storage())?;
    let mut digest = Sha256::new();
    digest.update(schema.to_be_bytes());
    let mut tables = vec!["installation", "identities", "credentials"];
    if schema >= 2 {
        tables.extend([
            "conversations",
            "participants",
            "messages",
            "requests",
            "journal",
        ]);
    }
    if schema >= 3 {
        tables.extend([
            "connector_routes",
            "provider_jobs",
            "provider_events",
            "provider_context",
        ]);
    }
    if schema >= 4 {
        tables.extend(["voice_routes", "voice_calls"]);
    }
    if schema >= 5 {
        tables.push("voice_ingress");
    }
    for table in tables {
        digest.update(table.as_bytes());
        // Names are a fixed allowlist, never user-controlled SQL.
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table} ORDER BY 1,2"))
            .map_err(|_| Error::storage())?;
        let columns = statement.column_count();
        let mut rows = statement.query([]).map_err(|_| Error::storage())?;
        while let Some(row) = rows.next().map_err(|_| Error::storage())? {
            for index in 0..columns {
                use rusqlite::types::ValueRef;
                let value = row.get_ref(index).map_err(|_| Error::storage())?;
                let bytes = match value {
                    ValueRef::Null => b"null".to_vec(),
                    ValueRef::Integer(v) => format!("i:{v}").into_bytes(),
                    ValueRef::Real(v) => format!("f:{v}").into_bytes(),
                    ValueRef::Text(v) => [b"t:", v].concat(),
                    ValueRef::Blob(v) => [b"b:", v].concat(),
                };
                digest.update((bytes.len() as u64).to_be_bytes());
                digest.update(bytes);
            }
        }
    }
    Ok(Snapshot {
        installation_id,
        schema,
        digest: hex::encode(digest.finalize()),
    })
}

impl Store {
    /// Online backup includes committed WAL contents through SQLite's backup API.
    /// Destination creation is exclusive. A failure leaves an incomplete file,
    /// never a verified backup manifest.
    pub fn backup(&self, destination: &Path) -> Result<Snapshot> {
        let source = self.connection()?;
        backup_connection(&source, destination)
    }

    /// Backup a supported previous schema without upgrading it. This remains a
    /// recovery operation, not an application-service compatibility promise.
    pub fn backup_database(source: &Path, destination: &Path) -> Result<Snapshot> {
        let source = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::storage())?;
        backup_connection(&source, destination)
    }

    pub fn authorize_recovery(path: &Path, token: &str) -> Result<()> {
        use rusqlite::OptionalExtension;
        let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::storage())?;
        if token.len() != 64 {
            return Err(Error::unauthorized());
        }
        let id: Option<String> = c
            .query_row(
                "SELECT id FROM credentials WHERE verifier=?1",
                [crate::verifier(token)],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        crate::identity::actor_from_connection(&c, &id.ok_or_else(Error::unauthorized)?)?
            .require("admin")
    }

    /// Read-only compatibility / integrity inspection, including previous schema.
    pub fn inspect_snapshot(path: &Path) -> Result<Snapshot> {
        let mut c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| Error::storage())?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let snapshot = inspect(&tx)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(snapshot)
    }

    /// Explicit v1→v2 migration. Caller holds the installation lock. The verified
    /// backup must match all current logical state; stale backups cannot qualify.
    pub fn migrate(path: &Path, backup: &Path, expected: &Snapshot) -> Result<Snapshot> {
        if &Self::inspect_snapshot(backup)? != expected {
            return Err(Error::new(
                "invalid-backup",
                "Backup integrity or manifest does not match",
            ));
        }
        let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|_| Error::storage())?;
        Self::configure(&connection)?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if &inspect(&tx)? != expected {
            return Err(Error::new("invalid-backup", "Backup is stale or belongs to another installation; create a fresh verified backup"));
        }
        if expected.schema >= SCHEMA_VERSION {
            return Err(Error::new(
                "unsupported-schema",
                "Current schema needs no migration",
            ));
        }
        if expected.schema == 1 {
            tx.execute_batch(include_str!("../migrations/002_conversations.sql"))
                .map_err(|_| Error::storage())?;
        }
        if expected.schema < 3 {
            tx.execute_batch(include_str!("../migrations/003_providers.sql"))
                .map_err(|_| Error::storage())?;
        }
        if expected.schema < 4 {
            tx.execute_batch(include_str!("../migrations/004_voice.sql"))
                .map_err(|_| Error::storage())?;
        }
        if expected.schema < 5 {
            tx.execute_batch(include_str!("../migrations/005_inbound_voice.sql"))
                .map_err(|_| Error::storage())?;
        }
        tx.execute_batch(include_str!("../migrations/006_voice_recovery.sql"))
            .map_err(|_| Error::storage())?;
        let snapshot = inspect(&tx)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(snapshot)
    }
}

fn backup_connection(source: &Connection, destination: &Path) -> Result<Snapshot> {
    create_private_file(destination)?;
    let mut target = Connection::open_with_flags(destination, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| Error::storage())?;
    {
        let backup =
            rusqlite::backup::Backup::new(source, &mut target).map_err(|_| Error::storage())?;
        backup
            .run_to_completion(128, Duration::from_millis(5), None)
            .map_err(|_| Error::storage())?;
    }
    target
        .execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")
        .map_err(|_| Error::storage())?;
    let info = inspect(&target)?;
    drop(target);
    std::fs::File::open(destination)
        .and_then(|f| f.sync_all())
        .map_err(|_| Error::storage())?;
    Ok(info)
}
