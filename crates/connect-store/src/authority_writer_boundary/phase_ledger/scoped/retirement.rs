//! Exact historical retirement deltas; never admission or a runtime absence claim.
use super::*;

fn row(c: &Connection, table: Table, id: &str) -> Result<Row> {
    row_snapshot(c, table, &[Value::Text(id.into())])?.ok_or_else(Error::forbidden)
}
/// Called only after the actual typed absence/current-owner observation. The
/// caller repeats it before commit and proves lifecycle unchanged AFTER this
/// tentative DML; a genuine wider change rolls back before broad admission.
/// An already-settled replacement can return an empty plan: its caller must use
/// a separately proved read-only no-op, not install a zero-delta SQL permit.
pub(crate) fn retire_plan<'a>(
    store: &Store,
    c: &Connection,
    witness: Witness<'a>,
) -> Result<Plan<'a>> {
    let (purpose, facts) = match &witness {
        Witness::OriginalRetire(lease) => (
            Purpose::OriginalRetire,
            lease.scoped_original_retire_facts(store, c)?,
        ),
        Witness::ReplacementRetire(origin) => (
            Purpose::ReplacementRetire,
            origin.scoped_replacement_retire_facts(store, c)?,
        ),
        _ => return Err(Error::forbidden()),
    };
    if facts.candidate.is_some()
        || facts.until.is_some()
        || facts.reservation.is_some()
        || facts.digest.is_some()
    {
        return Err(Error::forbidden());
    }
    let old_life = row(c, Table::NativeConnectionLifetimes, &facts.record)?;
    if text(&old_life, "connection_id")? != facts.source
        || text(&old_life, "runtime_epoch")? != facts.runtime
    {
        return Err(Error::forbidden());
    }
    let source = row(c, Table::Connections, &facts.source)?;
    let mut transitions = Vec::new();
    // Keep actual DML order: lifetime retirement precedes the public row.
    if integer(&old_life, "retired")? == 0 {
        let mut retired = old_life.clone();
        retired.set("retired", Value::Integer(1))?;
        transitions.push(Transition::new(c, Some(&old_life), Some(&retired))?);
    }
    if text(&source, "state")? != "retired" {
        let mut retired = source.clone();
        retired.set("state", Value::Text("retired".into()))?;
        retired.set(
            "generation",
            Value::Integer(
                integer(&source, "generation")?
                    .checked_add(1)
                    .ok_or_else(Error::forbidden)?,
            ),
        )?;
        retired.set(
            "revision",
            Value::Integer(
                integer(&source, "revision")?
                    .checked_add(1)
                    .ok_or_else(Error::forbidden)?,
            ),
        )?;
        for column in ["observed_runtime_id", "observed_at", "readiness_json"] {
            retired.set(column, Value::Null)?;
        }
        transitions.push(Transition::new(c, Some(&source), Some(&retired))?);
    }
    if purpose == Purpose::OriginalRetire {
        let original = row(c, Table::Operations, &facts.record)?;
        let mut settled = original.clone();
        for column in ["lease_owner", "lease_expires_at", "worker_runtime"] {
            settled.set(column, Value::Null)?;
        }
        settled.set(
            "worker_generation",
            Value::Integer(
                integer(&original, "worker_generation")?
                    .saturating_add(1)
                    .min(9_007_199_254_740_991),
            ),
        )?;
        transitions.push(Transition::new(c, Some(&original), Some(&settled))?);
    }
    Ok(Plan {
        witness,
        key: Key {
            record: facts.record,
            phase: purpose.phase(),
            attempt: facts.attempt.ok_or_else(Error::forbidden)?,
            effect: facts.effect.ok_or_else(Error::forbidden)?,
            expected: facts.expected.ok_or_else(Error::forbidden)?,
            runtime: facts.runtime,
            reservation: String::new(),
            digest: String::new(),
            stage: Stage::Confirm,
        },
        purpose,
        source: facts.source,
        candidate: None,
        required: facts.required,
        until: None,
        transitions,
    })
}
