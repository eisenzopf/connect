//! Private record persistence. A storage reservation protects exact bytes; it
//! grants no Identity, Session, source audience or delivery authority.
pub(crate) mod context;
pub use context::{RecordContextData, RecordContextInput, RecordContextProjector};

use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope, AuthorityWait, Store};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use connect_protocol::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::{sync::Arc, time::Duration};

const MAX_REVISION: u64 = 9_007_199_254_740_991;
const MAX_RECORDS: usize = 4096;
fn conflict() -> Error {
    Error::new("conflict", "Record original intent or revision conflicts")
}
fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
}
#[derive(Clone, PartialEq, Eq)]
pub struct RecordScope {
    pub tenant_id: String,
    pub conversation_id: String,
    pub session_id: String,
}
impl RecordScope {
    fn validate(&self) -> Result<()> {
        if [&self.tenant_id, &self.conversation_id, &self.session_id]
            .into_iter()
            .all(|v| valid_id(v))
        {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}
#[derive(Clone)]
pub struct RecordBytes {
    pub uuid: String,
    pub body: Vec<u8>,
    pub manifest: Vec<u8>,
}
impl RecordBytes {
    fn validate(&self) -> Result<()> {
        let uuid = uuid::Uuid::parse_str(&self.uuid).map_err(|_| Error::storage())?;
        if uuid.is_nil()
            || uuid.to_string() != self.uuid
            || self.body.is_empty()
            || self.body.len() > 16 * 1024 * 1024
            || self.manifest.is_empty()
            || self.manifest.len() > 1024 * 1024
        {
            return Err(Error::storage());
        }
        let body: serde_json::Value =
            serde_json::from_slice(&self.body).map_err(|_| Error::storage())?;
        if body.get("uuid").and_then(|v| v.as_str()) != Some(self.uuid.as_str()) {
            return Err(Error::storage());
        }
        Ok(())
    }
    fn handle(&self, revision: u64) -> RecordHandle {
        RecordHandle {
            location: location(&self.uuid),
            body_hash: hash(&self.body),
            manifest_hash: hash(&self.manifest),
            revision,
        }
    }
}
fn hash(bytes: &[u8]) -> String {
    format!("sha512-{}", URL_SAFE_NO_PAD.encode(Sha512::digest(bytes)))
}
fn location(uuid: &str) -> String {
    format!("connect:vcon/{uuid}")
}
fn selected_uuid(url: &str) -> Option<String> {
    let id = uuid::Uuid::parse_str(url.strip_prefix("connect:vcon/")?).ok()?;
    let id = id.to_string();
    (location(&id) == url).then_some(id)
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordHandle {
    pub location: String,
    pub body_hash: String,
    pub manifest_hash: String,
    pub revision: u64,
}
impl std::fmt::Debug for RecordHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordHandle")
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
pub struct RecordRead {
    pub bytes: RecordBytes,
    pub handle: RecordHandle,
    _reservation: Arc<AuthorityGuardSet>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RecordMutationKind {
    Delete,
    Redact,
}
impl RecordMutationKind {
    fn name(self) -> &'static str {
        match self {
            Self::Delete => "delete",
            Self::Redact => "redact",
        }
    }
}
#[derive(Clone)]
pub struct RecordMutationFact {
    pub operation_id: String,
    pub fingerprint: String,
    pub result: RecordHandle,
    pub at: chrono::DateTime<chrono::Utc>,
}
struct Row {
    handle: RecordHandle,
    bytes: Option<RecordBytes>,
}
fn row(c: &Connection, s: &RecordScope, id: &str) -> Result<Option<Row>> {
    c.query_row("SELECT location,revision,body_hash,manifest_hash,body,manifest FROM session_records WHERE uuid=?1 AND tenant_id=?2 AND conversation_id=?3 AND session_id=?4",
        params![id,s.tenant_id,s.conversation_id,s.session_id], |r| {
            let body: Option<Vec<u8>> = r.get(4)?;
            let manifest: Option<Vec<u8>> = r.get(5)?;
            Ok(Row { handle: RecordHandle { location:r.get(0)?,revision:r.get(1)?,body_hash:r.get(2)?,manifest_hash:r.get(3)? },
                bytes:body.zip(manifest).map(|(body,manifest)|RecordBytes{uuid:id.into(),body,manifest}) })
        }).optional().map_err(|_|Error::storage())
}
fn verify(row: &Row) -> Result<()> {
    if let Some(bytes) = &row.bytes {
        bytes.validate()?;
        if bytes.handle(row.handle.revision) != row.handle {
            return Err(Error::storage());
        }
    }
    Ok(())
}
impl Store {
    fn record_reservation(
        &self,
        uuid: &str,
        mode: AuthorityMode,
    ) -> Result<Arc<AuthorityGuardSet>> {
        self.authority.acquire(
            &[AuthorityScope::record(uuid, mode)?],
            &AuthorityWait::new(Duration::from_secs(2)),
        )
    }
    /// Trusted Service storage composition. Current mutation authority is the
    /// caller's responsibility, including retries. Never a public client API.
    pub fn put_record(&self, scope: &RecordScope, bytes: RecordBytes) -> Result<RecordHandle> {
        scope.validate()?;
        bytes.validate()?;
        let _held = self.record_reservation(&bytes.uuid, AuthorityMode::Write)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let handle = put(&tx, scope, &bytes, false)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(handle)
    }
    /// Select and retain the same read reservation through actual delivery.
    /// Unknown/foreign selectors reveal nothing and are never fetched remotely.
    pub fn read_record(
        &self,
        scope: &RecordScope,
        uuid: Option<&str>,
        url: Option<&str>,
        expected: Option<&RecordHandle>,
    ) -> Result<Option<RecordRead>> {
        scope.validate()?;
        let id = match (uuid, url) {
            (Some(id), None) => uuid::Uuid::parse_str(id)
                .ok()
                .filter(|v| v.to_string() == id)
                .map(|_| id.to_owned()),
            (None, Some(url)) => selected_uuid(url),
            (Some(id), Some(url)) => selected_uuid(url).filter(|selected| selected == id),
            _ => None,
        };
        let Some(id) = id else {
            return Ok(None);
        };
        let held = self.record_reservation(&id, AuthorityMode::Read)?;
        let c = self.connection()?;
        let Some(row) = row(&c, scope, &id)? else {
            return Ok(None);
        };
        verify(&row)?;
        if row.bytes.is_none() {
            return Ok(None);
        }
        if expected.is_some_and(|h| h != &row.handle) {
            return Err(conflict());
        }
        Ok(row.bytes.map(|bytes| RecordRead {
            bytes,
            handle: row.handle,
            _reservation: held,
        }))
    }
    /// Original-operation CAS and canonical transformation are one transaction
    /// under the exclusive record reservation. Replays recover historical facts
    /// even after later deletion; changed or reused original intents conflict.
    pub fn mutate_record(
        &self,
        scope: &RecordScope,
        operation: &str,
        kind: RecordMutationKind,
        fingerprint: &str,
        expected: &RecordHandle,
        transform: impl FnOnce(&RecordBytes) -> Result<Option<RecordBytes>>,
    ) -> Result<Option<RecordMutationFact>> {
        scope.validate()?;
        if !valid_id(operation)
            || fingerprint.is_empty()
            || fingerprint.len() > 512
            || expected.revision == 0
            || expected.revision >= MAX_REVISION
        {
            return Err(conflict());
        }
        let Some(id) = selected_uuid(&expected.location) else {
            return Ok(None);
        };
        // Provider context retains the conversation read fence through actual
        // IO. Mutation joins that fence before taking the record writer, so a
        // copied projection cannot outlive erasure or retention confirmation.
        let _source = self.acquire_authority(
            &[AuthorityScope::conversation(
                &scope.conversation_id,
                AuthorityMode::Write,
            )?],
            &AuthorityWait::new(Duration::from_secs(2)),
        )?;
        let _held = self.record_reservation(&id, AuthorityMode::Write)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let result = mutate_locked(
            &tx,
            scope,
            operation,
            kind,
            fingerprint,
            expected,
            transform,
        )?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    pub fn record_tombstone(
        &self,
        scope: &RecordScope,
        url: &str,
    ) -> Result<Option<RecordMutationFact>> {
        scope.validate()?;
        let Some(id) = selected_uuid(url) else {
            return Ok(None);
        };
        let _held = self.record_reservation(&id, AuthorityMode::Read)?;
        let c = self.connection()?;
        if row(&c, scope, &id)?.is_none_or(|r| r.bytes.is_some()) {
            return Ok(None);
        }
        let value:Option<(String,String,String,String)>=c.query_row("SELECT operation_id,fingerprint,result_json,at FROM session_record_mutations WHERE uuid=?1 AND kind='delete'",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        value
            .map(|(operation_id, fingerprint, result, at)| {
                Ok(RecordMutationFact {
                    operation_id,
                    fingerprint,
                    result: serde_json::from_str(&result).map_err(|_| Error::storage())?,
                    at: at.parse().map_err(|_| Error::storage())?,
                })
            })
            .transpose()
    }
    /// Local index only. These handles do not carry a source audience grant.
    pub fn list_records(
        &self,
        tenant: &str,
        conversation: &str,
        session: Option<&str>,
    ) -> Result<Vec<RecordHandle>> {
        if !valid_id(tenant) || !valid_id(conversation) || session.is_some_and(|v| !valid_id(v)) {
            return Err(Error::forbidden());
        }
        let c = self.connection()?;
        let mut query=c.prepare("SELECT location,body_hash,manifest_hash,revision FROM session_records WHERE tenant_id=?1 AND conversation_id=?2 AND (?3 IS NULL OR session_id=?3) AND deleted=0 ORDER BY uuid LIMIT 4097").map_err(|_|Error::storage())?;
        let rows = query
            .query_map(params![tenant, conversation, session], |r| {
                Ok(RecordHandle {
                    location: r.get(0)?,
                    body_hash: r.get(1)?,
                    manifest_hash: r.get(2)?,
                    revision: r.get(3)?,
                })
            })
            .map_err(|_| Error::storage())?;
        let values = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage())?;
        if values.len() > MAX_RECORDS {
            return Err(Error::new(
                "capacity-exceeded",
                "Record index capacity reached",
            ));
        }
        Ok(values)
    }
}

mod outcomes;
pub(crate) mod readiness;
pub use outcomes::{RecordIntent, RecordIntentBatch, RecordSourceEpoch};

fn put(
    c: &Connection,
    scope: &RecordScope,
    bytes: &RecordBytes,
    baseline: bool,
) -> Result<RecordHandle> {
    let owned:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM sessions s JOIN conversations c ON c.id=s.conversation_id WHERE s.id=?1 AND c.id=?2 AND c.tenant_id=?3)",params![scope.session_id,scope.conversation_id,scope.tenant_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !owned {
        return Err(Error::forbidden());
    }
    let reserved: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_record_intents WHERE uuid=?1)",
            [&bytes.uuid],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if reserved != baseline {
        return Err(conflict());
    }
    let handle = bytes.handle(1);
    let exists: bool = c
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_records WHERE uuid=?1)",
            [&bytes.uuid],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if exists {
        let prior = row(c, scope, &bytes.uuid)?.ok_or_else(conflict)?;
        verify(&prior)?;
        return if prior
            .bytes
            .is_some_and(|p| p.body == bytes.body && p.manifest == bytes.manifest)
            && prior.handle == handle
        {
            Ok(handle)
        } else {
            Err(conflict())
        };
    }
    let count:usize=c.query_row("SELECT count(*) FROM session_records WHERE tenant_id=?1 AND conversation_id=?2 AND session_id=?3",params![scope.tenant_id,scope.conversation_id,scope.session_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if count
        >= if baseline {
            MAX_RECORDS
        } else {
            MAX_RECORDS - 1
        }
    {
        return Err(Error::new(
            "capacity-exceeded",
            "Session record capacity reached",
        ));
    }
    c.execute("INSERT INTO session_records(uuid,tenant_id,conversation_id,session_id,location,revision,body,manifest,body_hash,manifest_hash) VALUES(?1,?2,?3,?4,?5,1,?6,?7,?8,?9)",params![bytes.uuid,scope.tenant_id,scope.conversation_id,scope.session_id,handle.location,bytes.body,bytes.manifest,handle.body_hash,handle.manifest_hash]).map_err(|_|Error::storage())?;

    retention::index(c, &bytes.uuid, Some(bytes))?;
    Ok(handle)
}

pub(crate) mod read;
pub use read::{
    RecordFetchIntent, RecordPolicySnapshot, RecordViewBinding, RecordViewOrigin, RecordViewer,
};

fn mutate_locked(
    tx: &rusqlite::Transaction<'_>,
    scope: &RecordScope,
    operation: &str,
    kind: RecordMutationKind,
    fingerprint: &str,
    expected: &RecordHandle,
    transform: impl FnOnce(&RecordBytes) -> Result<Option<RecordBytes>>,
) -> Result<Option<RecordMutationFact>> {
    let Some(id) = selected_uuid(&expected.location) else {
        return Ok(None);
    };
    let Some(prior) = row(&tx, scope, &id)? else {
        return Ok(None);
    };
    verify(&prior)?;
    let saved:Option<(String,String,String,String,String)>=tx.query_row("SELECT kind,fingerprint,expected_json,result_json,at FROM session_record_mutations WHERE uuid=?1 AND operation_id=?2",params![id,operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?;
    let original = serde_json::to_string(expected).map_err(|_| Error::storage())?;
    if let Some((saved_kind, saved_fp, saved_expected, result, at)) = saved {
        if saved_kind != kind.name() || saved_fp != fingerprint || saved_expected != original {
            return Err(conflict());
        }
        return Ok(Some(RecordMutationFact {
            operation_id: operation.into(),
            fingerprint: fingerprint.into(),
            result: serde_json::from_str(&result).map_err(|_| Error::storage())?,
            at: at.parse().map_err(|_| Error::storage())?,
        }));
    }
    if prior.handle != *expected {
        return Err(conflict());
    }
    let bytes = prior.bytes.ok_or_else(conflict)?;
    let count: usize = tx
        .query_row(
            "SELECT count(*) FROM session_record_mutations WHERE uuid=?1",
            [&id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if count >= MAX_RECORDS && kind == RecordMutationKind::Redact {
        return Err(Error::new(
            "capacity-exceeded",
            "Record mutation capacity reached",
        ));
    }
    let next = transform(&bytes)?;
    if next.is_none() != (kind == RecordMutationKind::Delete) {
        return Err(Error::storage());
    }
    let revision = expected.revision + 1;
    let result = if let Some(next) = &next {
        next.validate()?;
        if next.uuid != id {
            return Err(conflict());
        }
        next.handle(revision)
    } else {
        RecordHandle {
            revision,
            ..expected.clone()
        }
    };
    tx.execute("UPDATE session_records SET revision=?2,body=?3,manifest=?4,body_hash=?5,manifest_hash=?6,deleted=?7 WHERE uuid=?1",params![id,revision,next.as_ref().map(|b|&b.body),next.as_ref().map(|b|&b.manifest),result.body_hash,result.manifest_hash,next.is_none()]).map_err(|_|Error::storage())?;
    retention::index(tx, &id, next.as_ref())?;
    let at = chrono::Utc::now();
    tx.execute(
        "INSERT INTO session_record_mutations VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            id,
            operation,
            kind.name(),
            fingerprint,
            original,
            serde_json::to_string(&result).map_err(|_| Error::storage())?,
            at.to_rfc3339()
        ],
    )
    .map_err(|_| Error::storage())?;
    Ok(Some(RecordMutationFact {
        operation_id: operation.into(),
        fingerprint: fingerprint.into(),
        result,
        at,
    }))
}

pub mod mutations;
pub mod retention;
pub(crate) mod roles;
