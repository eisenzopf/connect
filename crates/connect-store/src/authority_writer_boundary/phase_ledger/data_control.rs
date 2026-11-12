//! Exact captured Data-only control metadata, never media or wire authority.
use super::*;
use rusqlite::types::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Table {
    Operations,
    Effects,
    Connections,
    StreamOperations,
    Snapshots,
}
impl Table {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Operations => "operations",
            Self::Effects => "operation_effects",
            Self::Connections => "connections",
            Self::StreamOperations => "native_stream_operations",
            Self::Snapshots => "native_stream_snapshots",
        }
    }
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "operations" => Self::Operations,
            "operation_effects" => Self::Effects,
            "connections" => Self::Connections,
            "native_stream_operations" => Self::StreamOperations,
            "native_stream_snapshots" => Self::Snapshots,
            _ => return None,
        })
    }
    pub(crate) fn columns(self) -> &'static [&'static str] {
        match self {
            Self::Operations => scoped::Table::Operations.columns(),
            Self::Effects => scoped::Table::Effects.columns(),
            Self::Connections => scoped::Table::Connections.columns(),
            Self::StreamOperations => &[
                "operation_record_id",
                "parent_record_id",
                "credential_id",
                "key_id",
                "device_id",
                "participant_id",
                "membership_epoch",
                "generation",
                "resources_json",
                "source_expires_at",
                "closed",
                "effect_started",
                "effect_id",
                "attempt_id",
                "attempt_runtime",
            ],
            Self::Snapshots => &[
                "connection_id",
                "generation",
                "stream_revision",
                "available",
                "projection_json",
            ],
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Operations => "record_id",
            Self::Effects | Self::Connections => "id",
            Self::StreamOperations => "operation_record_id",
            Self::Snapshots => "connection_id",
        }
    }
}
fn typed(v: &str) -> String {
    format!("json_array(typeof({v}),CASE typeof({v}) WHEN 'text' THEN {v}||'' WHEN 'blob' THEN hex({v}) ELSE {v} END)")
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
    pub(crate) fn load(c: &Connection, table: Table, id: &str) -> Result<Self> {
        Self::optional(c, table, id)?.ok_or_else(Error::forbidden)
    }
    pub(crate) fn optional(c: &Connection, table: Table, id: &str) -> Result<Option<Self>> {
        let columns = table
            .columns()
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(",");
        c.prepare_cached(&format!(
            "SELECT {columns} FROM \"{}\" WHERE \"{}\"=?1",
            table.name(),
            table.key()
        ))
        .map_err(|_| Error::storage())?
        .query_row([id], |r| {
            Ok(Self {
                table,
                values: (0..table.columns().len())
                    .map(|i| r.get(i))
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            })
        })
        .optional()
        .map_err(|_| Error::storage())
    }
    pub(crate) fn get(&self, column: &str) -> Result<&Value> {
        let i = self
            .table
            .columns()
            .iter()
            .position(|v| *v == column)
            .ok_or_else(Error::forbidden)?;
        Ok(&self.values[i])
    }
    pub(crate) fn text(&self, column: &str) -> Result<&str> {
        match self.get(column)? {
            Value::Text(v) => Ok(v),
            _ => Err(Error::forbidden()),
        }
    }
    pub(crate) fn integer(&self, column: &str) -> Result<i64> {
        match self.get(column)? {
            Value::Integer(v) => Ok(*v),
            _ => Err(Error::forbidden()),
        }
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
    fn tuple(&self, c: &Connection) -> Result<String> {
        let sql = format!(
            "SELECT json_array({})",
            (1..=self.values.len())
                .map(|i| typed(&format!("?{i}")))
                .collect::<Vec<_>>()
                .join(",")
        );
        c.prepare_cached(&sql)
            .map_err(|_| Error::storage())?
            .query_row(rusqlite::params_from_iter(self.values.iter()), |r| r.get(0))
            .map_err(|_| Error::storage())
    }
}
pub(crate) struct Change {
    table: Table,
    old: Option<Row>,
    new: Row,
    tuple: String,
    pub(crate) journal: bool,
}
impl Change {
    pub(crate) fn new(c: &Connection, old: &Row, new: &Row, journal: bool) -> Result<Self> {
        if old.table != new.table
            || old.get(old.table.key())? != new.get(new.table.key())?
            || old.values == new.values
        {
            return Err(Error::forbidden());
        }
        Self::from_rows(c, Some(old), new, journal)
    }
    pub(crate) fn insert(c: &Connection, new: &Row) -> Result<Self> {
        if new.table != Table::Snapshots {
            return Err(Error::forbidden());
        }
        Self::from_rows(c, None, new, false)
    }
    fn from_rows(c: &Connection, old: Option<&Row>, new: &Row, journal: bool) -> Result<Self> {
        let before = old.map(|row| row.tuple(c)).transpose()?;
        let tuple = c
            .prepare_cached("SELECT json_array(json(?1),json(?2))")
            .map_err(|_| Error::storage())?
            .query_row(rusqlite::params![before, new.tuple(c)?], |r| r.get(0))
            .map_err(|_| Error::storage())?;
        Ok(Self {
            table: new.table,
            old: old.cloned(),
            new: new.clone(),
            tuple,
            journal,
        })
    }
    #[cfg(test)]
    pub(crate) fn snapshot_insert(&self) -> bool {
        self.table == Table::Snapshots && self.old.is_none()
    }
    fn action(&self) -> &'static str {
        if self.old.is_some() {
            "update"
        } else {
            "insert"
        }
    }
    /// Execute exact changed columns; a new metadata-only descriptor uses plain
    /// INSERT, never REPLACE or a victim-authority shortcut.
    pub(crate) fn apply(&self, c: &Connection) -> Result<()> {
        let count = if let Some(old) = &self.old {
            let changed = self
                .table
                .columns()
                .iter()
                .enumerate()
                .filter(|(i, _)| old.values[*i] != self.new.values[*i])
                .collect::<Vec<_>>();
            let setters = changed
                .iter()
                .enumerate()
                .map(|(n, (_, col))| format!("\"{col}\"=?{}", n + 1))
                .collect::<Vec<_>>()
                .join(",");
            let mut params = changed
                .iter()
                .map(|(i, _)| self.new.values[*i].clone())
                .collect::<Vec<_>>();
            params.push(old.get(self.table.key())?.clone());
            c.prepare_cached(&format!(
                "UPDATE \"{}\" SET {setters} WHERE \"{}\"=?{}",
                self.table.name(),
                self.table.key(),
                params.len()
            ))
            .map_err(|_| Error::storage())?
            .execute(rusqlite::params_from_iter(params))
            .map_err(|_| Error::storage())?
        } else {
            let columns = self
                .table
                .columns()
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(",");
            let params = (1..=self.new.values.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            c.prepare_cached(&format!(
                "INSERT INTO \"{}\"({columns}) VALUES({params})",
                self.table.name()
            ))
            .map_err(|_| Error::storage())?
            .execute(rusqlite::params_from_iter(self.new.values.iter()))
            .map_err(|_| Error::storage())?
        };
        if count != 1 {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
pub(crate) struct Plan {
    pub(crate) key: Key,
    pub(crate) required: Vec<AuthorityScope>,
    pub(crate) until: i64,
    pub(crate) descriptor: &'static str,
    pub(crate) rows: Vec<Change>,
}
impl Plan {
    pub(crate) fn verify_final(&self, c: &Connection) -> Result<()> {
        let mut last = BTreeMap::new();
        for change in &self.rows {
            last.insert(
                (
                    change.table,
                    change.new.text(change.table.key())?.to_owned(),
                ),
                &change.new,
            );
        }
        for ((table, id), expected) in last {
            if Row::load(c, table, &id)?.tuple(c)? != expected.tuple(c)? {
                return Err(Error::forbidden());
            }
        }
        Ok(())
    }
}
pub(super) struct Capture {
    rows: Vec<(Table, &'static str, String)>,
    until: i64,
    next: usize,
}
pub(super) fn matches(
    active: &mut Active,
    action: &str,
    table: &str,
    effect: Option<&str>,
    phase: Option<&str>,
    expected: Option<&str>,
    attempt: Option<&str>,
    revision: Option<i64>,
) -> bool {
    let Some(capture) = active.data_control_capture.as_mut() else {
        return false;
    };
    if action == "data-control-active" {
        return phase == Some("data-control")
            && effect.is_none()
            && expected.is_none()
            && attempt.is_none()
            && revision.is_none();
    }
    if phase != Some("scoped-handoff")
        || !matches!(effect, Some("insert" | "update"))
        || attempt.is_some()
        || revision.is_some()
        || crate::now() >= capture.until
    {
        return false;
    }
    let Some(table) = Table::from_name(table) else {
        return false;
    };
    let Some((kind, expected_action, bytes)) = capture.rows.get(capture.next) else {
        return false;
    };
    if *kind != table || Some(*expected_action) != effect {
        return false;
    }
    match action {
        "scoped-row-active" => expected.is_none(),
        "scoped-row" if expected == Some(bytes.as_str()) => {
            capture.next += 1;
            true
        }
        _ => false,
    }
}
impl Permit<'_> {
    pub(crate) fn require_data_control_complete(&self) -> Result<()> {
        let slot = self.state.slot.lock().map_err(|_| Error::storage())?;
        let active = slot.phase_ledger.as_ref().ok_or_else(Error::forbidden)?;
        let capture = active
            .data_control_capture
            .as_ref()
            .ok_or_else(Error::forbidden)?;
        if !slot.active
            || slot.failed
            || active.nonce != self.nonce
            || capture.next != capture.rows.len()
        {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
impl State {
    /// The only plan producer is the exact opaque original captured receipt.
    /// No caller-selected IDs/rows, fact cache, IO grant or SQL inside callbacks.
    pub(crate) fn native_data_control_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        captured: &crate::NativeStreamCommitted,
    ) -> Result<Option<(Permit<'a>, Plan)>> {
        let owner = store.require_runtime_owner()?;
        let tag = c as *const Connection as usize;
        let available = |slot: &Slot| {
            slot.active
                && !slot.failed
                && slot.connection == tag
                && slot.birth.is_none()
                && slot.phase_ledger.is_none()
        };
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || c.transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
            || !available(&*self.slot.lock().map_err(|_| Error::storage())?)
        {
            return Err(Error::forbidden());
        }
        let Some(plan) = captured.data_control_plan(store, c)? else {
            return Ok(None);
        };
        if !store.authority_covers(proof, &plan.required)
            || crate::now() >= plan.until
            || plan.rows.is_empty()
        {
            return Err(Error::forbidden());
        }
        // Revalidate every complete initial row and adjacent sequential chain.
        let mut current = BTreeMap::new();
        for row in &plan.rows {
            let id = row.new.text(row.table.key())?.to_owned();
            let identity = (row.table, id.clone());
            if !current.contains_key(&identity) {
                current.insert(identity.clone(), Row::optional(c, row.table, &id)?);
            }
            let actual = current.get(&identity).ok_or_else(Error::forbidden)?;
            if actual.as_ref().map(|r| r.tuple(c)).transpose()?
                != row.old.as_ref().map(|r| r.tuple(c)).transpose()?
            {
                return Err(Error::forbidden());
            }
            current.insert(identity, Some(row.new.clone()));
        }
        let key = Key {
            record: plan.key.record.clone(),
            phase: plan.key.phase,
            attempt: plan.key.attempt.clone(),
            effect: plan.key.effect.clone(),
            expected: plan.key.expected.clone(),
            runtime: plan.key.runtime.clone(),
            reservation: plan.key.reservation.clone(),
            digest: plan.key.digest.clone(),
            stage: plan.key.stage,
        };
        let capture = Capture {
            rows: plan
                .rows
                .iter()
                .map(|r| (r.table, r.action(), r.tuple.clone()))
                .collect(),
            until: plan.until,
            next: 0,
        };
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !available(&slot) || c.is_autocommit() || crate::now() >= plan.until || !owner.current()
        {
            return Err(Error::forbidden());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
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
            key,
            nonce,
            root_revision: 0,
            reservation_root: None,
            candidate_audience: None,
            commit_capture: None,
            scoped_capture: None,
            data_control_capture: Some(capture),
            _proof: proof.clone(),
        });
        Ok(Some((
            Permit {
                state: self.clone(),
                nonce,
                _connection: std::marker::PhantomData,
            },
            plan,
        )))
    }
}
