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
    if schema >= 101 {
        tables.extend(["received_dtmf_sources", "received_dtmf_recipients"]);
    }
    if schema >= 100 {
        tables.extend([
            "native_dtmf_sources",
            "native_dtmf_recipients",
            "native_dtmf_gateways",
        ]);
    }
    if schema >= 98 {
        tables.extend(["session_codec_batches", "session_codec_targets"]);
    }
    if schema >= 85 {
        tables.push("user_delegations");
    }
    if schema >= 80 {
        tables.push("tenant_record_readers");
    }
    if schema >= 79 {
        tables.extend([
            "record_retention_settings",
            "session_record_retention",
            "record_retention_jobs",
        ]);
    }
    if schema >= 78 {
        tables.push("session_record_mutation_authority");
    }
    if schema >= 76 {
        tables.extend(["session_record_intents", "session_record_audiences"]);
    }
    if schema >= 75 {
        tables.extend(["session_records", "session_record_mutations"]);
    }
    if schema >= 72 {
        tables.extend([
            "incoming_contact_resources",
            "incoming_contact_holds",
            "incoming_contact_receipts",
            "incoming_contact_deliveries",
            "incoming_contact_rejections",
            "incoming_contact_audit",
        ]);
    }
    if schema >= 71 {
        tables.push("provider_message_endpoints");
    }
    if schema >= 70 {
        tables.push("provider_voice_quarantine_resolutions");
    }
    if schema >= 69 {
        tables.push("provider_voice_ingress");
    }
    if schema >= 67 {
        tables.push("provider_session_voice");
    }
    if schema >= 66 {
        tables.push("initial_telephone_receive_projections");
        tables.push("initial_telephone_answers");
    }
    if schema >= 65 {
        tables.push("initial_telephone_operations");
    }
    if schema >= 63 {
        tables.extend([
            "native_assistant_initializations",
            "native_assistant_context_updates",
        ]);
    }
    if schema >= 62 {
        tables.push("native_assistant_providers");
    }
    if schema >= 61 {
        tables.extend([
            "telephone_replacement_commits",
            "telephone_connection_lifetimes",
        ]);
    }
    if schema >= 60 {
        tables.push("telephone_replacement_phases");
    }
    if schema >= 59 {
        tables.push("telephone_replacement_resources");
    }
    if schema >= 58 {
        tables.push("native_replacement_audiences");
    }
    if schema >= 57 {
        tables.push("native_replacement_lifetimes");
    }
    if schema >= 56 {
        tables.push("native_replacement_media_steps");
    }
    if schema >= 55 {
        tables.extend(["native_replacements", "native_replacement_steps"]);
    }
    if schema >= 42 {
        tables.extend(["native_data_messages", "native_data_recipients"]);
    }
    if schema >= 43 {
        tables.push("native_data_writes");
    }
    if schema >= 40 {
        tables.push("native_stream_operations");
    }
    if schema >= 39 {
        tables.push("native_connection_lifetimes");
    }
    if schema >= 38 {
        tables.push("native_offer_rejections");
    }
    if schema >= 37 {
        tables.push("native_offer_operations");
    }
    if schema >= 36 {
        tables.push("connection_offers");
    }
    if schema >= 35 {
        tables.extend([
            "session_control",
            "session_presence_capabilities",
            "main_sessions",
        ]);
    }
    if schema >= 31 {
        tables.push("provider_dispatch");
    }
    if schema >= 2 {
        tables.extend(["conversations", "participants", "messages", "journal"]);
    }
    if schema < 32 {
        tables.push("requests");
    } else {
        tables.push("operation_control_receipts");
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
    if schema >= 7 {
        tables.extend([
            "sessions",
            "session_bindings",
            "availability",
            "artifacts",
            "context_deliveries",
        ]);
    }
    if schema >= 9 {
        tables.push("sip_legs");
    }
    if schema >= 10 {
        tables.extend([
            "sms_authorities",
            "sms_grants",
            "sms_controls",
            "sms_control_events",
        ]);
    }
    if schema >= 11 {
        tables.extend([
            "tasks",
            "task_events",
            "task_requests",
            "task_bindings",
            "task_grants",
            "task_replies",
            "task_effects",
            "task_claims",
        ]);
    }
    if schema >= 12 {
        tables.extend(["task_effect_authority", "task_effect_links"]);
    }
    if schema >= 14 {
        tables.extend(["media_end_intents", "media_start_admissions"]);
    }
    if schema >= 16 {
        tables.push("vapi_session_setups");
    }
    if schema >= 17 {
        tables.extend(["contact_pairings", "connector_manifests"]);
    }
    if schema >= 18 {
        tables.extend([
            "connector_observations",
            "connector_receipt_events",
            "connector_incoming",
            "connector_voice_observations",
        ]);
    }
    if schema >= 19 {
        tables.extend([
            "tenants",
            "identity_subjects",
            "devices",
            "endpoints",
            "message_audiences",
            "message_attachments",
            "message_receipts",
            "artifact_audiences",
            "session_participants",
            "connections",
            "operations",
            "operation_resources",
            "operation_effects",
            "operation_observations",
        ]);
    }
    if schema >= 21 {
        tables.extend([
            "membership_invitations",
            "audience_resources",
            "resource_audiences",
            "history_shares",
            "history_share_items",
        ]);
    }
    if schema >= 24 {
        tables.push("receipt_preferences");
    }
    if schema >= 25 {
        tables.extend([
            "operation_members",
            "operation_connections",
            "operation_endpoints",
            "operation_decisions",
            "operation_evidence",
        ]);
    }
    if schema >= 26 {
        tables.push("journal_retention");
    }
    if schema >= 27 {
        tables.push("journal_context");
    }
    if schema >= 28 {
        tables.extend(["peer_signing_keys", "credential_signing_keys"]);
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

    /// Gate 6 has a deliberate incompatible schema boundary. Retain the API
    /// only to return an actionable refusal to the operator's migration command.
    /// No historical conversion or mutation is attempted.
    pub fn migrate(_path: &Path, _backup: &Path, _expected: &Snapshot) -> Result<Snapshot> {
        Err(Error::new(
            "unsupported-schema",
            "Gate 6 does not convert historical state. Preserve a verified backup with its matching binary, then explicitly initialize a new state path",
        ))
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
