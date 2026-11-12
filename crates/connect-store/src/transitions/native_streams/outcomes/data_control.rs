//! Positive Data-only receipts keep existing AV authorization unchanged.
//! Other controls retain the conservative historical Writer path.
use super::*;
use crate::authority_writer_boundary::phase_ledger::{
    self,
    data_control::{Change, Plan, Row, Table},
};
use rusqlite::types::Value;
use std::{collections::BTreeMap, sync::Arc};

/// Trusted owned-host observation, implemented only around an actual sealed
/// native/Core commit witness. This is neither a wire assertion nor a grant.
/// Matching is historical and must not acquire IO, SQL or current policy.
pub trait NativeDataSubscriptionObservation: Send + Sync {
    fn matches(&self, scope: &NativeDataSubscriptionScope<'_>) -> bool;
}
pub struct NativeDataSubscriptionScope<'a> {
    pub conversation_id: &'a str,
    pub session_id: &'a str,
    pub operation_id: &'a str,
    pub record_id: &'a str,
    pub parent_record_id: &'a str,
    pub connection_id: &'a str,
    pub runtime_connection_id: &'a str,
    pub generation: u64,
    pub expected_revision: u64,
    pub stream_revision: u64,
    pub selected_stream_ids: &'a [String],
}
impl NativeStreamLease {
    /// Only the host holding a genuine actual native unchanged-AV commit witness
    /// may use this boundary. Without that witness committed() remains broad.
    pub fn committed_with_unchanged_av(
        self,
        revision: u64,
        resources_changed: bool,
        projection: Option<Vec<connect_protocol::native_streams::CodecBinding>>,
        observation: Arc<dyn NativeDataSubscriptionObservation>,
    ) -> Result<NativeStreamCommitted> {
        let mut receipt = self.committed(revision, resources_changed, projection)?;
        let CommittedOutcome::Streams { unchanged_av, .. } = &mut receipt.outcome else {
            return Err(conflict());
        };
        *unchanged_av = Some(observation);
        Ok(receipt)
    }
}
#[cfg(test)]
thread_local! {
    static BEFORE_ADMISSION: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
pub(crate) struct DataAdmissionTestHook;
#[cfg(test)]
impl Drop for DataAdmissionTestHook {
    fn drop(&mut self) {
        BEFORE_ADMISSION.with(|slot| {
            slot.borrow_mut().take();
        });
    }
}
#[cfg(test)]
impl Store {
    /// One invocation-local scheduling boundary after preview SQL dropped,
    /// before physical proof admission. It supplies no result/facts/authority.
    pub(crate) fn data_control_before_admission_for_test(
        f: impl FnOnce() + 'static,
    ) -> DataAdmissionTestHook {
        BEFORE_ADMISSION.with(|slot| {
            assert!(slot.borrow_mut().replace(Box::new(f)).is_none());
        });
        DataAdmissionTestHook
    }
}
fn invalid() -> Error {
    Error::forbidden()
}
fn json(c: &Connection, sql: &str, params: impl rusqlite::Params) -> Result<String> {
    c.prepare_cached(sql)
        .map_err(|_| Error::storage())?
        .query_row(params, |r| r.get(0))
        .map_err(|_| Error::storage())
}
fn av_records(value: &serde_json::Value) -> Result<BTreeMap<String, serde_json::Value>> {
    value["stream_metadata"]
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .filter(|v| v["medium"] != "data")
        .map(|v| Ok((v["id"].as_str().ok_or_else(invalid)?.to_owned(), v.clone())))
        .collect()
}
struct Candidate {
    row: rows::Saved,
    required: Vec<AuthorityScope>,
    until: i64,
    result: serde_json::Value,
    revision: u64,
}
impl NativeStreamCommitted {
    fn data_shape(&self) -> bool {
        match &self.outcome {
            CommittedOutcome::Gates { streams, .. } => {
                self.lease.command == Command::Update
                    && self
                        .lease
                        .gate_payload
                        .as_ref()
                        .is_some_and(|p| p["streams"].is_array())
                    && !streams.is_empty()
                    && streams.iter().all(|s| s.binding.medium == "data")
            }
            CommittedOutcome::Streams {
                projection: Some(_),
                unchanged_av: Some(_),
                ..
            } => self.lease.command == Command::Subscribe,
            _ => false,
        }
    }
    fn data_result(
        &self,
        c: &Connection,
        store: &Store,
    ) -> Result<Option<(rows::Saved, serde_json::Value, u64)>> {
        let lease = &self.lease;
        let row = exact(c, store, lease)?;
        let result = match &self.outcome {
            CommittedOutcome::Gates { revision, streams } => {
                let Some(payload) = lease.gate_payload.as_ref() else {
                    return Ok(None);
                };
                if streams.is_empty()
                    || streams.iter().any(|s| s.binding.medium != "data")
                    || payload["streams"].as_array().is_none()
                {
                    return Ok(None);
                }
                connect_protocol::native_gates::validate_observation(payload, streams)?;
                (
                    serde_json::json!({"connection_id":lease.connection,"connection_generation":lease.generation,"stream_revision":revision,"resources_changed":false,"action":payload["action"],"gate_streams":streams}),
                    *revision,
                )
            }
            CommittedOutcome::Streams {
                revision,
                resources_changed,
                projection: Some(bindings),
                unchanged_av: Some(witness),
            } => {
                if lease.command != Command::Subscribe {
                    return Ok(None);
                }
                let intent: Intent =
                    serde_json::from_str(row.intent.as_deref().ok_or_else(Error::storage)?)
                        .map_err(|_| Error::storage())?;
                let Some(selectors) = intent.payload["subscriptions"].as_array() else {
                    return Ok(None);
                };
                // Wire selectors are remote source IDs, not local receiver
                // binding IDs. Their Data identity is proved by the actual Core
                // commit witness against this exact persisted selector list.
                if selectors.is_empty() {
                    return Ok(None);
                }
                let mut selected_stream_ids = Vec::with_capacity(selectors.len());
                for selector in selectors {
                    let Some(object) = selector.as_object() else {
                        return Ok(None);
                    };
                    let Some(id) = selector["strm_id"].as_str() else {
                        return Ok(None);
                    };
                    if object.len() != 1 {
                        return Ok(None);
                    }
                    connect_protocol::transitions::id(id)?;
                    selected_stream_ids.push(id.to_owned());
                }
                let scope = NativeDataSubscriptionScope {
                    conversation_id: row.record.conversation_id.as_deref().ok_or_else(invalid)?,
                    session_id: row.record.session_id.as_deref().ok_or_else(invalid)?,
                    operation_id: &row.record.operation_id,
                    record_id: &lease.record,
                    parent_record_id: &lease.parent,
                    connection_id: &lease.connection,
                    runtime_connection_id: &lease.core_connection,
                    generation: lease.generation,
                    expected_revision: lease.expected_revision,
                    stream_revision: *revision,
                    selected_stream_ids: &selected_stream_ids,
                };
                if !witness.matches(&scope) {
                    return Ok(None);
                }
                let observed = connect_protocol::native_streams::SelectionObservation {
                    connection_id: lease.connection.clone(),
                    command_type: lease.command,
                    connection_generation: lease.generation,
                    expected_revision: lease.expected_revision,
                    stream_revision: *revision,
                    resources_changed: *resources_changed,
                    stream_bindings: Some(bindings.clone()),
                };
                observed.validate()?;
                (
                    serde_json::to_value(observed).map_err(|_| Error::storage())?,
                    *revision,
                )
            }
            _ => return Ok(None),
        };
        Ok(Some((row, result.0, result.1)))
    }
    fn data_candidate(&self, store: &Store, c: &Connection) -> Result<Option<Candidate>> {
        let Some((row, result, revision)) = self.data_result(c, store)? else {
            return Ok(None);
        };
        let lease = &self.lease;
        if row.record.state != OperationState::Preparing
            || row.record.cancel_requested
            || row.tombstoned
            || row.record.result.is_some()
            || row.record.problem.is_some()
            || crate::now() >= lease.expires_at
        {
            return Ok(None);
        }
        let source:Option<(String,String,String,i64)>=c.query_row("SELECT n.credential_id,n.key_id,n.device_id,n.source_expires_at FROM native_stream_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.state='preparing' AND o.lease_owner=?2 AND o.worker_runtime=?3 AND o.worker_generation=1 AND o.worker_cleanup=0 AND o.lease_expires_at=o.expires_at AND n.closed=0 AND n.effect_started=1 AND n.attempt_id=?2 AND n.attempt_runtime=?3 AND n.effect_id=?4 AND n.source_expires_at>?5 AND e.provider='native-stream' AND e.correlation_key=o.record_id AND e.idempotency_mode='none' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.observation_json IS NULL AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM session_codec_targets b WHERE b.native_operation_record_id=o.record_id)",params![lease.record,lease.attempt,lease.runtime,lease.effect,crate::now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let Some((credential, key, device, source_until)) = source else {
            return Ok(None);
        };
        let intent: Intent =
            serde_json::from_str(row.intent.as_deref().ok_or_else(Error::storage)?)
                .map_err(|_| Error::storage())?;
        let facts =
            store.native_stream_data_control_readers(c, &credential, &key, &device, &intent);
        let (projection, mut required, current_until) = match facts {
            Ok(v) => v,
            Err(e) if matches!(e.code.as_str(), "forbidden" | "unauthorized" | "not-found") => {
                return Ok(None)
            }
            Err(e) => return Err(e),
        };
        if projection.parent_record_id != lease.parent
            || projection.connection.runtime_connection_id != lease.core_connection
            || projection.connection.connection.generation != lease.generation
            || projection.stream_revision != lease.expected_revision
        {
            return Ok(None);
        }
        if let CommittedOutcome::Gates { streams, .. } = &self.outcome {
            for stream in streams {
                let known:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_snapshots s,json_each(s.projection_json,'$.stream_metadata') j WHERE s.connection_id=?1 AND s.generation=?2 AND s.stream_revision=?3 AND s.available=1 AND json_extract(j.value,'$.id')=?4 AND json_extract(j.value,'$.medium')='data' AND json_extract(j.value,'$.generation')=?5 AND json_extract(j.value,'$.state')!='ended')",params![lease.connection,lease.generation,lease.expected_revision,stream.binding.stream_id,stream.binding.stream_generation],|r|r.get(0)).map_err(|_|Error::storage())?;
                let offered = projection.connection.offer["streams_offered"]
                    .as_array()
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|v| v["id"] == stream.binding.stream_id && v["kind"] == "data")
                    });
                if !known && !offered {
                    return Ok(None);
                }
            }
        }
        if !crate::session_participant_lifecycle::reconcile_is_unchanged(
            c,
            store,
            row.record.session_id.as_deref().ok_or_else(invalid)?,
        )? {
            return Ok(None);
        }
        if required.iter().any(|scope| scope != &scope.read_scope()) {
            return Ok(None);
        }
        required.push(AuthorityScope::record(&lease.record, AuthorityMode::Write)?);
        let until = lease.expires_at.min(source_until).min(current_until);
        if crate::now() >= until {
            return Ok(None);
        }
        Ok(Some(Candidate {
            row,
            required: AuthorityScope::merged(&required)?,
            until,
            result,
            revision,
        }))
    }
    /// The authoritative builder is called afresh by the same-Unit constructor;
    /// callers cannot supply a writable row allowlist or broaden its purpose.
    pub(crate) fn data_control_plan(&self, store: &Store, c: &Connection) -> Result<Option<Plan>> {
        let Some(candidate) = self.data_candidate(store, c)? else {
            return Ok(None);
        };
        let lease = &self.lease;
        let mut changes = Vec::new();
        let effect = Row::load(c, Table::Effects, &lease.effect)?;
        let mut confirmed = effect.clone();
        confirmed.set("state", Value::Text("confirmed".into()))?;
        changes.push(Change::new(c, &effect, &confirmed, false)?);
        let connection = Row::load(c, Table::Connections, &lease.connection)?;
        if !matches!(connection.text("state")?, "ready" | "active")
            || connection.integer("generation")?
                != i64::try_from(lease.generation).map_err(|_| invalid())?
            || connection.text("observed_runtime_id")? != lease.runtime
        {
            return Ok(None);
        }
        let original_readiness = connection.text("readiness_json")?;
        let mut ready = connection.clone();
        let advanced = json(
            c,
            "SELECT json_set(?1,'$.stream_revision',?2)",
            params![original_readiness, candidate.revision],
        )?;
        ready.set("readiness_json", Value::Text(advanced))?;
        if ready.get("readiness_json")? != connection.get("readiness_json")? {
            changes.push(Change::new(c, &connection, &ready, false)?);
        }
        if matches!(self.outcome, CommittedOutcome::Gates { .. }) {
            let mut gate = ready.clone();
            let exact_result = candidate.result.to_string();
            gate.set(
                "readiness_json",
                Value::Text(json(
                    c,
                    "SELECT json_set(?1,'$.last_gate',json(?2))",
                    params![ready.text("readiness_json")?, exact_result],
                )?),
            )?;
            changes.push(Change::new(c, &ready, &gate, false)?);
        }
        let mut descriptor = "unreported";
        let existing:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_snapshots WHERE connection_id=?1 AND generation=?2 AND stream_revision=?3 AND available=1)",params![lease.connection,lease.generation,lease.expected_revision],|r|r.get(0)).map_err(|_|Error::storage())?;
        if existing {
            descriptor = "current";
            let old = Row::load(c, Table::Snapshots, &lease.connection)?;
            let old_value: serde_json::Value =
                match serde_json::from_str(old.text("projection_json")?) {
                    Ok(v) => v,
                    Err(_) => return Ok(None),
                };
            if crate::native_stream_projection::validate_snapshot(
                &old_value,
                u64::try_from(old.integer("generation")?).map_err(|_| invalid())?,
                u64::try_from(old.integer("stream_revision")?).map_err(|_| invalid())?,
            )
            .is_err()
            {
                return Ok(None);
            }
            let mut next = old_value.clone();
            match &self.outcome {
                CommittedOutcome::Gates { streams, .. } => {
                    let counter = next["stream_metadata_counter"]
                        .as_u64()
                        .and_then(|v| v.checked_add(1))
                        .filter(|v| *v <= 9_007_199_254_740_991)
                        .ok_or_else(invalid)?;
                    let records = next["stream_metadata"].as_array_mut().ok_or_else(invalid)?;
                    for stream in streams {
                        let Some(record) = records.iter_mut().find(|r| {
                            r["id"] == stream.binding.stream_id
                                && r["generation"] == stream.binding.stream_generation
                                && r["medium"] == "data"
                                && r["state"] != "ended"
                        }) else {
                            return Ok(None);
                        };
                        record["state"] = serde_json::json!(stream.state);
                        record["revision"] = serde_json::json!(counter);
                    }
                    next["stream_metadata_counter"] = serde_json::json!(counter);
                    next["stream_metadata_revision"] = serde_json::json!(candidate.revision);
                }
                CommittedOutcome::Streams {
                    projection: Some(bindings),
                    ..
                } => {
                    crate::native_stream_projection::replace(
                        &mut next,
                        lease.generation,
                        Some(lease.expected_revision),
                        candidate.revision,
                        bindings,
                    )?;
                }
                _ => return Ok(None),
            }
            if av_records(&old_value)? != av_records(&next)? {
                return Ok(None);
            }
            let mut updated = old.clone();
            updated.set(
                "stream_revision",
                Value::Integer(i64::try_from(candidate.revision).map_err(|_| invalid())?),
            )?;
            updated.set("projection_json", Value::Text(next.to_string()))?;
            if updated.get("projection_json")? != old.get("projection_json")?
                || updated.get("stream_revision")? != old.get("stream_revision")?
            {
                changes.push(Change::new(c, &old, &updated, false)?);
            }
        } else if let CommittedOutcome::Streams {
            projection: Some(bindings),
            ..
        } = &self.outcome
        {
            match Row::optional(c, Table::Snapshots, &lease.connection)? {
                None => {
                    // A fresh replacement has no old descriptor. Actual sealed
                    // Core bindings create only bounded observational metadata.
                    descriptor = "first";
                    let mut value = serde_json::json!({});
                    crate::native_stream_projection::replace(
                        &mut value,
                        lease.generation,
                        Some(lease.expected_revision),
                        candidate.revision,
                        bindings,
                    )?;
                    let inserted = Row::new(
                        Table::Snapshots,
                        vec![
                            Value::Text(lease.connection.clone()),
                            Value::Integer(i64::try_from(lease.generation).map_err(|_| invalid())?),
                            Value::Integer(
                                i64::try_from(candidate.revision).map_err(|_| invalid())?,
                            ),
                            Value::Integer(1),
                            Value::Text(value.to_string()),
                        ],
                    )?;
                    changes.push(Change::insert(c, &inserted)?);
                }
                Some(old)
                    if old.integer("generation")?
                        == i64::try_from(lease.generation).map_err(|_| invalid())?
                        && old.integer("stream_revision")?
                            < i64::try_from(lease.expected_revision).map_err(|_| invalid())? =>
                {
                    // snapshot() hides this strictly OLDER same-generation row.
                    // Rebuild the same non-grant descriptor as generic complete:
                    // replace(current=false) resets observations to negotiated,
                    // never invents active gates or assumes benign old drift.
                    descriptor = "stale-invisible";
                    let mut value: serde_json::Value =
                        match serde_json::from_str(old.text("projection_json")?) {
                            Ok(v) => v,
                            Err(_) => return Ok(None),
                        };
                    if crate::native_stream_projection::validate_snapshot(
                        &value,
                        u64::try_from(old.integer("generation")?).map_err(|_| invalid())?,
                        u64::try_from(old.integer("stream_revision")?).map_err(|_| invalid())?,
                    )
                    .is_err()
                    {
                        return Ok(None);
                    }
                    if crate::native_stream_projection::replace(
                        &mut value,
                        lease.generation,
                        Some(lease.expected_revision),
                        candidate.revision,
                        bindings,
                    )
                    .is_err()
                    {
                        return Ok(None);
                    }
                    let mut updated = old.clone();
                    updated.set(
                        "stream_revision",
                        Value::Integer(i64::try_from(candidate.revision).map_err(|_| invalid())?),
                    )?;
                    updated.set("available", Value::Integer(1))?;
                    updated.set("projection_json", Value::Text(value.to_string()))?;
                    changes.push(Change::new(c, &old, &updated, false)?);
                }
                Some(_) => return Ok(None),
            }
        }
        let root = Row::load(c, Table::Operations, &lease.record)?;
        let touched = crate::now();
        let mut committing = root.clone();
        committing.set("state", Value::Text("committing".into()))?;
        committing.set(
            "revision",
            Value::Integer(
                root.integer("revision")?
                    .checked_add(1)
                    .ok_or_else(invalid)?,
            ),
        )?;
        committing.set("updated_at", Value::Integer(touched))?;
        changes.push(Change::new(c, &root, &committing, true)?);
        let mut completed = committing.clone();
        completed.set("state", Value::Text("completed".into()))?;
        completed.set(
            "revision",
            Value::Integer(
                committing
                    .integer("revision")?
                    .checked_add(1)
                    .ok_or_else(invalid)?,
            ),
        )?;
        completed.set("result_json", Value::Text(candidate.result.to_string()))?;
        changes.push(Change::new(c, &committing, &completed, true)?);
        let stream = Row::load(c, Table::StreamOperations, &lease.record)?;
        let mut closed = stream.clone();
        closed.set("closed", Value::Integer(1))?;
        changes.push(Change::new(c, &stream, &closed, false)?);
        let mut settled = completed.clone();
        for col in ["lease_owner", "lease_expires_at", "worker_runtime"] {
            settled.set(col, Value::Null)?;
        }
        settled.set(
            "worker_generation",
            Value::Integer(
                completed
                    .integer("worker_generation")?
                    .checked_add(1)
                    .filter(|v| *v <= 9_007_199_254_740_991)
                    .ok_or_else(invalid)?,
            ),
        )?;
        changes.push(Change::new(c, &completed, &settled, false)?);
        Ok(Some(Plan {
            key: phase_ledger::Key {
                record: lease.record.clone(),
                phase: "data-control",
                attempt: lease.attempt.clone(),
                effect: lease.effect.clone(),
                expected: candidate.result.to_string(),
                runtime: lease.runtime.clone(),
                reservation: lease.parent.clone(),
                digest: candidate.row.digest,
                stage: phase_ledger::Stage::Confirm,
            },
            required: candidate.required,
            until: candidate.until,
            descriptor,
            rows: changes,
        }))
    }
}
impl Store {
    pub(super) fn complete_data_control_native_stream(
        &self,
        receipt: &NativeStreamCommitted,
    ) -> Result<Option<OperationRecord>> {
        // Queued Data and every conservative outcome retain their original
        // admission path without another SQL interval or runtime-owner lookup.
        if !receipt.data_shape() {
            return Ok(None);
        }
        let owner = self.require_runtime_owner()?;
        let preview = {
            let c = self.try_connection()?;
            let Some((row, result, _)) = receipt.data_result(&c, self)? else {
                return Ok(None);
            };
            if row.record.state == OperationState::Completed {
                Some((true, admission::row_scopes(&row)?, result))
            } else {
                receipt
                    .data_candidate(self, &c)?
                    .map(|v| (false, v.required, v.result))
            }
        };
        let Some((preview_completed, required, result)) = preview else {
            return Ok(None);
        };
        #[cfg(test)]
        if let Some(hook) = BEFORE_ADMISSION.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
        let authority = self.try_acquire_authority(&required)?;
        let mut c = self.try_connection()?;
        // A second owner of this exact immutable receipt may have completed it
        // after preview. Decide read-only replay from the ACTUAL locked primary
        // row, never fall through to a broad Writer on a known completion.
        let completed = exact(&c, self, &receipt.lease)?.record.state == OperationState::Completed;
        if preview_completed && !completed {
            return Err(conflict());
        }
        if completed {
            let tx = c
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(|_| Error::storage())?;
            let row = exact(&tx, self, &receipt.lease)?;
            let settled:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.closed=1 AND e.state='confirmed')",[&receipt.lease.record],|r|r.get(0)).map_err(|_|Error::storage())?;
            if row.record.state != OperationState::Completed
                || row.record.result.as_ref() != Some(&result)
                || !settled
                || !self.authority_covers(&authority, &admission::row_scopes(&row)?)
                || !owner.current()
            {
                return Err(conflict());
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(Some(row.record));
        }
        c.bind_available_mutation_authority(self, &authority)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let Some((permit, plan)) = self
            .authority_writer_boundary
            .native_data_control_ledger(self, &tx, &authority, receipt)?
        else {
            return Ok(None);
        };
        for change in &plan.rows {
            change.apply(&tx)?;
            if change.journal {
                rows::event(&tx, &receipt.lease.record, "operation_updated", self)?;
            }
        }
        permit.require_data_control_complete()?;
        plan.verify_final(&tx)?;
        let record = rows::by_record(&tx, &receipt.lease.record)?.record;
        let until = plan.until;
        let descriptor = plan.descriptor;
        drop(permit);
        if !owner.current() {
            return Err(Error::forbidden());
        }
        if crate::now() >= until {
            return Ok(None);
        }
        tx.commit().map_err(|_| Error::storage())?;
        drop(c);
        drop(authority);
        // One control-only record after actual COMMIT, never a packet log or
        // pre-commit success. Original record IDs deduplicate metadata replays.
        let kind = match &receipt.outcome {
            CommittedOutcome::Gates { .. } => "data-gate",
            CommittedOutcome::Streams { .. } => "data-subscription",
            _ => unreachable!("closed Data control plan variant"),
        };
        tracing::debug!(target: "connect_store::native_data_control", kind, descriptor,
            record_id = %record.record_id, "Captured Data-only control recorded");
        Ok(Some(record))
    }
}
