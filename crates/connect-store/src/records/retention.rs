//! Installation-owned local expiry. Selection commits before transformation;
//! retries use the original cutoff, exact expected handle and source decision.
use super::*;
use connect_protocol::record_mutations::Mode;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub source_id: String,
    pub mode: Mode,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Decision {
    Delete,
    Redact { sources: Vec<Source> },
}
impl Decision {
    fn kind(&self) -> RecordMutationKind {
        match self {
            Self::Delete => RecordMutationKind::Delete,
            Self::Redact { .. } => RecordMutationKind::Redact,
        }
    }
    fn validate(&self) -> Result<()> {
        if let Self::Redact { sources } = self {
            let unique: std::collections::BTreeSet<_> =
                sources.iter().map(|s| &s.source_id).collect();
            if sources.is_empty()
                || sources.len() > 4096
                || unique.len() != sources.len()
                || sources.iter().any(|s| !valid_id(&s.source_id))
            {
                return Err(Error::storage());
            }
        }
        Ok(())
    }
}
// Index exact persisted deadlines; never extend them from worker/restart time.
pub(super) fn index(c: &Connection, uuid: &str, bytes: Option<&RecordBytes>) -> Result<()> {
    let mut due = None;
    let mut document = None;
    if let Some(bytes) = bytes {
        let manifest: serde_json::Value =
            serde_json::from_slice(&bytes.manifest).map_err(|_| Error::storage())?;
        if let Some(sources) = manifest["sources"].as_array() {
            for source in sources {
                if source["state"] == "metadata-unavailable" {
                    continue;
                }
                for key in ["metadata_retention", "content_retention"] {
                    if key == "content_retention" && source["state"] == "content-unavailable" {
                        continue;
                    }
                    let value = &source[key];
                    if value["kind"] != "until" {
                        continue;
                    }
                    let instant = value["until"]
                        .as_str()
                        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                        .ok_or_else(Error::storage)?;
                    let at = instant.timestamp() + i64::from(instant.timestamp_subsec_nanos() > 0);
                    due = Some(due.map_or(at, |v: i64| v.min(at)));
                    if source["target"]["kind"] == "document" && key == "metadata_retention" {
                        document = Some(at);
                    }
                }
            }
        }
    }
    c.execute(
        "UPDATE session_records SET retention_due=?2,document_expires_at=?3 WHERE uuid=?1",
        params![uuid, due, document],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
fn owner(c: &Connection, uuid: &str) -> Result<RecordScope> {
    c.query_row(
        "SELECT tenant_id,conversation_id,session_id FROM session_records WHERE uuid=?1",
        [uuid],
        |r| {
            Ok(RecordScope {
                tenant_id: r.get(0)?,
                conversation_id: r.get(1)?,
                session_id: r.get(2)?,
            })
        },
    )
    .map_err(|_| Error::storage())
}
impl Store {
    /// Trusted protected installation configuration, never a client capability.
    /// This only selects policy for future terminal transactions.
    pub fn configure_record_retention(
        &self,
        policy: &connect_protocol::record_retention::Policy,
    ) -> Result<()> {
        policy.validate()?;
        let c = self.connection()?;
        c.execute(
            "UPDATE record_retention_settings SET policy_json=?1 WHERE singleton=1",
            [serde_json::to_string(policy).map_err(|_| Error::storage())?],
        )
        .map_err(|_| Error::storage())?;
        Ok(())
    }
    /// Pending decisions take priority over new due work. At most 32 records.
    pub fn pending_record_retention(&self, limit: usize, cutoff: i64) -> Result<Vec<String>> {
        if !(1..=32).contains(&limit) {
            return Err(Error::new(
                "invalid-request",
                "Retention batch must contain one to 32 items",
            ));
        }
        let c = self.connection()?;
        let mut q=c.prepare("SELECT uuid FROM (SELECT uuid,0 AS priority,cutoff AS due FROM record_retention_jobs WHERE state='pending' UNION ALL SELECT uuid,1,retention_due FROM session_records r WHERE deleted=0 AND retention_due<=?1 AND NOT EXISTS(SELECT 1 FROM record_retention_jobs j WHERE j.uuid=r.uuid AND j.state='pending')) ORDER BY priority,due,uuid LIMIT ?2").map_err(|_|Error::storage())?;
        let rows = q
            .query_map(params![cutoff, limit], |r| r.get(0))
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage());
        rows
    }
    /// Callback is bounded CPU-only canonical selection. No protected read is
    /// returned, so acquiring the later writer cannot deadlock on our own read.
    pub fn prepare_record_retention(
        &self,
        uuid: &str,
        cutoff: i64,
        select: impl FnOnce(&RecordBytes, &RecordHandle, &str, i64) -> Result<Option<Decision>>,
    ) -> Result<Option<String>> {
        if uuid::Uuid::parse_str(uuid)
            .ok()
            .is_none_or(|v| v.to_string() != uuid)
        {
            return Err(Error::storage());
        }
        let _held = self.record_reservation(uuid, AuthorityMode::Write)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let pending: Option<String> = tx
            .query_row(
                "SELECT operation_id FROM record_retention_jobs WHERE uuid=?1 AND state='pending'",
                [uuid],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?;
        if pending.is_some() {
            return Ok(pending);
        }
        let scope = owner(&tx, uuid)?;
        let prior = row(&tx, &scope, uuid)?.ok_or_else(Error::storage)?;
        verify(&prior)?;
        let due: Option<i64> = tx
            .query_row(
                "SELECT retention_due FROM session_records WHERE uuid=?1 AND deleted=0",
                [uuid],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| Error::storage())?
            .flatten();
        if due.is_none_or(|at| at > cutoff) {
            return Ok(None);
        }
        let operation = format!("retention_{}", uuid::Uuid::new_v4().simple());
        let decision = select(
            prior.bytes.as_ref().ok_or_else(Error::storage)?,
            &prior.handle,
            &operation,
            cutoff,
        )?
        .ok_or_else(Error::storage)?;
        decision.validate()?;
        tx.execute(
            "INSERT INTO record_retention_jobs VALUES(?1,?2,?3,?4,?5,?6,'pending')",
            params![
                operation,
                uuid,
                prior.handle.revision,
                serde_json::to_string(&prior.handle).map_err(|_| Error::storage())?,
                cutoff,
                serde_json::to_string(&decision).map_err(|_| Error::storage())?
            ],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(operation))
    }
    /// Only the durable decision is applied. A competing public mutation marks
    /// it superseded; the next iteration may select a new exact revision.
    pub fn apply_record_retention(
        &self,
        operation: &str,
        transform: impl FnOnce(
            &RecordBytes,
            &RecordHandle,
            &str,
            &Decision,
        ) -> Result<Option<RecordBytes>>,
    ) -> Result<bool> {
        let (uuid, scope) = {
            let c = self.connection()?;
            let uuid: String = c
                .query_row(
                    "SELECT uuid FROM record_retention_jobs WHERE operation_id=?1",
                    [operation],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            let scope = owner(&c, &uuid)?;
            (uuid, scope)
        };
        let scopes = {
            let c = self.connection()?;
            crate::authority_admission::conversation_scopes(
                &c,
                &scope.conversation_id,
                AuthorityMode::Write,
            )?
        };
        let _source = self
            .authority
            .acquire(&scopes, &AuthorityWait::new(Duration::from_secs(2)))?;
        let _held = self.record_reservation(&uuid, AuthorityMode::Write)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let current = crate::authority_admission::conversation_scopes(
            &tx,
            &scope.conversation_id,
            AuthorityMode::Write,
        )?;
        if !self.authority_covers(&_source, &current) {
            return Err(conflict());
        }
        let (expected,cutoff,decision,state):(String,i64,String,String)=tx.query_row("SELECT expected_json,cutoff,decision_json,state FROM record_retention_jobs WHERE operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::storage())?;
        if state != "pending" {
            return Ok(false);
        }
        let expected: RecordHandle =
            serde_json::from_str(&expected).map_err(|_| Error::storage())?;
        let decision: Decision = serde_json::from_str(&decision).map_err(|_| Error::storage())?;
        decision.validate()?;
        let prior = row(&tx, &scope, &uuid)?.ok_or_else(Error::storage)?;
        verify(&prior)?;
        if prior.handle != expected || prior.bytes.is_none() {
            tx.execute(
                "UPDATE record_retention_jobs SET state='superseded' WHERE operation_id=?1",
                [operation],
            )
            .map_err(|_| Error::storage())?;
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(false);
        }
        let fingerprint = hash(
            &serde_json::to_vec(&("connect-retention-v1", cutoff, &decision))
                .map_err(|_| Error::storage())?,
        );
        let fact = mutate_locked(
            &tx,
            &scope,
            operation,
            decision.kind(),
            &fingerprint,
            &expected,
            |bytes| transform(bytes, &expected, operation, &decision),
        )?
        .ok_or_else(Error::storage)?;
        mutations::publish(self, &tx, &scope, &uuid, decision.kind(), &fact, operation)?;
        tx.execute(
            "UPDATE record_retention_jobs SET state='applied' WHERE operation_id=?1",
            [operation],
        )
        .map_err(|_| Error::storage())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
