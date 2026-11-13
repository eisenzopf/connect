//! Exact closed row deltas for a single registered handoff SQL transaction.
//! This permit neither authorizes media nor turns a connection fence into a
//! conversation fence. Semantic phase builders validate immutable receipts and
//! current authorization before constructing this complete immutable plan.
use super::*;
use rusqlite::types::Value;
use std::collections::BTreeMap;
mod retirement;
pub(crate) use retirement::retire_plan;
mod strict_validation;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Table {
    Operations,
    Effects,
    ReplacementSteps,
    Connections,
    Offers,
    Sessions,
    NativeConnectionLifetimes,
    NativeReplacementLifetimes,
}
impl Table {
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "operations" => Some(Self::Operations),
            "operation_effects" => Some(Self::Effects),
            "native_replacement_steps" => Some(Self::ReplacementSteps),
            "connections" => Some(Self::Connections),
            "connection_offers" => Some(Self::Offers),
            "sessions" => Some(Self::Sessions),
            "native_connection_lifetimes" => Some(Self::NativeConnectionLifetimes),
            "native_replacement_lifetimes" => Some(Self::NativeReplacementLifetimes),
            _ => None,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Operations => "operations",
            Self::Effects => "operation_effects",
            Self::ReplacementSteps => "native_replacement_steps",
            Self::Connections => "connections",
            Self::Offers => "connection_offers",
            Self::Sessions => "sessions",
            Self::NativeConnectionLifetimes => "native_connection_lifetimes",
            Self::NativeReplacementLifetimes => "native_replacement_lifetimes",
        }
    }
    pub(crate) fn columns(self) -> &'static [&'static str] {
        match self {
            Self::Operations => &[
                "record_id",
                "tenant_id",
                "owner_id",
                "operation_id",
                "digest",
                "command_type",
                "conversation_id",
                "session_id",
                "connection_id",
                "expected_revision",
                "intent_json",
                "state",
                "revision",
                "result_json",
                "problem_json",
                "tombstoned",
                "worker_generation",
                "lease_owner",
                "lease_expires_at",
                "created_at",
                "updated_at",
                "expires_at",
                "ledger_version",
                "worker_runtime",
                "worker_cleanup",
                "cancel_requested",
            ],
            Self::Effects => &[
                "id",
                "operation_record_id",
                "provider",
                "correlation_key",
                "idempotency_mode",
                "state",
                "attempt",
                "provider_ref",
                "observation_json",
                "submission_generation",
                "compensates_effect_id",
            ],
            Self::ReplacementSteps => &[
                "operation_record_id",
                "phase",
                "attempt_id",
                "effect_id",
                "started",
                "observation_json",
            ],
            Self::Connections => &[
                "id",
                "session_id",
                "participant_id",
                "membership_epoch",
                "identity_id",
                "tenant_id",
                "endpoint_id",
                "generation",
                "replaces",
                "state",
                "revision",
                "observed_runtime_id",
                "observed_at",
                "readiness_json",
            ],
            Self::Offers => &[
                "connection_id",
                "runtime_connection_id",
                "operation_id",
                "credential_id",
                "offer_json",
                "created_at",
            ],
            Self::Sessions => &[
                "id",
                "conversation_id",
                "initiator_id",
                "participant_id",
                "state",
                "media",
                "created_at",
                "updated_at",
                "ended_reason",
                "topology_revision",
                "audience_mode",
                "state_revision",
                "terminal_reason_code",
            ],
            Self::NativeConnectionLifetimes => &[
                "operation_record_id",
                "connection_id",
                "runtime_epoch",
                "generation",
                "retired",
            ],
            Self::NativeReplacementLifetimes => &[
                "operation_record_id",
                "credential_id",
                "key_id",
                "device_id",
                "endpoint_id",
                "participant_id",
                "membership_epoch",
                "generation",
                "resources_json",
                "source_expires_at",
                "attempt_runtime",
                "attempt_id",
                "effect_id",
            ],
        }
    }
    fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Operations => &["record_id"],
            Self::Effects => &["id"],
            Self::ReplacementSteps => &["operation_record_id", "phase"],
            Self::Connections => &["id"],
            Self::Offers => &["connection_id"],
            Self::Sessions => &["id"],
            Self::NativeConnectionLifetimes => &["operation_record_id"],
            Self::NativeReplacementLifetimes => &["operation_record_id"],
        }
    }
}
#[derive(Clone)]
pub(crate) struct Row {
    table: Table,
    values: Vec<Value>,
}
impl Row {
    pub(crate) fn new(table: Table, values: Vec<Value>) -> Result<Self> {
        if values.len() != table.columns().len() {
            return Err(Error::forbidden());
        }
        Ok(Self { table, values })
    }
    pub(crate) fn get(&self, column: &str) -> Result<&Value> {
        self.table
            .columns()
            .iter()
            .position(|v| *v == column)
            .map(|i| &self.values[i])
            .ok_or_else(Error::forbidden)
    }
    pub(crate) fn set(&mut self, column: &str, value: Value) -> Result<()> {
        let i = self
            .table
            .columns()
            .iter()
            .position(|v| *v == column)
            .ok_or_else(Error::forbidden)?;
        self.values[i] = value;
        Ok(())
    }
    fn key(&self) -> Result<Vec<Value>> {
        self.table
            .keys()
            .iter()
            .map(|k| self.get(k).cloned())
            .collect()
    }
}
/// The same typed SQL encoding is used for bound values and trigger OLD/NEW.
/// TEXT is compared byte-for-byte, including JSON whitespace/key order. The
/// concatenation strips only SQLite's JSON subtype, never parses TEXT as JSON.
fn typed(value: &str) -> String {
    format!(
        "json_array(typeof({value}),CASE typeof({value}) WHEN 'text' THEN {value}||'' WHEN 'blob' THEN hex({value}) ELSE {value} END)"
    )
}
pub(crate) fn trigger_tuple(table: Table, alias: &str) -> String {
    format!(
        "json_array({})",
        table
            .columns()
            .iter()
            .map(|c| typed(&format!("{alias}.\"{c}\"")))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn row_tuple(c: &Connection, row: &Row) -> Result<String> {
    let sql = format!(
        "SELECT json_array({})",
        (1..=row.values.len())
            .map(|i| typed(&format!("?{i}")))
            .collect::<Vec<_>>()
            .join(",")
    );
    c.prepare_cached(&sql)
        .map_err(|_| Error::storage())?
        .query_row(rusqlite::params_from_iter(row.values.iter()), |r| r.get(0))
        .map_err(|_| Error::storage())
}
pub(crate) fn row_snapshot(c: &Connection, table: Table, keys: &[Value]) -> Result<Option<Row>> {
    if keys.len() != table.keys().len() {
        return Err(Error::forbidden());
    }
    let columns = table
        .columns()
        .iter()
        .map(|c| format!("\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    let predicates = table
        .keys()
        .iter()
        .enumerate()
        .map(|(i, k)| format!("\"{k}\" IS ?{}", i + 1))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT {columns} FROM \"{}\" WHERE {predicates}",
        table.name()
    );
    c.prepare_cached(&sql)
        .map_err(|_| Error::storage())?
        .query_row(rusqlite::params_from_iter(keys.iter()), |r| {
            let values = (0..table.columns().len())
                .map(|i| r.get(i))
                .collect::<rusqlite::Result<Vec<Value>>>()?;
            Ok(Row { table, values })
        })
        .optional()
        .map_err(|_| Error::storage())
}
/// No delete exception is supported. PK changes are never a handoff delta.
pub(crate) struct Transition {
    table: Table,
    action: &'static str,
    old: Option<Row>,
    new: Row,
    tuple: String,
}
impl Transition {
    /// Owned synthetic row copies for constructor-refusal unit controls. No
    /// permission, mutable provenance or shipping interface is exposed.
    #[cfg(test)]
    pub(crate) fn rows_for_test(&self) -> (Table, Option<Row>, Row) {
        (self.table, self.old.clone(), self.new.clone())
    }
    pub(crate) fn new(c: &Connection, old: Option<&Row>, new: Option<&Row>) -> Result<Self> {
        let new = new.ok_or_else(Error::forbidden)?;
        if let Some(old) = old {
            if old.table != new.table || old.key()? != new.key()? {
                return Err(Error::forbidden());
            }
        }
        let old_tuple = old.map(|r| row_tuple(c, r)).transpose()?;
        let new_tuple = row_tuple(c, new)?;
        let tuple = c
            .prepare_cached("SELECT json_array(json(?1),json(?2))")
            .map_err(|_| Error::storage())?
            .query_row(rusqlite::params![old_tuple, new_tuple], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        Ok(Self {
            table: new.table,
            action: if old.is_some() { "update" } else { "insert" },
            old: old.cloned(),
            new: new.clone(),
            tuple,
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    CommitBegin,
    FinalReady,
    OriginalRetire,
    ReplacementRetire,
}
/// References to sealed Store provenance, never caller-provided row IDs.
pub(crate) enum Witness<'a> {
    Begin {
        origin: &'a crate::NativeReplacementMediaOrigin,
        read: &'a crate::NativeReplacementSceneRead,
    },
    Ready(&'a crate::NativeReplacementCommitted),
    OriginalRetire(&'a crate::NativeOfferLease),
    ReplacementRetire(&'a crate::NativeMediaOrigin),
}
/// Derived by each provenance type under this exact primary SQL transaction,
/// through its existing complete current/historical authorization boundary.
pub(crate) struct WitnessFacts {
    pub(crate) record: String,
    pub(crate) runtime: String,
    pub(crate) source: String,
    pub(crate) candidate: Option<String>,
    pub(crate) required: Vec<AuthorityScope>,
    pub(crate) until: Option<i64>,
    pub(crate) attempt: Option<String>,
    pub(crate) effect: Option<String>,
    pub(crate) expected: Option<String>,
    pub(crate) reservation: Option<String>,
    pub(crate) digest: Option<String>,
}
pub(crate) struct Plan<'a> {
    pub(crate) witness: Witness<'a>,
    pub(crate) key: Key,
    pub(crate) purpose: Purpose,
    pub(crate) source: String,
    pub(crate) candidate: Option<String>,
    pub(crate) required: Vec<AuthorityScope>,
    pub(crate) until: Option<i64>,
    pub(crate) transitions: Vec<Transition>,
}
pub(super) struct Capture {
    rows: Vec<Transition>,
    until: Option<i64>,
}
pub(super) fn matches(
    active: &Active,
    action: &str,
    record: &str,
    effect: Option<&str>,
    phase: Option<&str>,
    expected: Option<&str>,
    attempt: Option<&str>,
    revision: Option<i64>,
) -> bool {
    let Some(capture) = &active.scoped_capture else {
        return false;
    };
    if action == "scoped-row-active" {
        return phase == Some("scoped-handoff")
            && expected.is_none()
            && attempt.is_none()
            && revision.is_none()
            && capture.until.is_none_or(|until| crate::now() < until)
            && capture
                .rows
                .iter()
                .any(|row| record == row.table.name() && effect == Some(row.action));
    }
    action == "scoped-row"
        && phase == Some("scoped-handoff")
        && attempt.is_none()
        && revision.is_none()
        && capture.until.is_none_or(|until| crate::now() < until)
        && capture.rows.iter().any(|row| {
            record == row.table.name()
                && effect == Some(row.action)
                && expected == Some(row.tuple.as_str())
        })
}

impl Purpose {
    fn phase(self) -> &'static str {
        match self {
            Self::CommitBegin => "scoped-commit-begin",
            Self::FinalReady => "scoped-final-ready",
            Self::OriginalRetire => "scoped-original-retire",
            Self::ReplacementRetire => "scoped-replacement-retire",
        }
    }
    fn cleanup(self) -> bool {
        matches!(self, Self::OriginalRetire | Self::ReplacementRetire)
    }
}
fn text<'a>(row: &'a Row, column: &str) -> Result<&'a str> {
    match row.get(column)? {
        Value::Text(value) => Ok(value),
        _ => Err(Error::forbidden()),
    }
}
fn integer(row: &Row, column: &str) -> Result<i64> {
    match row.get(column)? {
        Value::Integer(value) => Ok(*value),
        _ => Err(Error::forbidden()),
    }
}
fn changed_only(old: &Row, new: &Row, allowed: &[&str]) -> bool {
    old.table == new.table
        && old
            .table
            .columns()
            .iter()
            .zip(old.values.iter().zip(&new.values))
            .all(|(name, (a, b))| a == b || allowed.contains(name))
}
struct Facts {
    conversation: String,
    session: String,
    revision: i64,
}
fn plan_facts(store: &Store, c: &Connection, p: &Plan<'_>) -> Result<Facts> {
    for id in [&p.key.record, &p.key.attempt, &p.key.effect] {
        connect_protocol::transitions::id(id).map_err(|_| Error::forbidden())?;
    }
    if p.key.phase != p.purpose.phase()
        || p.key.runtime != store.runtime_epoch()
        || !store.require_runtime_owner()?.current()
        || (p.purpose == Purpose::CommitBegin) != (p.key.stage == Stage::Begin)
        || p.until.is_some_and(|until| crate::now() >= until)
    {
        return Err(Error::forbidden());
    }
    if p.purpose.cleanup() {
        if p.candidate.is_some() || p.until.is_some() {
            return Err(Error::forbidden());
        }
        let original = p.purpose == Purpose::OriginalRetire;
        let row:Option<(String,String,i64)>=c.query_row(
            "SELECT o.conversation_id,o.session_id,o.revision FROM native_admission_sources n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.state='completed' AND o.tombstoned=0 AND n.effect_started=1 AND n.attempt_id=?2 AND n.effect_id=?3 AND n.attempt_runtime=?4 AND n.connection_id=?5 AND e.state='confirmed' AND ((?6 AND EXISTS(SELECT 1 FROM native_offer_operations original WHERE original.operation_record_id=o.record_id)) OR (NOT ?6 AND EXISTS(SELECT 1 FROM native_replacement_lifetimes replacement WHERE replacement.operation_record_id=o.record_id)))",
            rusqlite::params![p.key.record,p.key.attempt,p.key.effect,p.key.runtime,p.source,original],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(|_|Error::storage())?;
        let (conversation, session, revision) = row.ok_or_else(Error::forbidden)?;
        return Ok(Facts {
            conversation,
            session,
            revision,
        });
    }
    let candidate = p.candidate.as_ref().ok_or_else(Error::forbidden)?;
    if candidate == &p.source || p.until.is_none() {
        return Err(Error::forbidden());
    }
    let row:Option<(String,String,i64,i64)>=c.query_row(
        "SELECT o.conversation_id,o.session_id,o.revision,min(o.expires_at,n.expires_at) FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.command_type='connection.replace' AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND o.connection_id=?2 AND n.candidate_connection_id=?3 AND n.runtime_epoch=?4 AND n.reservation_id=?5 AND n.plan_digest=?6 AND o.worker_runtime=?7 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND o.expires_at>?8 AND n.expires_at>?8",
        rusqlite::params![p.key.record,p.source,candidate,p.key.runtime,p.key.reservation,p.key.digest,store.transition_runtime,crate::now()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(|_|Error::storage())?;
    let (conversation, session, revision, cutoff) = row.ok_or_else(Error::forbidden)?;
    if p.until.is_none_or(|until| until > cutoff) {
        return Err(Error::forbidden());
    }
    let phase_ok:bool=if p.purpose==Purpose::CommitBegin {
        c.query_row("SELECT NOT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit') AND NOT EXISTS(SELECT 1 FROM operation_effects WHERE id=?2) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps WHERE attempt_id=?3 OR effect_id=?2) AND EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.started=1 AND s.observation_json IS NOT NULL AND e.state='confirmed')",rusqlite::params![p.key.record,p.key.effect,p.key.attempt],|r|r.get(0))
    } else {
        c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='commit' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json IS NOT NULL AND e.state='confirmed' AND e.observation_json=?4)",rusqlite::params![p.key.record,p.key.attempt,p.key.effect,p.key.expected],|r|r.get(0))
    }.map_err(|_|Error::storage())?;
    if !phase_ok {
        return Err(Error::forbidden());
    }
    Ok(Facts {
        conversation,
        session,
        revision,
    })
}
fn validate_transition(p: &Plan<'_>, f: &Facts, t: &Transition) -> Result<()> {
    let row = &t.new;
    let fail = || Error::forbidden();
    let owned_connection = |id: &str| id == p.source || p.candidate.as_deref() == Some(id);
    let column_changes = match t.table {
        Table::Operations => {
            if text(row, "record_id")? != p.key.record {
                return Err(fail());
            }
            if p.purpose == Purpose::FinalReady {
                vec![
                    "state",
                    "result_json",
                    "lease_owner",
                    "lease_expires_at",
                    "worker_runtime",
                    "worker_generation",
                    "revision",
                    "updated_at",
                ]
            } else if p.purpose.cleanup() {
                vec![
                    "lease_owner",
                    "lease_expires_at",
                    "worker_runtime",
                    "worker_generation",
                ]
            } else {
                vec!["revision", "updated_at"]
            }
        }
        Table::Effects => {
            if p.purpose != Purpose::CommitBegin
                || text(row, "id")? != p.key.effect
                || text(row, "operation_record_id")? != p.key.record
                || text(row, "provider")? != "native-replacement-commit"
                || text(row, "correlation_key")? != p.key.effect
                || text(row, "idempotency_mode")? != "none"
                || text(row, "observation_json")? != p.key.expected
            {
                return Err(fail());
            }
            vec!["state", "attempt", "submission_generation"]
        }
        Table::ReplacementSteps => {
            if p.purpose != Purpose::CommitBegin
                || text(row, "operation_record_id")? != p.key.record
                || text(row, "phase")? != "commit"
                || text(row, "attempt_id")? != p.key.attempt
                || text(row, "effect_id")? != p.key.effect
                || row.get("observation_json")? != &Value::Null
            {
                return Err(fail());
            }
            vec!["started"]
        }
        Table::Connections => {
            if p.purpose == Purpose::CommitBegin
                || !owned_connection(text(row, "id")?)
                || text(row, "session_id")? != f.session
            {
                return Err(fail());
            }
            vec![
                "generation",
                "replaces",
                "state",
                "revision",
                "observed_runtime_id",
                "observed_at",
                "readiness_json",
            ]
        }
        Table::Offers => {
            if p.purpose != Purpose::FinalReady
                || p.candidate.as_deref() != Some(text(row, "connection_id")?)
            {
                return Err(fail());
            }
            vec![
                "runtime_connection_id",
                "operation_id",
                "credential_id",
                "offer_json",
                "created_at",
            ]
        }
        Table::Sessions => {
            if p.purpose != Purpose::FinalReady
                || text(row, "id")? != f.session
                || text(row, "conversation_id")? != f.conversation
            {
                return Err(fail());
            }
            vec!["topology_revision", "updated_at"]
        }
        Table::NativeConnectionLifetimes => {
            if p.purpose == Purpose::CommitBegin || !owned_connection(text(row, "connection_id")?) {
                return Err(fail());
            }
            if t.old.is_none()
                && (p.purpose != Purpose::FinalReady
                    || text(row, "operation_record_id")? != p.key.record
                    || p.candidate.as_deref() != Some(text(row, "connection_id")?)
                    || integer(row, "retired")? != 0)
            {
                return Err(fail());
            }
            vec!["retired"]
        }
        Table::NativeReplacementLifetimes => {
            if p.purpose != Purpose::FinalReady
                || t.old.is_some()
                || text(row, "operation_record_id")? != p.key.record
                || text(row, "attempt_id")? != p.key.attempt
                || text(row, "effect_id")? != p.key.effect
                || text(row, "attempt_runtime")? != p.key.runtime
            {
                return Err(fail());
            }
            vec![]
        }
    };
    if let Some(old) = &t.old {
        if !changed_only(old, row, &column_changes) {
            return Err(fail());
        }
        match t.table {
            Table::Sessions => {
                if integer(row, "topology_revision")?
                    != integer(old, "topology_revision")?
                        .checked_add(1)
                        .ok_or_else(fail)?
                {
                    return Err(fail());
                }
            }
            Table::Connections => {
                let retiring = text(row, "id")? == p.source;
                if retiring {
                    if text(row, "state")? != "retired"
                        || !["ready", "active", "held"].contains(&text(old, "state")?)
                        || integer(row, "generation")?
                            != integer(old, "generation")?
                                .checked_add(1)
                                .ok_or_else(fail)?
                        || integer(row, "revision")?
                            != integer(old, "revision")?.checked_add(1).ok_or_else(fail)?
                        || ["observed_runtime_id", "observed_at", "readiness_json"]
                            .iter()
                            .any(|k| !row.get(k).is_ok_and(|v| v == &Value::Null))
                        || row.get("replaces")? != old.get("replaces")?
                    {
                        return Err(fail());
                    }
                } else if text(old, "state")? != "retired"
                    || text(row, "state")? != "ready"
                    || text(row, "replaces")? != p.source
                {
                    return Err(fail());
                }
            }
            Table::NativeConnectionLifetimes => {
                if integer(row, "retired")? != 1 {
                    return Err(fail());
                }
            }
            _ => {}
        }
    } else if !matches!(
        (p.purpose, t.table),
        (
            Purpose::CommitBegin,
            Table::Effects | Table::ReplacementSteps
        ) | (
            Purpose::FinalReady,
            Table::Connections
                | Table::Offers
                | Table::NativeConnectionLifetimes
                | Table::NativeReplacementLifetimes
        )
    ) {
        return Err(fail());
    }
    Ok(())
}
fn witness_facts(
    store: &Store,
    c: &Connection,
    proof: &Arc<AuthorityGuardSet>,
    plan: &Plan<'_>,
    resolved_ready: Option<(&crate::NativeReplacementCommitted, WitnessFacts)>,
) -> Result<WitnessFacts> {
    let facts = if let Some((captured, facts)) = resolved_ready {
        match (&plan.witness, plan.purpose) {
            (Witness::Ready(actual), Purpose::FinalReady) if std::ptr::eq(captured, *actual) => {
                facts
            }
            _ => return Err(Error::forbidden()),
        }
    } else {
        match (&plan.witness, plan.purpose) {
            (Witness::Begin { origin, read }, Purpose::CommitBegin) => {
                origin.scoped_commit_begin_facts(store, c, read)?
            }
            (Witness::Ready(capture), Purpose::FinalReady) => {
                capture.scoped_final_ready_facts(store, c, proof)?
            }
            (Witness::OriginalRetire(lease), Purpose::OriginalRetire) => {
                lease.scoped_original_retire_facts(store, c)?
            }
            (Witness::ReplacementRetire(origin), Purpose::ReplacementRetire) => {
                origin.scoped_replacement_retire_facts(store, c)?
            }
            _ => return Err(Error::forbidden()),
        }
    };
    if facts.record != plan.key.record
        || facts.runtime != plan.key.runtime
        || facts.source != plan.source
        || facts.candidate != plan.candidate
        || facts
            .attempt
            .as_ref()
            .is_some_and(|v| v != &plan.key.attempt)
        || facts.effect.as_ref().is_some_and(|v| v != &plan.key.effect)
        || facts
            .expected
            .as_ref()
            .is_some_and(|v| v != &plan.key.expected)
        || facts.reservation.as_deref().unwrap_or("") != plan.key.reservation
        || facts.digest.as_deref().unwrap_or("") != plan.key.digest
        || match (facts.until, plan.until) {
            (Some(actual), Some(planned)) => planned > actual || crate::now() >= planned,
            (None, None) => false,
            _ => true,
        }
    {
        return Err(Error::forbidden());
    }
    Ok(facts)
}
/// Closed publication values only: no SQL handle, callback, or authority facts.
/// The fixed owner planner and strict validator bind these to the same capture.
#[derive(Clone)]
pub(crate) struct ReadyPlanInputs {
    pub descriptor: serde_json::Value,
    pub intent: connect_protocol::connection_control::OfferIntent,
    pub readiness: String,
    pub result: serde_json::Value,
    pub topology: u64,
    pub recorded_at: i64,
    pub hint_until: i64,
}
impl State {
    /// Derive fresh Ready facts once, then invoke only the fixed read-only owner
    /// planner. No caller closure, reusable facts, or preview token is accepted.
    pub(crate) fn native_final_ready_mutation_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        captured: &crate::NativeReplacementCommitted,
        inputs: ReadyPlanInputs,
    ) -> Result<Option<(Permit<'a>, i64)>> {
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || c.is_autocommit()
            || c.transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
        {
            return Err(Error::forbidden());
        }
        {
            let slot = self.slot.lock().map_err(|_| Error::storage())?;
            if !slot.active
                || slot.failed
                || slot.connection != c as *const Connection as usize
                || slot.birth.is_some()
                || slot.phase_ledger.is_some()
            {
                return Err(Error::forbidden());
            }
        }
        let witness = match captured.scoped_final_ready_facts(store, c, proof) {
            Ok(facts) => facts,
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "forbidden" | "unauthorized" | "conflict" | "not-ready"
                ) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ready-current-witness");
        let plan = captured.scoped_final_ready_plan(c, &witness, inputs)?;
        let until = plan.until.ok_or_else(Error::forbidden)?;
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ready-pure-plan");
        self.scoped_mutation_ledger_impl(store, c, proof, plan, Some((captured, witness)))
            .map(|permit| Some((permit, until)))
    }
    /// General/test callers still independently derive their opaque witness.
    pub(crate) fn native_scoped_mutation_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        plan: Plan<'_>,
    ) -> Result<Permit<'a>> {
        self.scoped_mutation_ledger_impl(store, c, proof, plan, None)
    }
    /// The plan is complete before installation. SQLite callbacks only compare
    /// immutable bytes; no SQL, guard acquisition, or policy refresh occurs there.
    fn scoped_mutation_ledger_impl<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        plan: Plan<'_>,
        resolved_ready: Option<(&crate::NativeReplacementCommitted, WitnessFacts)>,
    ) -> Result<Permit<'a>> {
        let tag = c as *const Connection as usize;
        let available = |slot: &Slot| {
            slot.active
                && !slot.failed
                && slot.connection == tag
                && slot.birth.is_none()
                && slot.phase_ledger.is_none()
        };
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || c.is_autocommit()
            || c.transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
            || !available(&*self.slot.lock().map_err(|_| Error::storage())?)
            || plan.transitions.is_empty()
            || plan.transitions.len() > 128
        {
            return Err(Error::forbidden());
        }
        let facts = plan_facts(store, c, &plan)?;
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ctor-envelope");
        let witness = witness_facts(store, c, proof, &plan, resolved_ready)?;
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ctor-opaque-witness");
        strict_validation::validate(c, &plan, &facts)?;
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ctor-strict-validation");
        let mut required = crate::authority_admission::conversation_scopes(
            c,
            &facts.conversation,
            AuthorityMode::Read,
        )?;
        required.extend(witness.required);
        required.extend(plan.required.iter().cloned());
        required.push(AuthorityScope::record(
            &plan.key.record,
            AuthorityMode::Write,
        )?);
        required.push(AuthorityScope::connection(
            &plan.source,
            AuthorityMode::Write,
        )?);
        if let Some(candidate) = &plan.candidate {
            // witness_facts above independently derives the exact current
            // continuation for FinalReady. Its closed typed SQL plan only
            // projects that same installed binding; every other phase keeps
            // the existing candidate Writer. Plan omission cannot select this.
            let candidate_mode = if matches!(
                (&plan.witness, plan.purpose),
                (Witness::Ready(_), Purpose::FinalReady)
            ) {
                AuthorityMode::Read
            } else {
                AuthorityMode::Write
            };
            required.push(AuthorityScope::connection(candidate, candidate_mode)?);
        }
        if !store.authority_covers(proof, &required) {
            return Err(Error::forbidden());
        }
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ctor-complete-coverage");
        let mut rows: BTreeMap<(Table, String), Row> = BTreeMap::new();
        for transition in &plan.transitions {
            validate_transition(&plan, &facts, transition)?;
            let key = transition.new.key()?;
            let encoded_key = c
                .query_row(
                    &format!(
                        "SELECT json_array({})",
                        (1..=key.len())
                            .map(|i| typed(&format!("?{i}")))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    rusqlite::params_from_iter(key.iter()),
                    |r| r.get::<_, String>(0),
                )
                .map_err(|_| Error::storage())?;
            let identity = (transition.table, encoded_key);
            let previous = if let Some(row) = rows.get(&identity) {
                Some(row.clone())
            } else {
                row_snapshot(c, transition.table, &key)?
            };
            if previous.as_ref().map(|r| row_tuple(c, r)).transpose()?
                != transition
                    .old
                    .as_ref()
                    .map(|r| row_tuple(c, r))
                    .transpose()?
            {
                return Err(Error::forbidden());
            }
            rows.insert(identity, transition.new.clone());
        }
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ctor-typed-current-chain");
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !available(&slot)
            || c.is_autocommit()
            || plan.until.is_some_and(|until| crate::now() >= until)
        {
            return Err(Error::forbidden());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        // The permit must end before COMMIT. Pin the complete actual mixed
        // proof to the SQL Unit too, so dropping permit/caller Arcs cannot
        // release any affected Writer or retained Reader before transaction end.
        // Unit::finish releases these outside this mutex after commit/rollback.
        if !slot.proofs.iter().any(|held| Arc::ptr_eq(held, proof)) {
            if slot.proofs.len() >= 32 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "SQL mutation proof capacity exceeded",
                ));
            }
            slot.proofs.push(proof.clone());
        }
        slot.next_birth_nonce = nonce;
        slot.phase_ledger = Some(Active {
            key: plan.key,
            nonce,
            root_revision: facts.revision,
            reservation_root: None,
            candidate_audience: None,
            commit_capture: None,
            scoped_capture: Some(Capture {
                rows: plan.transitions,
                until: plan.until,
            }),
            data_control_capture: None,
            _proof: proof.clone(),
        });
        #[cfg(test)]
        crate::scoped_phase_timing::mark("scoped-ctor-slot-installed");
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
}

#[cfg(test)]
mod typed_tuple_tests;
